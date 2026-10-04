//! Reading the table off the volume, as a stream.
//!
//! The table of a busy system volume is gigabytes, and a disk analyzer that
//! needs gigabytes of memory to say where the disk went is one people close.
//! Here the table goes past in chunks, each record is boiled down to a
//! [`Record`] the moment it is read, and the chunk is reused. See ADR 0006.
//! The format itself — the boot sector, the record header, the attributes —
//! is read by [`super::format`].

use std::io::{Read as _, Seek as _, SeekFrom};
use std::os::windows::ffi::OsStrExt as _;
use std::path::{Path, PathBuf};

use windows_sys::Win32::Foundation::{CloseHandle, GENERIC_READ, GENERIC_WRITE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, FlushFileBuffers, GetVolumeInformationW, GetVolumeNameForVolumeMountPointW, GetVolumePathNameW,
    OPEN_EXISTING,
};

use super::format::{self, Attribute, Geometry, Run};
use super::{Name, Record, Records, record_of};
use crate::error::{Error, Result};
use crate::scanner::Progress;

/// How much of the table is read at once.
///
/// Large enough that the read is sequential in every sense that matters to a
/// disk, small enough that the buffer is not the scan's memory. A multiple of
/// every record size NTFS uses (1 KiB and 4 KiB) and of every cluster size.
const CHUNK_BYTES: u64 = 16 << 20;

/// The named stream the Windows Overlay Filter keeps a compressed file in.
const WOF_STREAM: &str = "WofCompressedData";

/// How much of the volume's start is read for the boot sector: one sector on
/// a 4K-native disk, eight on any other — and a raw read must be whole
/// sectors of a size not yet known.
const BOOT_READ: usize = 4096;

/// Where a scan root sits, as the MFT reader needs to know it.
#[derive(Debug, Clone)]
pub(super) struct Located {
    /// The volume as a device: `\\?\Volume{…}`, no trailing separator.
    device: PathBuf,
    /// The record of the directory being scanned.
    pub(super) root_record: u64,
    /// The volume serial the walk's file identities carry.
    pub(super) serial: u64,
}

/// Finds the NTFS volume under `root` and the record `root` is.
///
/// `None` when `root` is not on NTFS or the questions cannot be asked — which
/// is how [`super::acceleration`] learns not to offer a prompt that would buy
/// nothing. Asked through the volume mount point rather than the drive letter,
/// so a volume mounted into a folder is read as itself, not as the drive the
/// folder happens to be on.
pub(super) fn locate(root: &Path) -> Option<Located> {
    let root = std::path::absolute(root).ok()?;
    let mount = volume_path_of(&root)?;
    if filesystem_of(&mount)?.as_str() != "NTFS" {
        return None;
    }
    let device = device_of(&mount)?;
    let identity = crate::platform::identity_of(&root)?;
    // The file id of an NTFS file is its 64-bit reference widened; the record
    // is the low 48 bits of that.
    #[allow(clippy::cast_possible_truncation, reason = "an NTFS file id fits in 64 bits; the high half is zero")]
    let reference = identity.file as u64;
    Some(Located {
        device,
        root_record: record_of(reference),
        serial: identity.volume,
    })
}

/// Streams the volume's table into records.
///
/// # Errors
///
/// [`Error::Unavailable`] when the volume cannot be opened or its table cannot
/// be streamed — the caller then walks instead — and [`Error::Cancelled`] when
/// the reader asked to stop.
pub(super) fn read(located: &Located, progress: &Progress) -> Result<Records> {
    // What NTFS has changed in memory but not written yet is invisible to a
    // raw read of the device: a folder created a second ago may not be in the
    // table on disk. Flushing the volume first is what makes the tree describe
    // the volume as it is rather than as it was. It needs the elevation the
    // read needs anyway; if it is refused the read goes ahead and is at worst a
    // second behind.
    flush(&located.device);

    // The table is read straight from the device. A raw volume read needs its
    // offset and length on sector boundaries; every read here starts on a
    // cluster and is a whole number of clusters, which is always a whole
    // number of sectors.
    let mut device = std::fs::File::open(&located.device).map_err(|error| unavailable("open the volume", &error))?;
    let geometry = read_geometry(&mut device)?;
    let first = read_first_record(&mut device, &geometry)?;
    let (table_bytes, runs) = runs_of_the_table(&first, &geometry)?;

    let cluster = geometry.cluster_bytes;
    let record_bytes = usize::try_from(geometry.record_bytes).map_err(|_| Error::Unavailable("the record size does not fit in memory".into()))?;
    let total_records = table_bytes / geometry.record_bytes;
    let mut records = Records::new();
    let mut number = 0_u64;
    let mut pending: Vec<u8> = Vec::new();
    let mut chunk = Vec::new();

    for run in runs {
        let Run::Data { at: start, bytes: length } = run else {
            return Err(Error::Unavailable("the table has a hole in it, which NTFS never writes".into()));
        };
        let mut offset = 0_u64;
        while offset < length && number < total_records {
            if progress.cancelled() {
                return Err(Error::Cancelled);
            }
            // What the table still holds, rounded up to a whole cluster so the
            // read stays aligned; a record past the table's end is never taken.
            let left_in_table = (total_records - number) * geometry.record_bytes - pending.len() as u64;
            let wanted = CHUNK_BYTES.min(length - offset).min(left_in_table.div_ceil(cluster) * cluster);
            if wanted == 0 {
                break;
            }
            chunk.resize(usize::try_from(wanted).unwrap_or(usize::MAX), 0);
            device
                .seek(SeekFrom::Start(start + offset))
                .and_then(|_| device.read_exact(&mut chunk))
                .map_err(|error| unavailable("read the table", &error))?;
            offset += wanted;

            // A record can straddle two runs when a cluster is smaller than a
            // record; what is left over waits for the next read.
            pending.extend_from_slice(&chunk);
            let whole = pending.len() / record_bytes * record_bytes;
            let mut read_here = 0_u64;
            for bytes in pending[..whole].chunks_exact_mut(record_bytes) {
                if number >= total_records {
                    break;
                }
                take(number, bytes, &mut records);
                number += 1;
                read_here += 1;
            }
            pending.drain(..whole);
            progress.read_records(read_here);
        }
    }

    Ok(records)
}

/// Reads the boot sector: where the table is, and how it is cut.
fn read_geometry(device: &mut std::fs::File) -> Result<Geometry> {
    let mut boot = vec![0_u8; BOOT_READ];
    device
        .seek(SeekFrom::Start(0))
        .and_then(|_| device.read_exact(&mut boot))
        .map_err(|error| unavailable("read the boot sector", &error))?;
    format::geometry(&boot).ok_or_else(|| Error::Unavailable("the boot sector does not describe an NTFS volume".into()))
}

/// Reads the table's own record, record 0, which says where the rest of it is.
fn read_first_record(device: &mut std::fs::File, geometry: &Geometry) -> Result<Vec<u8>> {
    let length = usize::try_from(geometry.record_bytes.next_multiple_of(geometry.sector_bytes))
        .map_err(|_| Error::Unavailable("the record size does not fit in memory".into()))?;
    let mut bytes = vec![0_u8; length];
    device
        .seek(SeekFrom::Start(geometry.table_at))
        .and_then(|_| device.read_exact(&mut bytes))
        .map_err(|error| unavailable("read the table's first record", &error))?;
    bytes.truncate(usize::try_from(geometry.record_bytes).unwrap_or(usize::MAX));
    if !format::fix_up(&mut bytes) {
        return Err(Error::Unavailable("the table's first record is torn".into()));
    }
    Ok(bytes)
}

/// The data runs of `$MFT` itself: where on the volume the table lives.
fn runs_of_the_table(first: &[u8], geometry: &Geometry) -> Result<(u64, Vec<Run>)> {
    let header = format::header(first).ok_or_else(|| Error::Unavailable("the table's first record is not a file record".into()))?;
    let found = format::attributes(first, &header).find(|attribute| attribute.kind() == format::DATA && attribute.name().is_none());
    match found.map(|attribute| attribute.runs(geometry.cluster_bytes)) {
        Some(Ok(runs)) => Ok(runs),
        // A table so fragmented that its own map spills into a second record.
        // Rare enough that following the spill is not worth a second code path;
        // the walk reads the volume instead and says why.
        Some(Err(problem)) => Err(Error::Unavailable(format!("could not map the table: {problem}"))),
        None => Err(Error::Unavailable("the table has no data stream".into())),
    }
}

/// Boils one raw record down into `records`.
fn take(number: u64, bytes: &mut [u8], records: &mut Records) {
    if !format::fix_up(bytes) {
        return;
    }
    let Some(header) = format::header(bytes) else {
        return;
    };
    if !header.in_use() {
        return;
    }

    // An extension record carries attributes of a file whose base record is
    // elsewhere; everything it says belongs there.
    let base = header.base.map_or(number, record_of);
    let record = records.entry(base);
    if base == number {
        record.in_use = true;
        record.sequence = header.sequence;
        record.directory = header.is_directory();
    }

    for attribute in format::attributes(bytes, &header) {
        read_attribute(&attribute, record);
    }
}

/// What one attribute adds to what is known about its file.
fn read_attribute(attribute: &Attribute<'_>, record: &mut Record) {
    match attribute.kind() {
        format::STANDARD_INFORMATION => {
            if let Some(standard) = attribute.standard_information() {
                record.modified = Some(standard.modified);
                record.attributes = standard.attributes;
            }
        }
        format::FILE_NAME => {
            if let Some(name) = attribute.file_name() {
                // A reparse tag is also copied into each name; it stands in
                // only until the `$REPARSE_POINT` attribute itself is read.
                if record.reparse_tag.is_none() {
                    record.reparse_tag = name.reparse_tag();
                }
                if !name.is_dos_alias() {
                    record.names.push(Name {
                        parent: name.parent,
                        name: name.name,
                    });
                }
            }
        }
        format::DATA => match attribute.name() {
            None => {
                if let Some(size) = attribute.stream_size() {
                    record.size = size;
                }
            }
            Some(name) if name == WOF_STREAM => {
                if let Some(size) = attribute.stream_size() {
                    record.backing = Some(size);
                }
            }
            Some(_) => {}
        },
        format::REPARSE_POINT => {
            if let Some(tag) = attribute.reparse_tag() {
                record.reparse_tag = Some(tag);
            }
        }
        _ => {}
    }
}

/// An error from the volume as the fallback will report it, with its causes.
fn unavailable(doing: &str, error: &(dyn std::error::Error + 'static)) -> Error {
    let mut message = format!("could not {doing}: {error}");
    let mut cause = error.source();
    while let Some(inner) = cause {
        message.push_str(": ");
        message.push_str(&inner.to_string());
        cause = inner.source();
    }
    Error::Unavailable(message)
}

/// A path as Windows wants it: UTF-16, NUL-terminated.
fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain(std::iter::once(0)).collect()
}

/// A NUL-terminated UTF-16 buffer as a string.
fn from_wide(buffer: &[u16]) -> String {
    let end = buffer.iter().position(|&unit| unit == 0).unwrap_or(buffer.len());
    String::from_utf16_lossy(&buffer[..end])
}

/// The mount point of the volume `path` is on: `C:\`, or `C:\mnt\data\` for a
/// volume mounted into a folder.
fn volume_path_of(path: &Path) -> Option<PathBuf> {
    let wide = wide(path);
    let mut buffer = vec![0_u16; 1024];
    // SAFETY: `wide` is NUL-terminated and outlives the call; `buffer` is
    // writable for the length passed.
    #[allow(unsafe_code, reason = "the volume a path is on is not exposed by std")]
    let ok = unsafe { GetVolumePathNameW(wide.as_ptr(), buffer.as_mut_ptr(), u32::try_from(buffer.len()).ok()?) };
    (ok != 0).then(|| PathBuf::from(from_wide(&buffer)))
}

/// The filesystem a mount point holds: `NTFS`, `ReFS`, `exFAT`, …
fn filesystem_of(mount: &Path) -> Option<String> {
    let wide = wide(mount);
    let mut name = vec![0_u16; 64];
    // SAFETY: `wide` is NUL-terminated and outlives the call; the null pointers
    // are the documented "not asked for" arguments, and `name` is writable for
    // the length passed.
    #[allow(unsafe_code, reason = "a volume's filesystem is not exposed by std")]
    let ok = unsafe {
        GetVolumeInformationW(
            wide.as_ptr(),
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            name.as_mut_ptr(),
            u32::try_from(name.len()).ok()?,
        )
    };
    (ok != 0).then(|| from_wide(&name))
}

/// The device path of the volume at a mount point: `\\?\Volume{…}`.
fn device_of(mount: &Path) -> Option<PathBuf> {
    let wide = wide(mount);
    let mut buffer = vec![0_u16; 64];
    // SAFETY: `wide` is NUL-terminated and ends in a separator, as the call
    // requires of a mount point; `buffer` is writable for the length passed.
    #[allow(unsafe_code, reason = "a volume's device name is not exposed by std")]
    let ok = unsafe { GetVolumeNameForVolumeMountPointW(wide.as_ptr(), buffer.as_mut_ptr(), u32::try_from(buffer.len()).ok()?) };
    if ok == 0 {
        return None;
    }
    // The name comes back as a directory, `\\?\Volume{…}\`; opened like that it
    // is the root folder, and without the separator it is the device.
    let name = from_wide(&buffer);
    Some(PathBuf::from(name.trim_end_matches('\\')))
}

/// Asks NTFS to write what it holds in memory for this volume.
fn flush(device: &Path) {
    let wide = wide(device);
    // SAFETY: `wide` is NUL-terminated and outlives the call; the null pointers
    // are the documented "no security attributes, no template" arguments.
    #[allow(unsafe_code, reason = "flushing a volume needs a handle std will not open")]
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            std::ptr::null(),
            OPEN_EXISTING,
            0,
            std::ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return;
    }
    // SAFETY: `handle` came from a successful `CreateFileW` and is closed once,
    // here, after the flush.
    #[allow(unsafe_code, reason = "flushing a volume needs a handle std will not open")]
    unsafe {
        FlushFileBuffers(handle);
        CloseHandle(handle);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_volume_that_is_not_there_is_not_located() {
        assert!(locate(Path::new("Q:\\no\\such\\place\\at\\all")).is_none());
    }
}

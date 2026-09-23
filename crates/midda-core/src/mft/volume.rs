//! Reading the table off the volume, as a stream.
//!
//! `ntfs-reader` knows the format — the boot sector, the data runs of `$MFT`,
//! the attributes inside a record — and this module uses it for exactly that.
//! What it does not use is the crate's `Mft`, which reads the whole table into
//! one buffer before handing out a record: the table of a busy system volume is
//! gigabytes, and a disk analyzer that needs gigabytes of memory to say where
//! the disk went is one people close. Here the table goes past in chunks, each
//! record is boiled down to a [`Record`] the moment it is read, and the chunk
//! is reused. See ADR 0006.

use std::io::{Read as _, Seek as _, SeekFrom};
use std::os::windows::ffi::OsStrExt as _;
use std::path::{Path, PathBuf};

use ntfs_reader::api::{NtfsAttributeType, NtfsFileRecordHeader};
use ntfs_reader::attribute::{DataRun, NtfsAttribute};
use ntfs_reader::file::NtfsFile;
use ntfs_reader::mft::Mft;
use ntfs_reader::volume::Volume;
use windows_sys::Win32::Foundation::{CloseHandle, GENERIC_READ, GENERIC_WRITE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, FlushFileBuffers, GetVolumeInformationW, GetVolumeNameForVolumeMountPointW, GetVolumePathNameW,
    OPEN_EXISTING,
};

use super::assemble::resident_size;
use super::{Name, Record, Records, record_of};
use crate::error::{Error, Result};
use crate::scanner::Progress;
use crate::size::Size;

/// How much of the table is read at once.
///
/// Large enough that the read is sequential in every sense that matters to a
/// disk, small enough that the buffer is not the scan's memory. A multiple of
/// every record size NTFS uses (1 KiB and 4 KiB) and of every cluster size.
const CHUNK_BYTES: u64 = 16 << 20;

/// The stride of the update sequence array: NTFS protects each 512-byte stretch
/// of a record, whatever the disk's own sector size.
const PROTECTED_STRETCH: usize = 512;

/// Record header flag: the record describes a live file.
const IN_USE: u16 = 0x0001;
/// Record header flag: the file is a directory.
const IS_DIRECTORY: u16 = 0x0002;
/// Attribute header flag: the stream is compressed.
const ATTRIBUTE_COMPRESSED: u16 = 0x0001;
/// Attribute header flag: the stream is sparse.
const ATTRIBUTE_SPARSE: u16 = 0x8000;
/// `$REPARSE_POINT`, which `ntfs-reader` does not name.
const REPARSE_POINT: u32 = 0xC0;
/// The named stream the Windows Overlay Filter keeps a compressed file in.
const WOF_STREAM: &str = "WofCompressedData";
/// Where a compressed or sparse stream's header keeps the bytes it really
/// occupies, which is what `AllocationSize` reports for such a file.
const TOTAL_ALLOCATED_AT: usize = 0x40;

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
    let volume = Volume::new(&located.device).map_err(|error| unavailable("open the volume", &error))?;

    // What NTFS has changed in memory but not written yet is invisible to a
    // raw read of the device: a folder created a second ago may not be in the
    // table on disk. Flushing the volume first is what makes the tree describe
    // the volume as it is rather than as it was. It needs the elevation the
    // read needs anyway; if it is refused the read goes ahead and is at worst a
    // second behind.
    flush(&located.device);

    let mut reader = ntfs_reader::aligned_reader::open_volume(&located.device).map_err(|error| unavailable("read the volume", &error))?;
    let first =
        Mft::get_record_fs(&mut reader, volume.file_record_size, volume.mft_position).map_err(|error| unavailable("read the table's first record", &error))?;
    let (table_bytes, runs) = runs_of_the_table(&first, &volume)?;

    drop(reader);

    // The table itself is read straight from the device, not through
    // `ntfs-reader`'s aligned reader: that one serves at most one 4 KiB block per
    // call, a seek and a read for every four records, and on a table of
    // gigabytes the syscalls are the scan. A raw volume read needs its offset
    // and length on sector boundaries; every read here starts on a cluster and
    // is a whole number of clusters, which is always a whole number of sectors.
    let mut device = std::fs::File::open(&located.device).map_err(|error| unavailable("read the volume", &error))?;
    let cluster = volume.cluster_size;

    let record_bytes = usize::try_from(volume.file_record_size).map_err(|_| Error::Unavailable("the record size does not fit in memory".into()))?;
    let total_records = table_bytes / volume.file_record_size;
    let mut records = Records::new();
    let mut number = 0_u64;
    let mut pending: Vec<u8> = Vec::new();
    let mut chunk = Vec::new();

    for run in runs {
        let DataRun::Data { lcn: start, length } = run else {
            return Err(Error::Unavailable("the table has a hole in it, which NTFS never writes".into()));
        };
        let mut offset = 0_u64;
        while offset < length && number < total_records {
            if progress.cancelled() {
                return Err(Error::Cancelled);
            }
            // What the table still holds, rounded up to a whole cluster so the
            // read stays aligned; a record past the table's end is never taken.
            let left_in_table = (total_records - number) * volume.file_record_size - pending.len() as u64;
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

/// The data runs of `$MFT` itself: where on the volume the table lives.
fn runs_of_the_table(first: &[u8], volume: &Volume) -> Result<(u64, Vec<DataRun>)> {
    let table = NtfsFile::new(0, first);
    let mut found = None;
    table.attributes(|attribute| {
        if found.is_none() && attribute.header.type_id == NtfsAttributeType::Data as u32 && attribute.header.name_length == 0 {
            found = Some(attribute.get_nonresident_data_runs(volume));
        }
    });
    match found {
        Some(Ok(runs)) => Ok(runs),
        // A table so fragmented that its own map spills into a second record.
        // Rare enough that following the spill is not worth a second code path;
        // the walk reads the volume instead and says why.
        Some(Err(error)) => Err(unavailable("map the table", &error)),
        None => Err(Error::Unavailable("the table has no data stream".into())),
    }
}

/// Boils one raw record down into `records`.
fn take(number: u64, bytes: &mut [u8], records: &mut Records) {
    if !fix_up(bytes) || !NtfsFile::is_valid(bytes) {
        return;
    }
    let file = NtfsFile::new(number, bytes);
    let header: &NtfsFileRecordHeader = file.header;
    let flags = header.flags;
    if flags & IN_USE == 0 {
        return;
    }

    // An extension record carries attributes of a file whose base record is
    // elsewhere; everything it says belongs there.
    let base = file.base_record_number().unwrap_or(number);
    let record = records.entry(base);
    if base == number {
        record.in_use = true;
        record.sequence = header.sequence_value;
        record.directory = flags & IS_DIRECTORY != 0;
    }

    file.attributes(|attribute| read_attribute(attribute, record));
}

/// What one attribute adds to what is known about its file.
fn read_attribute(attribute: &NtfsAttribute<'_>, record: &mut Record) {
    let type_id = attribute.header.type_id;
    if type_id == NtfsAttributeType::StandardInformation as u32 {
        if let Some(standard) = attribute.as_standard_info() {
            record.modified = Some(standard.modification_time);
            record.attributes = standard.file_attributes;
        }
    } else if type_id == NtfsAttributeType::FileName as u32 {
        if let Some(name) = attribute.as_name() {
            // A reparse tag is also copied into each name; it stands in only
            // until the `$REPARSE_POINT` attribute itself is read.
            if name.is_reparse_point() && record.reparse_tag.is_none() {
                record.reparse_tag = Some(name.header.reparse_point_tag);
            }
            if !name.is_dos_alias() {
                record.names.push(Name {
                    parent: name.header.parent_directory_reference,
                    name: name.to_string(),
                });
            }
        }
    } else if type_id == NtfsAttributeType::Data as u32 && attribute.header.name_length == 0 {
        if let Some(size) = size_of_stream(attribute) {
            record.size = size;
        }
    } else if type_id == NtfsAttributeType::Data as u32
        && name_of(attribute).is_some_and(|name| name == WOF_STREAM)
        && let Some(size) = size_of_stream(attribute)
    {
        record.backing = Some(size);
    } else if type_id == REPARSE_POINT
        && let Some(tag) = attribute
            .get_resident()
            .and_then(|value| value.get(..4))
            .and_then(|bytes| bytes.try_into().ok())
    {
        record.reparse_tag = Some(u32::from_le_bytes(tag));
    }
}

/// The name of a named attribute, when it has one and it can be read.
fn name_of(attribute: &NtfsAttribute<'_>) -> Option<String> {
    let length = usize::from(attribute.header.name_length);
    if length == 0 {
        return None;
    }
    let offset = usize::from(attribute.header.name_offset);
    let bytes = attribute.data().get(offset..offset + length * 2)?;
    let units: Vec<u16> = bytes.as_chunks::<2>().0.iter().map(|pair| u16::from_le_bytes(*pair)).collect();
    String::from_utf16(&units).ok()
}

/// The unnamed stream's size, both ways — what `FileStandardInfo` would have
/// said for the file.
///
/// `None` for a later piece of a stream split across records: only the piece
/// that starts at the beginning carries the sizes.
fn size_of_stream(attribute: &NtfsAttribute<'_>) -> Option<Size> {
    if let Some(resident) = attribute.resident_header() {
        return Some(resident_size(u64::from(resident.value_length)));
    }
    let stream = attribute.nonresident_header()?;
    if stream.lowest_vcn != 0 {
        return None;
    }
    let flags = attribute.header.flags;
    let allocated = if flags & (ATTRIBUTE_COMPRESSED | ATTRIBUTE_SPARSE) == 0 {
        stream.allocated_size
    } else {
        // For a compressed or sparse stream, `allocated_size` is the span the
        // stream covers; the clusters it really holds are in the field after,
        // and that is the number `AllocationSize` reports. Measured: 45 056 for
        // a 360 000-byte compressed file (see `crate::platform`).
        attribute
            .data()
            .get(TOTAL_ALLOCATED_AT..TOTAL_ALLOCATED_AT + 8)
            .and_then(|bytes| bytes.try_into().ok())
            .map_or(stream.allocated_size, u64::from_le_bytes)
    };
    Some(Size {
        logical: stream.data_size,
        allocated,
    })
}

/// Undoes NTFS's torn-write protection on one record, in place.
///
/// The last two bytes of every 512-byte stretch of a record are replaced on
/// disk by a check value, and the real bytes kept in an array near the start.
/// A stretch whose check value does not match was not written whole; the
/// record is then not to be trusted, and is skipped rather than half-read.
fn fix_up(record: &mut [u8]) -> bool {
    let Some(&[offset_low, offset_high, count_low, count_high]) = record.get(4..8) else {
        return false;
    };
    let offset = usize::from(u16::from_le_bytes([offset_low, offset_high]));
    let count = usize::from(u16::from_le_bytes([count_low, count_high]));
    if count == 0 || offset + count * 2 > record.len() {
        return false;
    }
    let check = [record[offset], record[offset + 1]];
    for stretch in 1..count {
        let end = stretch * PROTECTED_STRETCH;
        if end > record.len() {
            return false;
        }
        if record[end - 2..end] != check {
            return false;
        }
        let saved = offset + stretch * 2;
        record[end - 2] = record[saved];
        record[end - 1] = record[saved + 1];
    }
    true
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

    /// A 1 KiB record with an update sequence array at 0x30 of three entries:
    /// the check value and the saved last two bytes of each 512-byte stretch.
    fn protected(check: [u8; 2], saved: [[u8; 2]; 2]) -> Vec<u8> {
        let mut record = vec![0_u8; 1024];
        record[4..6].copy_from_slice(&0x30_u16.to_le_bytes());
        record[6..8].copy_from_slice(&3_u16.to_le_bytes());
        record[0x30..0x32].copy_from_slice(&check);
        record[0x32..0x34].copy_from_slice(&saved[0]);
        record[0x34..0x36].copy_from_slice(&saved[1]);
        record[510..512].copy_from_slice(&check);
        record[1022..1024].copy_from_slice(&check);
        record
    }

    #[test]
    fn a_protected_record_gets_its_real_bytes_back() {
        let mut record = protected([0x07, 0x00], [[0xAA, 0xBB], [0xCC, 0xDD]]);
        assert!(fix_up(&mut record));
        assert_eq!(&record[510..512], &[0xAA, 0xBB]);
        assert_eq!(&record[1022..1024], &[0xCC, 0xDD]);
    }

    #[test]
    fn a_torn_record_is_refused_rather_than_half_read() {
        let mut record = protected([0x07, 0x00], [[0xAA, 0xBB], [0xCC, 0xDD]]);
        // The second stretch was not written: its check value is stale.
        record[1022..1024].copy_from_slice(&[0x06, 0x00]);
        assert!(!fix_up(&mut record));
    }

    #[test]
    fn a_record_with_no_protection_array_is_refused() {
        let mut record = vec![0_u8; 1024];
        assert!(!fix_up(&mut record));
        let mut short = vec![0_u8; 6];
        assert!(!fix_up(&mut short));
    }

    #[test]
    fn a_volume_that_is_not_there_is_not_located() {
        assert!(locate(Path::new("Q:\\no\\such\\place\\at\\all")).is_none());
    }
}

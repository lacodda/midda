//! The two sizes a file has, and where the second one comes from.
//!
//! Every file has a *logical* size — the number of bytes a program reads out of
//! it — and an *allocated* size, the space it actually takes on the volume.
//! They are rarely equal and sometimes wildly apart:
//!
//! - a 1-byte file occupies a whole cluster, typically 4 KiB;
//! - an NTFS-compressed file occupies less than it reads;
//! - a sparse file (a VHDX, a database) may read as 100 GB and occupy 12;
//! - a cloud placeholder reads as its full size and occupies almost nothing.
//!
//! midda shows the allocated size by default, because someone who opened a disk
//! analyzer came to free space, and freeing a sparse 100 GB file returns 12.
//! The logical size is kept beside it from the first version rather than added
//! later: a tree that only stored one of them could not answer the other
//! question without a full rescan.

use serde::{Deserialize, Serialize};

/// What a file or a subtree takes, measured both ways.
///
/// Both members are plain byte counts. Folder totals are sums of these, so a
/// folder's `allocated` is the space its contents would return.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Size {
    /// The bytes a reader would get out of the file.
    pub logical: u64,
    /// The bytes the volume gives up if the file goes away.
    pub allocated: u64,
}

impl Size {
    /// A size that is the same measured either way — what a test fixture or a
    /// non-Windows filesystem produces when nothing special is going on.
    #[must_use]
    pub const fn flat(bytes: u64) -> Self {
        Self {
            logical: bytes,
            allocated: bytes,
        }
    }

    /// Nothing at all.
    #[must_use]
    pub const fn zero() -> Self {
        Self { logical: 0, allocated: 0 }
    }

    /// Adds `other` into this one.
    ///
    /// Saturating rather than wrapping: a sum that overflows `u64` is not a
    /// real disk, but a wrapped total would show a volume as nearly empty,
    /// which is the one lie this product must not tell.
    pub fn add(&mut self, other: Self) {
        self.logical = self.logical.saturating_add(other.logical);
        self.allocated = self.allocated.saturating_add(other.allocated);
    }
}

impl std::ops::Add for Size {
    type Output = Self;

    fn add(mut self, other: Self) -> Self {
        Self::add(&mut self, other);
        self
    }
}

impl std::iter::Sum for Size {
    fn sum<I: Iterator<Item = Self>>(iter: I) -> Self {
        iter.fold(Self::zero(), |mut total, one| {
            total.add(one);
            total
        })
    }
}

/// Which of the two sizes the caller wants to sort or display by.
///
/// The default is deliberate and is the product's position, not a preference:
/// `Allocated`.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SizeBasis {
    /// What the volume gives up. The default everywhere in midda.
    #[default]
    Allocated,
    /// What a reader would get out of the file.
    Logical,
}

impl SizeBasis {
    /// Picks this basis out of a measured size.
    #[must_use]
    pub const fn of(self, size: Size) -> u64 {
        match self {
            Self::Allocated => size.allocated,
            Self::Logical => size.logical,
        }
    }
}

/// Rounds `bytes` up to a whole number of `cluster` bytes.
///
/// This is what allocation on a block device does, and it is the fallback for
/// platforms that cannot report the real allocated size: better than claiming
/// the logical size, because the answer to "what do a million 100-byte files
/// cost" is then roughly right instead of off by forty times.
#[must_use]
pub fn round_up_to_cluster(bytes: u64, cluster: u64) -> u64 {
    if cluster == 0 {
        return bytes;
    }
    // `div_ceil` on the count of clusters, then back to bytes. Saturating, so a
    // size near `u64::MAX` cannot wrap to nothing.
    bytes.div_ceil(cluster).saturating_mul(cluster)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_flat_size_reads_the_same_either_way() {
        let size = Size::flat(4096);
        assert_eq!(SizeBasis::Allocated.of(size), 4096);
        assert_eq!(SizeBasis::Logical.of(size), 4096);
    }

    #[test]
    fn the_default_basis_is_what_the_volume_gives_up() {
        // Not a preference: someone who opened a disk analyzer came to free
        // space. A change here changes the product.
        assert_eq!(SizeBasis::default(), SizeBasis::Allocated);

        let sparse = Size {
            logical: 100_000_000_000,
            allocated: 12_000_000_000,
        };
        assert_eq!(SizeBasis::default().of(sparse), 12_000_000_000);
    }

    #[test]
    fn sizes_add_on_both_axes_at_once() {
        let total: Size = [Size { logical: 10, allocated: 4096 }, Size { logical: 20, allocated: 4096 }].into_iter().sum();
        assert_eq!(total, Size { logical: 30, allocated: 8192 });
    }

    #[test]
    fn a_total_that_would_overflow_saturates_rather_than_wrapping() {
        // A wrapped total would paint a full volume as nearly empty. Saturating
        // is wrong by a knowable amount; wrapping is wrong by everything.
        let mut total = Size {
            logical: u64::MAX,
            allocated: u64::MAX,
        };
        total.add(Size::flat(4096));
        assert_eq!(
            total,
            Size {
                logical: u64::MAX,
                allocated: u64::MAX
            }
        );
    }

    #[test]
    fn a_small_file_costs_a_whole_cluster() {
        assert_eq!(round_up_to_cluster(1, 4096), 4096);
        assert_eq!(round_up_to_cluster(4096, 4096), 4096);
        assert_eq!(round_up_to_cluster(4097, 4096), 8192);
        assert_eq!(round_up_to_cluster(0, 4096), 0);
    }

    #[test]
    fn an_unknown_cluster_size_leaves_the_number_alone() {
        assert_eq!(round_up_to_cluster(1234, 0), 1234);
    }
}

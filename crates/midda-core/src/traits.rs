//! Why a size is the number it is.
//!
//! Most files need no explanation: they read as what they occupy, give or take
//! the rounding to a cluster. A few do not, and for those the difference
//! between the two numbers is large enough that a reader who is not told why
//! will conclude the scanner is broken:
//!
//! - a **cloud placeholder** reads as its full size and occupies nothing,
//!   because the bytes are on someone else's disk;
//! - an **NTFS-compressed** file occupies less than it reads;
//! - a **sparse** file — a VHDX, a database — may read as 100 GB and occupy 12;
//! - a **hard link** is a second name for bytes already counted somewhere else,
//!   so one of its names occupies everything and the others occupy nothing.
//!
//! These facts are carried on the node rather than recomputed for display. They
//! are what the filesystem said at scan time, and a window that asked the disk
//! again would be asking about a disk that has moved on.
//!
//! The last one is not a property of the file at all — it is a property of the
//! *scan*, decided in [`crate::links`] after the walk, and it is the reason
//! this lives beside the size rather than inside [`crate::platform`].

use serde::{Deserialize, Serialize};

/// What is unusual about an entry, as far as its size is concerned.
///
/// A bitset rather than an enum: a file can be sparse *and* a placeholder *and*
/// have a second name, and the honest answer names all three. Eight bits sit
/// inside the padding `Node` already has, so this costs nothing per node on a
/// tree of several million.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Traits(u8);

impl Traits {
    /// The bytes live in the cloud; the file is a stub until something opens it.
    pub const PLACEHOLDER: Self = Self(1 << 0);
    /// NTFS is storing the contents compressed.
    pub const COMPRESSED: Self = Self(1 << 1);
    /// The file has holes: ranges that read as zeroes and occupy nothing.
    pub const SPARSE: Self = Self(1 << 2);
    /// The same bytes are reachable under another name on this volume.
    pub const LINKED: Self = Self(1 << 3);
    /// This is the name the shared bytes were counted under. Only ever set
    /// together with [`Self::LINKED`].
    pub const COUNTED_HERE: Self = Self(1 << 4);
    /// Somewhere beneath this directory is a name for bytes counted elsewhere.
    ///
    /// Set on directories by the roll-up, never on files. Without it a folder
    /// holding nothing but second names reports `0 B` with no account of
    /// itself, which is the most alarming row a disk analyzer can draw: a
    /// folder the reader can see is full, reported as empty. Seen on screen
    /// during the live run of 2026-09-20.
    pub const HOLDS_SHARED: Self = Self(1 << 5);

    /// Nothing unusual.
    #[must_use]
    pub const fn none() -> Self {
        Self(0)
    }

    /// Whether every trait in `other` is present here.
    ///
    /// `Traits::none()` is contained in everything, which is the mathematically
    /// right answer and never a question anyone asks.
    #[must_use]
    pub const fn has(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// Whether any trait at all is set.
    #[must_use]
    pub const fn any(self) -> bool {
        self.0 != 0
    }

    /// This set with `other` added.
    #[must_use]
    pub const fn with(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// This set with `other` removed.
    #[must_use]
    pub const fn without(self, other: Self) -> Self {
        Self(self.0 & !other.0)
    }

    /// Adds `other` in place.
    pub const fn insert(&mut self, other: Self) {
        self.0 |= other.0;
    }

    /// The raw bits, for a consumer that has to serialize them as a number.
    #[must_use]
    pub const fn bits(self) -> u8 {
        self.0
    }

    /// Whether this entry is a second name for bytes counted elsewhere.
    ///
    /// This is the one combination worth a name of its own: it is the answer to
    /// "why does this 50 MB file say it occupies nothing", and getting the
    /// polarity backwards would put the explanation on exactly the wrong row.
    #[must_use]
    pub const fn is_shared_name(self) -> bool {
        self.has(Self::LINKED) && !self.has(Self::COUNTED_HERE)
    }
}

impl std::ops::BitOr for Traits {
    type Output = Self;

    fn bitor(self, other: Self) -> Self {
        self.with(other)
    }
}

impl std::ops::BitOrAssign for Traits {
    fn bitor_assign(&mut self, other: Self) {
        self.insert(other);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_file_has_nothing_to_explain() {
        let plain = Traits::none();
        assert!(!plain.any());
        assert!(!plain.has(Traits::PLACEHOLDER));
        assert!(!plain.is_shared_name());
    }

    #[test]
    fn traits_stack_rather_than_replacing_one_another() {
        // A cloud placeholder is sparse as well, and a scanner that reported
        // only the last thing it noticed would explain the smaller surprise and
        // hide the larger one.
        let both = Traits::PLACEHOLDER | Traits::SPARSE;
        assert!(both.has(Traits::PLACEHOLDER));
        assert!(both.has(Traits::SPARSE));
        assert!(!both.has(Traits::COMPRESSED));
    }

    #[test]
    fn having_all_of_a_pair_is_not_the_same_as_having_either() {
        let one = Traits::PLACEHOLDER;
        assert!(!one.has(Traits::PLACEHOLDER | Traits::SPARSE), "one of a pair is not both of it");
        assert!(one.has(Traits::PLACEHOLDER));
    }

    #[test]
    fn the_name_holding_the_bytes_is_not_the_one_that_needs_explaining() {
        // The polarity that matters: the row that says "0 bytes" is the one
        // wanting a footnote, not the row that says 50 MB.
        let owner = Traits::LINKED | Traits::COUNTED_HERE;
        let other = Traits::LINKED;

        assert!(!owner.is_shared_name(), "the name the bytes were counted under is not a shared one");
        assert!(other.is_shared_name(), "a second name is where the explanation belongs");
    }

    #[test]
    fn an_unlinked_file_is_never_a_shared_name() {
        // COUNTED_HERE without LINKED is nonsense the core never produces, and
        // if it ever did it must not read as "shared".
        assert!(!Traits::COUNTED_HERE.is_shared_name());
        assert!(!Traits::none().is_shared_name());
    }

    #[test]
    fn removing_a_trait_leaves_the_others_alone() {
        let all = Traits::PLACEHOLDER | Traits::SPARSE | Traits::LINKED;
        let left = all.without(Traits::SPARSE);
        assert!(left.has(Traits::PLACEHOLDER));
        assert!(left.has(Traits::LINKED));
        assert!(!left.has(Traits::SPARSE));
    }

    #[test]
    fn traits_survive_a_round_trip_as_a_number() {
        // They cross into the window as a number; a change of representation
        // here would silently reinterpret every bit on the other side.
        let traits = Traits::PLACEHOLDER | Traits::LINKED;
        let json = serde_json::to_string(&traits).expect("serialize");
        assert_eq!(json, traits.bits().to_string(), "traits travel as their bits, not as a struct");
        let back: Traits = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, traits);
    }
}

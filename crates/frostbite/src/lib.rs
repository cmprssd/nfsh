//! Readers for the Frostbite data files in a Need for Speed: Heat install, written from
//! our own reading of the files (see `docs/FORMATS.md`). Every reader checks bounds and
//! returns an [`Error`] on malformed input; none of them panic.

pub mod cas;
pub mod dbobject;
pub mod ebx;
pub mod frames;
pub mod install;
pub mod level;
pub mod lighting;
pub mod mesh;
pub mod oodle;
pub mod reader;
pub mod sb;
pub mod store;
pub mod texture;
pub mod toc;

use std::fmt;

/// A file that does not have the shape the readers expect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(pub String);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

/// Builds an [`Error`] from a format string.
macro_rules! bail {
    ($($arg:tt)*) => { return Err($crate::Error(format!($($arg)*))) };
}
pub(crate) use bail;

/// Where an asset's stored bytes are: one of the `cas_NN.cas` archives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CasRef {
    /// In `Patch/` rather than `Data/`.
    pub patch: bool,
    /// Index into the layout's install packages (`superbundleinstallpackage`, ...).
    pub package: u8,
    /// The `NN` in `cas_NN.cas`.
    pub cas: u8,
}

impl CasRef {
    /// Decodes the packed word: byte 2 is the patch flag, byte 1 the package, byte 0
    /// the archive number. Byte 3 is always zero in Heat's files.
    pub fn from_word(word: u32) -> Result<Self> {
        let [high, patch, package, cas] = word.to_be_bytes();
        if high != 0 || patch > 1 {
            bail!("cas reference {word:#010x} has unknown flags");
        }
        Ok(Self {
            patch: patch == 1,
            package,
            cas,
        })
    }
}

/// A run of stored (usually compressed) bytes in one archive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Location {
    pub cas: CasRef,
    pub offset: u32,
    pub size: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cas_ref_unpacks_patch_package_and_number() {
        assert_eq!(
            CasRef::from_word(0x0001_010c),
            Ok(CasRef {
                patch: true,
                package: 1,
                cas: 12
            })
        );
        assert_eq!(
            CasRef::from_word(0x0000_0004),
            Ok(CasRef {
                patch: false,
                package: 0,
                cas: 4
            })
        );
        assert!(CasRef::from_word(0x0002_0004).is_err());
        assert!(CasRef::from_word(0x0100_0004).is_err());
    }
}

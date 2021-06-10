//! Common error handling types and utilities

// For macro item export
// #![allow(clippy::single_component_path_imports)]

use std::error::Error;
use std::ffi::OsStr;
use std::fmt::{self, Display, Formatter};
use std::io;
use std::path::PathBuf;

#[cfg(feature = "logging")]
pub(crate) use log::{error, warn};
#[cfg(not(feature = "logging"))]
mod log {
    macro_rules! error {
        ($($any:tt)*) => {
            eprintln!($($any)*);
        }
    }
    macro_rules! warning {
        ($($any:tt)*) => {
            eprintln!($($any)*);
        }
    }
    pub(crate) use {error, warning};
}
#[cfg(not(feature = "logging"))]
pub(crate) use log::{error, warning as warn};

/// Represents size of DCA file entries and also byte position inside them
/// Alias to the return type of [`std::io::Seek`] offset
pub type FilePosition = u64;

/// Central error type for (de)compressing DCA archives
#[derive(Debug)]
#[non_exhaustive]
pub enum ArchiveError {
    /// Represents failed IO operation directly on the archive file
    ArchiveIo(io::Error),
    /// When decompressing, structural violation was detected
    CorruptedArchive {
        /// Byte offset in the archive
        position: FilePosition,
        /// Logical name of the malformed section
        section: DecompressionError,
    },
    /// File within archive contents fails for I/O reasons - it can't be opened, read or written into
    BadFileIo(PathBuf, io::Error),
    /// Filename of archive contents doesn't conform to expected requirements
    InvalidDcaFilename(PathBuf, DcaFilenameError),
}

/// Standard conveniency alias
pub type Result<T, E = ArchiveError> = std::result::Result<T, E>;

impl Display for ArchiveError {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result<(), fmt::Error> {
        use ArchiveError::*;
        match self {
            ArchiveIo(_) => write!(f, "failed to read/write archive contents"),
            BadFileIo(path, _) => write!(f, "operation on archive entry {:?} failed", path),
            CorruptedArchive { position, section } => write!(
                f,
                "invalid state detected while parsing archive's section {:?} at position {}",
                section, position
            ),
            InvalidDcaFilename(path, problem) => write!(
                f,
                "filename entry {:?} doesn't match DCA naming requirements: {}",
                path.file_name().unwrap_or_else(|| OsStr::new("\"\"")),
                problem
            ),
        }
    }
}

impl Error for ArchiveError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        use ArchiveError::*;
        match self {
            ArchiveIo(io_err) | BadFileIo(_, io_err) => Some(io_err),
            _ => None,
        }
    }
}

pub trait Handler {
    /// Processing errors that may, but may not fail operation
    fn on_err(&self, err: ArchiveError) -> Result<(), ArchiveError>;
}

/// Convenience control flow macro for [`Handler`] - situations where the operation
/// may, but doesn't have to cause termination of the whole function
///
/// If expression succeeds, unwraps it
/// Otherwise, exits early if handler decides so
/// Otherwise, executes code from failblock, allowing control flow change or alternative ok result
///
/// Note: currently unused, but left in for curious reader
#[allow(unused_macros)]
macro_rules! handled {
    (try {$e:expr} else if $handle:ident($map_err:expr) $fail_blk:block) => {
        match $e.map_err($map_err) {
            Ok(ok) => ok,
            Err(err) => {
                $handle.on_err(err)?;
                $fail_blk;
            }
        }
    };
    (try {$e:expr} else if $handle:ident $fail_blk:block) => {
        handled!(try {$e} else if $handle(|e|e) $fail_blk)
    };
}

/// Errors concerning invalid filename for DCA entry
#[derive(Debug, PartialEq)]
#[non_exhaustive]
pub enum DcaFilenameError {
    /// Name not valid UTF-8
    NotUnicode,
    /// Unsupported character detected at certain position
    InvalidChar(char, usize),
}

impl Display for DcaFilenameError {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result<(), fmt::Error> {
        use DcaFilenameError::*;
        match self {
            NotUnicode => write!(f, "name is not valid UTF-8"),
            InvalidChar(ch, pos) => write!(f, "unsupported character '{}' at position {}", ch, pos),
        }
    }
}

impl Error for DcaFilenameError {}

/// Convert OS-specific filename into DCA-compatible name
pub fn into_dca_filename(name: &OsStr) -> Result<&str, DcaFilenameError> {
    use DcaFilenameError as O;
    let name = name.to_str().ok_or(O::NotUnicode)?;
    if let Some((pos, ch)) = name
        .chars()
        .enumerate()
        .find(|(_, ch)| ['\n', '/'].contains(ch))
    {
        Err(O::InvalidChar(ch, pos))
    } else {
        Ok(name)
    }
}

/// Lists various sections of DCA file format where problems during extraction could occur
#[derive(Debug)]
pub enum DecompressionError {
    Header,
    FileName,
    FileSize,
    Payload,
    Footer,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dca_filename() {
        assert_eq!(into_dca_filename(OsStr::new("hello")).unwrap(), "hello");
        assert_eq!(
            into_dca_filename(OsStr::new("foo/bar")).unwrap_err(),
            DcaFilenameError::InvalidChar('/', 3)
        );
        assert_eq!(
            into_dca_filename(OsStr::new("a\n")).unwrap_err(),
            DcaFilenameError::InvalidChar('\n', 1)
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_dca_filename_nonunicode() {
        use std::os::unix::ffi::OsStrExt;

        let invalid_unicode = b"ok\x80\x00\xFF";
        assert_eq!(
            into_dca_filename(OsStr::from_bytes(&invalid_unicode[..])).unwrap_err(),
            DcaFilenameError::NotUnicode
        );
    }

    #[cfg(windows)]
    #[test]
    fn test_dca_filename_nonunicode() {
        use std::ffi::OsString;
        use std::os::windows::prelude::*;

        let invalid_unicode = ['o' as u16, 'k' as u16, 0xD800];
        assert_eq!(
            into_dca_filename(&OsString::from_wide(&invalid_unicode)).unwrap_err(),
            DcaFilenameError::NotUnicode
        );
    }
}

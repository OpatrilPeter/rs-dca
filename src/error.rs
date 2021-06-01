//! Common error handling types and utilities

// For macro item export
#![allow(clippy::single_component_path_imports)]

use std::ffi::OsStr;
use std::io;
use std::path::PathBuf;

#[cfg(feature = "logging")]
pub(crate) use log::error;
#[cfg(not(feature = "logging"))]
macro_rules! error {
    ($($any:tt)*) => {
        eprintln!($($any)*);
    }
}
#[cfg(not(feature = "logging"))]
pub(crate) use error;

#[derive(Debug)]
#[non_exhaustive]
pub enum ArchiveError {
    /// Represents failed IO operation directly on the archive file
    ArchiveIo(io::Error),
    /// When decompressing, structure violation was detected
    CorruptedArchive {
        position: usize,
        section: DecompressionError,
    },
    /// File within archive contents fails for I/O reasons - it can't be opened, read or written into
    BadFileIo(PathBuf, io::Error),
    /// Filename of archive contents doesn't conform to expected requirements
    InvalidDcaFilename(PathBuf, DcaFilenameError),
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
pub(crate) use handled;

/// Errors concerning invalid filename for DCA entry
#[derive(Debug)]
#[non_exhaustive]
pub enum DcaFilenameError {
    /// Name not valid UTF-8
    NotUnicode,
    /// Unsupported character detected at certain position
    InvalidChar(char, usize),
}

/// Convert OS-specific filename into DCA-compatible name
pub fn dca_filename(name: &OsStr) -> Result<&str, DcaFilenameError> {
    use DcaFilenameError as O;
    let name = name.to_str().ok_or(O::NotUnicode)?;
    if let Some(pos) = name.find(&['\n', '/'][..]) {
        return Err(O::InvalidChar(name.chars().nth(pos).unwrap(), pos));
    }
    Ok(name)
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

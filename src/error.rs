use std::ffi::OsStr;
use std::io;
use std::path::PathBuf;

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

#[derive(Debug)]
#[non_exhaustive]
pub enum DcaFilenameError {
    NotUnicode,
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

#[derive(Debug)]
pub enum DecompressionError {
    Header,
    FileName,
    FileSize,
    Payload,
    Footer,
}

//! Handles extraction from DCA archives

use std::cmp::min;
use std::convert::TryInto;
use std::fs::{self, File};
use std::io::{self, prelude::*, BufRead, Seek};
use std::path::{Path, PathBuf};

use crate::error::{error, handled, ArchiveError, DecompressionError, Handler};

/// Error [`Handler`] that allows extraction failures, logging each encountered problem
struct DefaultHandler<'a> {
    archive_name: &'a Path,
}

impl<'a> DefaultHandler<'a> {
    fn new(archive_name: &'a Path) -> Self {
        Self { archive_name }
    }
    fn on_fatal(&self, err: &ArchiveError) {
        use ArchiveError::*;
        match err {
            ArchiveIo(io_err) => {
                error!(
                    "Failed read from archive {:?} due to following error: {}",
                    self.archive_name, io_err
                );
            }
            CorruptedArchive { position, section } => {
                error!(
                    "Unexpected end of archive at position {} while processing section {:?}",
                    position, section
                );
            }
            _ => {
                error!(
                    "Extraction of archive {:?} failed due to error {:?}",
                    self.archive_name, err
                );
            }
        }
    }
}

impl<'a> Handler for DefaultHandler<'a> {
    fn on_err(&self, err: ArchiveError) -> Result<(), ArchiveError> {
        use ArchiveError::*;
        match err {
            BadFileIo(fname, io_err) => {
                error!(
                    "Extraction of file {:?} failed due to following error {}, skipping.",
                    fname, io_err
                );
                Ok(())
            }
            // Other problems are fatal
            err => Err(err),
        }
    }
}

/// Returns true if read from the reader matches fixed byte sequence
fn read_matches<const N: usize>(
    reader: &mut impl Read,
    reference: &'static [u8; N],
    position: &mut usize,
) -> Result<bool, io::Error> {
    let mut buf = [0u8; N];
    reader.read_exact(&mut buf)?;

    let res = &buf == reference;
    if res {
        *position += N;
    }
    Ok(res)
}

/// Reads UTF-8 text from the archive up to the following newline (\n) character, and transforms
/// the read string into appropriate result type via `processor` callback.
///
/// Also allows to read nothing, returning None in that case.
fn read_line<T>(
    reader: &mut impl BufRead,
    line_buf: &mut String,
    position: &mut usize,
    processor: impl FnOnce(&str) -> Result<T, ArchiveError>,
) -> Result<Option<T>, ArchiveError> {
    line_buf.truncate(0);
    reader
        .read_line(line_buf)
        .map_err(ArchiveError::ArchiveIo)?;
    if line_buf.is_empty() {
        return Ok(None);
    }
    let res = processor(&line_buf[..line_buf.len() - 1]);
    if res.is_ok() {
        *position += line_buf.len();
    }
    res.map(Some)
}

/// Reads file size segment from the archive
fn read_file_size(
    reader: &mut impl BufRead,
    line_buf: &mut String,
    position: &mut usize,
) -> Result<usize, ArchiveError> {
    let old_pos = *position;
    let handler = |s: &str| {
        s.parse::<usize>()
            .map_err(|_| ArchiveError::CorruptedArchive {
                position: old_pos,
                section: DecompressionError::FileSize,
            })
    };
    read_line(reader, line_buf, position, handler).and_then(|val| match val {
        Some(size) => Ok(size),
        None => Err(ArchiveError::CorruptedArchive {
            position: *position,
            section: DecompressionError::FileSize,
        }),
    })
}

struct ExtractFileError {
    error: ArchiveError,
    // If the problem is recoverable, contains bytes that weren't yet read
    remaining: Option<usize>,
}
/// Conveniency implementation for all fatal conditions
impl From<ArchiveError> for ExtractFileError {
    fn from(err: ArchiveError) -> Self {
        Self {
            error: err,
            remaining: None,
        }
    }
}

/// Read contents of the file from the archive into provided sink
///
/// `count` is number of bytes file should have
///
/// Note that if function fails to write into provided sink, remaining byte count
/// is returned, allow skipping the rest of the file
fn extract_file(
    reader: &mut (impl BufRead + Seek),
    count: usize,
    sink: &mut impl Write,
    sink_name: &Path,
    position: &mut usize,
) -> Result<(), ExtractFileError> {
    use ArchiveError as E;

    let mut remaining_size = count;
    loop {
        let buf = reader.fill_buf().map_err(E::ArchiveIo)?;
        let read_upto = min(remaining_size, buf.len());
        if read_upto == 0 {
            if remaining_size > 0 {
                return Err(E::CorruptedArchive {
                    position: *position,
                    section: DecompressionError::Payload,
                }
                .into());
            }
            break;
        }
        sink.write_all(&buf[..read_upto])
            .map_err(|e| ExtractFileError {
                error: E::BadFileIo(sink_name.to_owned(), e),
                remaining: Some(remaining_size),
            })?;
        reader.consume(read_upto);
        *position += read_upto;
        remaining_size -= read_upto;
    }
    // Footer
    if !read_matches(reader, b"\n", position).map_err(E::ArchiveIo)? {
        return Err(E::CorruptedArchive {
            position: *position,
            section: DecompressionError::Footer,
        }
        .into());
    }
    Ok(())
}

/// Advances reading cursor to the end of given file in the archive, using
/// `count` as expected (remaining) file size
fn skip_file(
    reader: &mut (impl BufRead + Seek),
    count: usize,
    position: &mut usize,
) -> Result<(), ArchiveError> {
    // Size bigger than positive signed offset!
    let offset = count
        .try_into()
        .map_err(|_| ArchiveError::CorruptedArchive {
            position: *position,
            section: DecompressionError::FileSize,
        })?;
    reader
        .seek(io::SeekFrom::Current(offset))
        .map_err(ArchiveError::ArchiveIo)?;
    *position += count;
    // Footer
    if !read_matches(reader, b"\n", position).map_err(ArchiveError::ArchiveIo)? {
        return Err(ArchiveError::CorruptedArchive {
            position: *position,
            section: DecompressionError::Footer,
        });
    }
    Ok(())
}

/// Lower level decompression interface for DCA archives.
///
/// Compared to [`decompress_files`], allows detailed custom handling of various errors,
/// see [`Handler`] and [`ArchiveError`] respectively for details.
pub fn decompress_from(
    reader: &mut (impl BufRead + Seek),
    work_directory: &Path,
    handle: &impl Handler,
) -> Result<(), ArchiveError> {
    use ArchiveError as E;

    let mut position = 0usize;

    // Header
    if !read_matches(reader, b"DCA\n", &mut position).map_err(E::ArchiveIo)? {
        return Err(E::CorruptedArchive {
            position,
            section: DecompressionError::Header,
        });
    }

    let mut line_buf = String::new();
    loop {
        let fname: String =
            match read_line(reader, &mut line_buf, &mut position, |s| Ok(s.to_owned()))? {
                // Final file
                None => break,
                Some(fname) => fname,
            };

        let fsize = read_file_size(reader, &mut line_buf, &mut position)?;

        let fname_buf: PathBuf = [work_directory, Path::new(&fname)].iter().collect();
        let file = handled!(
            try { File::create(&fname_buf) }
            else if handle(|e| E::BadFileIo(fname_buf.clone(), e)) {
                skip_file(reader, fsize, &mut position)?;
                continue;
            }
        );
        let write_single_file = || {
            let mut writer = io::BufWriter::new(file);

            extract_file(reader, fsize, &mut writer, &fname_buf, &mut position)
        };
        handled!(
            try {
                match write_single_file() {
                    Ok(()) => Ok(()),
                    Err(err) => {
                        if let Err(del_err) = fs::remove_file(&fname_buf) {
                            error!("Extraction of {:?} failed, but the temporary file couldn't be deleted due to error {}. Please remove it manually.", fname_buf, del_err);
                        }
                        if let Some(size) = err.remaining {
                            skip_file(reader, size, &mut position)?;
                            Err(err.error)
                        }
                        else {
                            return Err(err.error);
                        }
                    }
                }
            }
            else if handle {}
        )
    }

    Ok(())
}

/// Decompresses DCA archive `archive_name` into `work_directory`.
///
/// Has simple high-level interface that skips and logs out files that fail to extract - (see [`decompress_from`]
/// if more control over process is desired.
///
/// On failure during extraction, already extracted files are kept while files in the middle of extraction
/// get deleted.
///
/// # Examples
///
/// ```no_run
/// use dca::decompress_files;
///
/// decompress_files("subdir/archive.dca", "outputdir")
///     .expect("decompression failed");
/// ```
/// ```no_run
/// use dca::decompress_files;
/// use dca::error::ArchiveError;
///
/// match decompress_files("archive.dca", "output") {
///     Ok(()) => println!("Archive extracted."),
///     Err(ArchiveError::CorruptedArchive{position, ..}) => println!("Archive corrupted at position {}!", position),
///     Err(ArchiveError::BadFileIo(path, _)) => println!("Couldn't extract {:?}.", path),
///     Err(_) => println!("Failed to extract archive."),
/// }
/// ```
pub fn decompress_files(archive_name: impl AsRef<Path>, work_directory: impl AsRef<Path>) -> Result<(), ArchiveError> {
    let archive_name = archive_name.as_ref();
    let work_directory = work_directory.as_ref();

    let handler = DefaultHandler::new(archive_name);

    let arch = File::open(archive_name).map_err(|e| {
        let e = ArchiveError::ArchiveIo(e);
        handler.on_fatal(&e);
        e
    })?;
    let mut reader = io::BufReader::new(arch);

    decompress_from(&mut reader, work_directory, &handler).map_err(|e| {
        handler.on_fatal(&e);
        e
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::fs::read_dir;
    use std::io::Cursor;
    use assert_fs::{prelude::*, TempDir};

    fn make_dir() -> TempDir {
        TempDir::new().unwrap()
    }

    fn check_dir_size(dir: impl AsRef<Path>, size: usize) {
        assert_eq!(read_dir(dir.as_ref()).unwrap().into_iter().count(), size);
    }

    #[test]
    fn test_empty() {
        let dir = make_dir();

        let mut contents = Cursor::new(b"DCA\n");
        decompress_from(
            &mut contents,
            dir.path(),
            &DefaultHandler::new(Path::new("")),
        )
        .unwrap();

        check_dir_size(dir, 0);
    }

    #[test]
    fn test_single() {
        let dir = make_dir();

        let mut contents = Cursor::new(b"DCA\nhello\n5\nworld\n");
        decompress_from(
            &mut contents,
            dir.path(),
            &DefaultHandler::new(Path::new("")),
        )
        .unwrap();

        check_dir_size(&dir, 1);
        dir.child("hello").assert(b"world" as &[u8]);
    }

    #[test]
    fn test_multiple() {
        let dir = make_dir();

        let mut contents =
            Cursor::new(b"DCA\nbinary\n6\n\x00\xFF\x80123\ntext\n6\n\ndca\n\n\nempty\n0\n\n");
        decompress_from(
            &mut contents,
            dir.path(),
            &DefaultHandler::new(Path::new("")),
        )
        .unwrap();

        check_dir_size(&dir, 3);
        dir.child("binary").assert(b"\x00\xFF\x80123" as &[u8]);
        dir.child("text").assert(b"\ndca\n\n" as &[u8]);
        dir.child("empty").assert(b"" as &[u8]);
    }

    #[test]
    fn test_errors() {
        let dir = make_dir();
        let handler = &DefaultHandler::new(Path::new("bad"));

        let mut contents = Cursor::new(b"");
        let err = decompress_from(&mut contents, dir.path(), handler).unwrap_err();
        match err {
            ArchiveError::ArchiveIo(io_err) if io_err.kind() == io::ErrorKind::UnexpectedEof => (),
            e => panic!("Unexpected error type {:?}", e),
        }

        let mut contents = Cursor::new(b"DCAv2\nfoo\n3\nbar\n");
        let err = decompress_from(&mut contents, dir.path(), handler).unwrap_err();
        match err {
            ArchiveError::CorruptedArchive {
                position,
                section: DecompressionError::Header,
            } => assert!((0..=4).contains(&position)),
            e => panic!("Unexpected error type {:?}", e),
        }

        let mut contents = Cursor::new(b"DCA\nfoo\n1000\nbar");
        let err = decompress_from(&mut contents, dir.path(), handler).unwrap_err();
        match err {
            ArchiveError::CorruptedArchive {
                position: 16,
                section: DecompressionError::Payload,
            } => (),
            e => panic!("Unexpected error type {:?}", e),
        }

        let mut contents = Cursor::new(b"DCA\nfoo\n3\nbar");
        let err = decompress_from(&mut contents, dir.path(), handler).unwrap_err();
        match err {
            ArchiveError::ArchiveIo(io_err) if io_err.kind() == io::ErrorKind::UnexpectedEof => (),
            e => panic!("Unexpected error type {:?}", e),
        }
    }

    #[test]
    fn test_handle() {
        let dir = make_dir();
        // Existence of subdirectory should force inability to create file of same name
        dir.child("bad").create_dir_all().unwrap();

        struct LaxHandler;
        impl Handler for LaxHandler {
            fn on_err(&self, err: ArchiveError) -> Result<(), ArchiveError> {
                if let ArchiveError::BadFileIo(_, _) = err {
                    Ok(())
                } else {
                    Err(err)
                }
            }
        }

        let mut contents = Cursor::new(
            b"DCA\nfoo\n3\n123\n\
            bad\n3\n456\n\
            bar\n3\n789\n"
        );
        decompress_from(&mut contents, dir.path(), &LaxHandler).unwrap();

        check_dir_size(&dir, 3);
        dir.child("foo").assert("123");
        dir.child("bad").assert(predicates::path::is_dir());
        dir.child("bar").assert("789");

        for entry in read_dir(dir.path()).unwrap() {
            let entry = entry.unwrap();
            match entry {
                entry if entry.path().is_dir() => {
                    assert_eq!(entry.path(), dir.child("bad").path());
                }
                entry if entry.path().is_file() => {
                    assert!(["foo", "bar"]
                        .contains(&entry.path().file_name().and_then(|x| x.to_str()).unwrap()));
                }
                e => panic!("Unexpected directory entry {:?}", e),
            }
        }
    }
}

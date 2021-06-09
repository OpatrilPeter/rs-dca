//! Handles extraction from DCA archives

use std::cmp::min;
use std::fs::{self, File};
use std::io::{self, prelude::*, BufRead, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use crate::error::{
    error, warn, ArchiveError, DecompressionError, FilePosition, Handler as ErrorHandler,
};

use ArchiveError as E;

/// [`ErrorHandler`] that allows extraction failures, logging each encountered problem
pub struct DefaultErrorHandler<'a> {
    archive_name: &'a Path,
}

impl<'a> DefaultErrorHandler<'a> {
    pub fn new(archive_name: &'a Path) -> Self {
        Self { archive_name }
    }
    fn on_fatal(&self, err: &ArchiveError) {
        match err {
            E::ArchiveIo(io_err) => {
                error!(
                    "Failed read from archive {:?} due to following error: {}",
                    self.archive_name, io_err
                );
            }
            E::CorruptedArchive { position, section } => {
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

impl<'a> ErrorHandler for DefaultErrorHandler<'a> {
    fn on_err(&self, err: ArchiveError) -> Result<(), ArchiveError> {
        match err {
            E::BadFileIo(fname, io_err) => {
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
    position: &mut FilePosition,
) -> Result<bool, io::Error> {
    let mut buf = [0u8; N];
    reader.read_exact(&mut buf)?;

    let res = &buf == reference;
    if res {
        *position += N as FilePosition;
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
    position: &mut FilePosition,
    processor: impl FnOnce(&str) -> Result<T, ArchiveError>,
) -> Result<Option<T>, ArchiveError> {
    line_buf.truncate(0);
    reader.read_line(line_buf).map_err(E::ArchiveIo)?;
    if line_buf.is_empty() {
        return Ok(None);
    }
    let res = processor(&line_buf[..line_buf.len() - 1]);
    if res.is_ok() {
        *position += line_buf.len() as FilePosition;
    }
    res.map(Some)
}

/// Reads file size segment from the archive
fn read_file_size(
    reader: &mut impl BufRead,
    line_buf: &mut String,
    position: &mut FilePosition,
) -> Result<FilePosition, ArchiveError> {
    let old_pos = *position;
    let handler = |s: &str| {
        s.parse::<FilePosition>().map_err(|_| E::CorruptedArchive {
            position: old_pos,
            section: DecompressionError::FileSize,
        })
    };
    read_line(reader, line_buf, position, handler).and_then(|val| match val {
        Some(size) => Ok(size),
        None => Err(E::CorruptedArchive {
            position: *position,
            section: DecompressionError::FileSize,
        }),
    })
}

/// Reads contents of the file from the archive into provided sink
///
/// `count` is number of bytes file should have
///
/// Note that if function fails to write into provided sink, remaining byte count
/// is returned, allow skipping the rest of the file
fn extract_file(
    reader: &mut impl BufRead,
    count: FilePosition,
    sink: &mut impl Write,
    sink_name: &Path,
) -> Result<(), ArchiveError> {
    // Implementation note: the casts between usize and FilePosition seem unifiable, but aren't
    // Buffer is usize(d), which in theoretical case may be much smaller than size of DCA entries
    let mut remaining_size = count;
    loop {
        let buf = reader.fill_buf().map_err(E::ArchiveIo)?;
        let read_upto = min(remaining_size, buf.len() as FilePosition) as usize;
        if read_upto == 0 {
            if remaining_size > 0 {
                return Err(E::ArchiveIo(io::Error::from(io::ErrorKind::UnexpectedEof)));
            }
            break;
        }
        sink.write_all(&buf[..read_upto])
            .map_err(|e| E::BadFileIo(sink_name.to_owned(), e))?;
        reader.consume(read_upto);
        remaining_size -= read_upto as FilePosition;
    }
    Ok(())
}

/// Metadata about file about to be extracted
pub struct FileDescriptor<'a, R: BufRead> {
    /// Archive entry filename. Shall be valid DCA entry filename
    pub name: &'a str,
    /// File size of the file entry in bytes
    pub len: FilePosition,
    /// From this reader, you can read up to [`Self::len`] bytes. Attempt to reading more than that is, however, well defined and results in EOF
    pub reader: &'a mut R,
}

/// Represents callback for consuming files extracted with [`decompress_from`].
pub trait FileHandler {
    /// Takes (appropriately sized) reader representing the file contents, along with additional
    /// metadata.
    ///
    /// This callback may fail in two ways - either due to the read from the reader (use [`ArchiveError::ArchiveIo`])
    /// or due to internal errors (use [`ArchiveError::BadFileIo`]).
    ///
    /// Usage of `BadFileIo` indicates that further exctraction from archive is still possible.
    /// Note that final position in reader is irrelevant.
    fn on_file<R: BufRead>(&mut self, file: FileDescriptor<'_, R>) -> Result<(), ArchiveError>;
}

/// Standard [`FileHandler`] implemented by extracting all files into given directory
pub struct DefaultFileHandler<'a> {
    work_directory: &'a Path,
}
impl<'a> DefaultFileHandler<'a> {
    pub fn new(work_directory: &'a Path) -> Self {
        Self { work_directory }
    }
}

impl<'a> FileHandler for DefaultFileHandler<'a> {
    fn on_file<'b, R: BufRead>(
        &'b mut self,
        file: FileDescriptor<'b, R>,
    ) -> Result<(), ArchiveError> {
        let FileDescriptor {
            name: fname,
            reader,
            len,
        } = file;
        let fname_buf: PathBuf = self.work_directory.join(&fname);

        let bad_io = |e| E::BadFileIo(fname_buf.clone(), e);
        let file = File::create(&fname_buf).map_err(bad_io)?;

        let write_file = || {
            let mut writer = io::BufWriter::new(file);

            extract_file(reader, len, &mut writer, &fname_buf)
        };
        match write_file() {
            Ok(()) => Ok(()),
            Err(err) => {
                if let Err(del_err) = fs::remove_file(&fname_buf) {
                    error!("Extraction of {:?} failed, but the temporary file couldn't be deleted due to error {}. Please remove it manually.", fname_buf, del_err);
                }
                Err(err)
            }
        }
    }
}

/// Simple [`FileHandler`] that doesn't extract the file, just delegates it to underlying callable
pub struct CallbackFileHandler<C>(pub C)
where
    C: FnMut(&str, FilePosition, &mut dyn BufRead) -> Result<(), ArchiveError>;

impl<C> FileHandler for CallbackFileHandler<C>
where
    C: FnMut(&str, FilePosition, &mut dyn BufRead) -> Result<(), ArchiveError>,
{
    fn on_file<R: BufRead>(&mut self, file: FileDescriptor<'_, R>) -> Result<(), ArchiveError> {
        (self.0)(file.name, file.len, file.reader)
    }
}

// Note: simpler wrapper-less version of CallbackFileHandler that compiler rejects on use
// impl<C> FileHandler for C
// where
//     C: FnMut(&str, FilePosition, &mut dyn BufRead) -> Result<(), ArchiveError>,
// {
//     fn on_file<R: BufRead>(&mut self, file: FileDescriptor<'_, R>) -> Result<(), ArchiveError>
//     {
//         self(file.name, file.len, file.reader)
//     }
// }

/// Lower level decompression interface for DCA archives.
///
/// Instead of extracting files outright, it represents them as [`FileDescriptor`]
/// and deledates handling to [`FileHandler`].
///
/// Also allows detailed custom handling of various errors through [`ErrorHandler`],
/// notably including ignoring failed extractions outright.
pub fn decompress_from(
    reader: &mut (impl BufRead + Seek),
    handle_file: &mut impl FileHandler,
    handle_err: &impl ErrorHandler,
) -> Result<(), ArchiveError> {
    let mut position = reader.stream_position().map_err(E::ArchiveIo)?;

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

        let footer_position = position + fsize;

        match handle_file.on_file(FileDescriptor {
            name: &fname,
            len: fsize,
            reader: &mut reader.take(fsize as FilePosition),
        }) {
            Ok(()) => (),
            Err(e) => match e {
                E::ArchiveIo(io_err) if io_err.kind() == io::ErrorKind::UnexpectedEof => {
                    // Shouldn't fail
                    let position = reader.seek(SeekFrom::End(0)).unwrap_or(FilePosition::MAX);
                    return Err(E::CorruptedArchive {
                        position,
                        section: DecompressionError::Payload,
                    });
                }
                E::BadFileIo(..) => handle_err.on_err(e)?,
                E::ArchiveIo(..) => return Err(e),
                _ => {
                    warn!(
                        "FileHandler of decompress_from returned unexpected error type {:?}",
                        e
                    );
                    return Err(e);
                }
            },
        }
        // This is mildly redundant if handler is well behaved and already fully reads up to this point,
        // but we can't depend on soundness of external code + it streamlines the handler contract
        reader
            .seek(SeekFrom::Start(footer_position))
            .map_err(E::ArchiveIo)?;
        position = footer_position;
        // Footer
        if !read_matches(reader, b"\n", &mut position).map_err(E::ArchiveIo)? {
            return Err(ArchiveError::CorruptedArchive {
                position,
                section: DecompressionError::Footer,
            });
        }
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
pub fn decompress_files(
    archive_name: impl AsRef<Path>,
    work_directory: impl AsRef<Path>,
) -> Result<(), ArchiveError> {
    let archive_name = archive_name.as_ref();
    let work_directory = work_directory.as_ref();

    let mut fhandler = DefaultFileHandler::new(work_directory);
    let ehandler = DefaultErrorHandler::new(archive_name);

    let arch = File::open(archive_name).map_err(|e| {
        let e = ArchiveError::ArchiveIo(e);
        ehandler.on_fatal(&e);
        e
    })?;
    let mut reader = io::BufReader::new(arch);

    decompress_from(&mut reader, &mut fhandler, &ehandler).map_err(|e| {
        ehandler.on_fatal(&e);
        e
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use assert_fs::prelude::*;
    use std::borrow::Borrow;
    use std::fs::read_dir;
    use std::io::Cursor;

    use crate::testutils::*;

    fn files(dir: &impl AsRef<Path>) -> DefaultFileHandler<'_> {
        DefaultFileHandler::new(dir.as_ref())
    }

    fn std_errors() -> DefaultErrorHandler<'static> {
        DefaultErrorHandler::new(Path::new("test-archive.dca"))
    }

    #[test]
    fn test_empty() {
        let dir = make_dir();

        let mut contents = Cursor::new(b"DCA\n");
        decompress_from(
            &mut contents,
            &mut files(&dir),
            &DefaultErrorHandler::new(Path::new("")),
        )
        .unwrap();

        assert_eq!(dir_size(&dir), 0);
    }

    #[test]
    fn test_single() {
        let dir = make_dir();

        let mut contents = Cursor::new(b"DCA\nhello\n5\nworld\n");
        decompress_from(&mut contents, &mut files(&dir), &std_errors()).unwrap();

        assert_eq!(dir_size(&dir), 1);
        dir.child("hello").assert(b"world" as &[u8]);
    }

    #[test]
    fn test_multiple() {
        let dir = make_dir();

        let mut contents =
            Cursor::new(b"DCA\nbinary\n6\n\x00\xFF\x80123\ntext\n6\n\ndca\n\n\nempty\n0\n\n");
        decompress_from(&mut contents, &mut files(&dir), &std_errors()).unwrap();

        assert_eq!(dir_size(&dir), 3);
        dir.child("binary").assert(b"\x00\xFF\x80123" as &[u8]);
        dir.child("text").assert(b"\ndca\n\n" as &[u8]);
        dir.child("empty").assert(b"" as &[u8]);
    }

    #[test]
    fn test_errors() {
        let dir = make_dir();
        let handler = &DefaultErrorHandler::new(Path::new("bad"));

        let mut contents = Cursor::new(b"");
        let err = decompress_from(&mut contents, &mut files(&dir), handler).unwrap_err();
        match err {
            ArchiveError::ArchiveIo(io_err) if io_err.kind() == io::ErrorKind::UnexpectedEof => (),
            e => panic!("Unexpected error type {:?}", e),
        }

        let mut contents = Cursor::new(b"DCAv2\nfoo\n3\nbar\n");
        let err = decompress_from(&mut contents, &mut files(&dir), handler).unwrap_err();
        match err {
            ArchiveError::CorruptedArchive {
                position,
                section: DecompressionError::Header,
            } => assert!((0..=4).contains(&position)),
            e => panic!("Unexpected error type {:?}", e),
        }

        let mut contents = Cursor::new(b"DCA\nfoo\n1000\nbar");
        let err = decompress_from(&mut contents, &mut files(&dir), handler).unwrap_err();
        match err {
            ArchiveError::CorruptedArchive {
                position: 16,
                section: DecompressionError::Payload,
            } => (),
            e => panic!("Unexpected error type {:?}", e),
        }

        let mut contents = Cursor::new(b"DCA\nfoo\n3\nbar");
        let err = decompress_from(&mut contents, &mut files(&dir), handler).unwrap_err();
        match err {
            ArchiveError::ArchiveIo(io_err) if io_err.kind() == io::ErrorKind::UnexpectedEof => (),
            e => panic!("Unexpected error type {:?}", e),
        }
    }

    #[test]
    fn test_err_handler() {
        let dir = make_dir();
        // Existence of subdirectory should force inability to create file of same name
        dir.child("bad").create_dir_all().unwrap();

        struct LaxHandler;
        impl ErrorHandler for LaxHandler {
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
            bar\n3\n789\n",
        );
        decompress_from(&mut contents, &mut files(&dir), &LaxHandler).unwrap();

        assert_eq!(dir_size(&dir), 3);
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

    #[test]
    fn test_file_handler() {
        #[derive(Default, Debug)]
        struct VecFiles(Vec<(String, String)>);
        impl FileHandler for VecFiles {
            fn on_file<R: BufRead>(
                &mut self,
                file: FileDescriptor<'_, R>,
            ) -> Result<(), ArchiveError> {
                let mut buf = String::new();
                file.reader.read_to_string(&mut buf).unwrap();
                self.0.push((file.name.to_owned(), buf));
                Ok(())
            }
        }

        let mut contents = Cursor::new(
            b"DCA\n\
            first\n3\n123\n\
            second\n5\nhello\n",
        );
        let mut files = VecFiles::default();
        decompress_from(&mut contents, &mut files, &std_errors()).unwrap();

        let results = vec![("first", "123"), ("second", "hello")];
        assert_eq_iters(files.0.iter().map(|(a, b)| (a.borrow(), b.borrow())), results.into_iter());
    }
}

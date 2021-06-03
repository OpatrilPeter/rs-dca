//! Handles creation of DCA archives

use std::fs::{self, File};
use std::io::{self, prelude::*};
use std::path::Path;

use crate::error::{dca_filename, error, ArchiveError, FilePosition, Handler as ErrorHandler};

/// Error [`Handler`] that fails on every condition, logging each encountered problem
pub struct DefaultErrorHandler<'a> {
    archive_name: &'a Path,
}
impl<'a> DefaultErrorHandler<'a> {
    pub fn new(archive_name: &'a Path) -> Self {
        Self { archive_name }
    }
    pub fn on_fatal(&self, err: &ArchiveError) {
        use ArchiveError::*;
        match err {
            ArchiveIo(io_err) => {
                error!(
                    "Failed to write into archive {:?} due to following error: {}",
                    self.archive_name, io_err
                );
            }
            BadFileIo(fname, io_err) => {
                error!(
                    "Cannot add file {:?} into archive {:?} due to following error: {}",
                    fname, self.archive_name, io_err
                );
            }
            InvalidDcaFilename(fname, load_err) => {
                // TODO: precise error not pretty-printed
                error!("File {:?} does not have a legal filename and cannot be added into the archive, due to following error: {:?}", fname, load_err);
            }
            err => error!(
                "Creation of archive {:?} failed due to error {:?}",
                self.archive_name, err
            ),
        }
    }
}

impl<'a> ErrorHandler for DefaultErrorHandler<'a> {
    fn on_err(&self, err: ArchiveError) -> Result<(), ArchiveError> {
        // All errors are fatal
        Err(err)
    }
}

pub struct FileDescriptor<'a, R: BufRead> {
    pub path: &'a Path,
    pub reader: R,
    pub len: FilePosition,
}

/// Allows full customization of [`compress_into`] input file handling.
pub trait FileHandler {
    type Reader: BufRead;
    /// Event-driven Iterator-like protocol.
    ///
    /// Is called once for each file to be added with actual archive appending logic passed as `compress` callback.
    /// Allows implementaion to open/close output files or do some appropriate equivalent.
    ///
    /// Various possible results:
    ///
    /// Ok(None) => No more file to add
    /// Ok(Some(())) => File processed successfuly
    /// Err(_) => [`ArchiveError`] - either from underlying callback or internally created [`io::Error`] promoted into [`ArchiveError::BadFileIo`]
    fn add_file<Callback>(&mut self, compress: Callback) -> Result<Option<()>, ArchiveError>
    where
        Callback: FnOnce(FileDescriptor<'_, Self::Reader>) -> Result<(), ArchiveError>;
}

/// Handler for [`compress_into`], feeding it list of files.
/// This class is responsible for opening and closing eachg
pub struct DefaultFileHandler<I>
// where
//     I: Iterator,
//     I::Item : AsRef<Path>
{
    /// Iterable of files (file paths)
    files: I,
}
impl<I> DefaultFileHandler<I> {
    pub fn new<II>(files: II) -> Self
    where
        II: IntoIterator<IntoIter = I>,
        I: Iterator,
        I::Item: AsRef<Path>,
    {
        Self {
            files: files.into_iter(),
        }
    }
}

impl<I> FileHandler for DefaultFileHandler<I>
where
    I: Iterator,
    I::Item: AsRef<Path>,
{
    type Reader = io::BufReader<fs::File>;
    fn add_file<Callback>(&mut self, compress: Callback) -> Result<Option<()>, ArchiveError>
    where
        Callback: FnOnce(FileDescriptor<'_, Self::Reader>) -> Result<(), ArchiveError>,
    {
        use ArchiveError as E;

        let file_path = match self.files.next() {
            None => return Ok(None),
            Some(f) => f,
        };
        let file_path = file_path.as_ref();
        let bad_io = |e| E::BadFileIo(file_path.to_owned(), e);

        let file = File::open(file_path).map_err(bad_io)?;
        let mut reader = io::BufReader::new(file);
        let file_len = reader.seek(io::SeekFrom::End(0)).map_err(bad_io)?;
        reader.seek(io::SeekFrom::Start(0)).map_err(bad_io)?;

        compress(FileDescriptor {
            path: file_path,
            reader,
            len: file_len,
        })?;
        Ok(Some(()))
    }
}

/// Lower level DCA archive construction interface.
///
/// The functionality is customizable by event handlers in following way:
///
/// Writes into standard Writer.
///
/// Instead of operating on filesystem directly, it relies on [`FileHandler`] to provide it
/// with input file abstractions.
///
/// Nonfatal error states are passed into [`ErrorHandler`] that may decide to transform them,
/// abort the compression or ignore them.
///
/// Also see [compress_files] for more hands-off interface.
pub fn compress_into(
    writer: &mut impl Write,
    handle_file: &mut impl FileHandler,
    handle_err: &mut impl ErrorHandler,
) -> Result<(), ArchiveError> {
    use ArchiveError as E;

    writer.write_all(b"DCA\n").map_err(E::ArchiveIo)?;
    loop {
        match handle_file.add_file(|file| {
            let FileDescriptor {
                mut reader,
                path,
                len,
            } = file;

            // Validate filename
            let fname = path.file_name().ok_or_else(|| {
                E::BadFileIo(path.to_owned(), io::Error::from(io::ErrorKind::NotFound))
            })?;

            let name =
                dca_filename(fname).map_err(|e| E::InvalidDcaFilename(path.to_owned(), e))?;

            writer
                .write_fmt(format_args!("{}\n{}\n", name, len))
                .map_err(E::ArchiveIo)?;

            loop {
                let buf = reader
                    .fill_buf()
                    .map_err(|e| E::BadFileIo(path.to_owned(), e))?;
                if buf.is_empty() {
                    break;
                }
                let bytes = writer.write(buf).map_err(E::ArchiveIo)?;
                reader.consume(bytes);
            }
            writer.write_all(b"\n").map_err(E::ArchiveIo)?;
            Ok(())
        }) {
            Ok(None) => break,
            Ok(Some(())) => (),
            Err(err) => match &err {
                // We can't comtinue compressing if we're in inconsistent state
                E::ArchiveIo(_) => return Err(err),
                _ => handle_err.on_err(err)?,
            },
        }
    }
    Ok(())
}

/// Compresses list of files into new DCA archive.
///
/// The new archive will be created at `archive_name` path,
/// (_not_ creating nonexisting directories).
/// Accepts list of paths to individual files, but as DCA format
/// is flat, no directories are permitted. Multiple files with same name
/// can be technically stored in the archive, but there's no additional
/// metadata to disambiguate them.
///
/// May fail for various I/O reasons, see [`ArchiveError`] for details. Fails on first error - if
/// you're interested in more tuneable compression, see [`compress_into`].
///
/// # Example
///
/// ```no_run
/// use dca::compress_files;
///
/// compress_files(&["text.txt", "src.rs", "binary.blob"], "archive.dca")
///     .expect("failed to create the archive");
/// ```
pub fn compress_files<PathIter>(
    files: PathIter,
    archive_name: impl AsRef<Path>,
) -> Result<(), ArchiveError>
where
    PathIter: IntoIterator,
    PathIter::Item: AsRef<Path>,
{
    let archive_name = archive_name.as_ref();

    let mut fhandler = DefaultFileHandler::new(files);
    let mut ehandler = DefaultErrorHandler::new(archive_name);

    let arch = File::create(archive_name).map_err(|e| {
        let e = ArchiveError::ArchiveIo(e);
        ehandler.on_fatal(&e);
        e
    })?;
    let mut writer = io::BufWriter::new(arch);
    compress_into(&mut writer, &mut fhandler, &mut ehandler).map_err(|e| {
        ehandler.on_fatal(&e);
        if let Err(io_err) = fs::remove_file(archive_name) {
            error!("Removal of incorrectly created archive {:?} failed with error {}, please remove it manually.", archive_name, io_err);
        }
        e
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::error::DcaFilenameError;
    use assert_fs::{prelude::*, TempDir};
    use std::ffi::OsStr;
    use std::io::BufReader;

    fn std_errors() -> DefaultErrorHandler<'static> {
        DefaultErrorHandler::new(Path::new("test-archive.dca"))
    }
    fn files<P: AsRef<Path>>(slice: &[P]) -> DefaultFileHandler<impl Iterator<Item = &P>> {
        DefaultFileHandler::new(slice)
    }

    #[test]
    fn test_empty() {
        let mut out = Vec::<u8>::new();
        compress_into(&mut out, &mut files(&[] as &[&Path]), &mut std_errors())
            .expect("Failed to compress file");

        assert_eq!(out, b"DCA\n");
    }

    #[test]
    fn test_single_file() {
        let dir = TempDir::new().unwrap();
        dir.child("test").write_str("Hello world!").unwrap();

        let mut out = Vec::<u8>::new();
        compress_into(
            &mut out,
            &mut files(&[dir.child("test")]),
            &mut std_errors(),
        )
        .expect("Failed to compress file");

        assert_eq!(out, b"DCA\ntest\n12\nHello world!\n");
    }

    #[test]
    fn test_many_files() {
        let dir = TempDir::new().unwrap();

        dir.child("binary")
            .write_binary(b"\x00\xFF314\x10\x10")
            .unwrap();
        dir.child("empty").touch().unwrap();
        dir.child("large")
            .write_binary(&[0xDEu8; 10 * 1024 * 1024])
            .unwrap();
        dir.child("text").write_str("dumb\ncat\narchive\n").unwrap();

        let mut out = Vec::<u8>::new();
        compress_into(
            &mut out,
            &mut files(&[
                dir.child("empty"),
                dir.child("large"),
                dir.child("binary"),
                dir.child("text"),
            ]),
            &mut std_errors(),
        )
        .expect("Failed to compress file");

        #[rustfmt::skip]
        let contents = [b"DCA\n" as &[u8],
            b"empty\n0\n\n",
            b"large\n", (10 * 1024 * 1024i32).to_string().as_bytes(), b"\n", &[0xDEu8; 10 * 1024 * 1024], b"\n",
            b"binary\n7\n\x00\xFF314\x10\x10\n",
            b"text\n17\ndumb\ncat\narchive\n\n",
        ].concat().to_vec();

        assert_eq!(out, contents);
    }

    #[test]
    fn test_errors() {
        let dir = TempDir::new().unwrap();
        {
            let mut out = Vec::<u8>::new();
            let bad = compress_into(
                &mut out,
                &mut files(&[dir.child("nonexisting")]),
                &mut std_errors(),
            )
            .unwrap_err();
            match bad {
                ArchiveError::BadFileIo(path, io_err)
                    if path == dir.child("nonexisting").path() =>
                {
                    assert!(io_err.kind() == io::ErrorKind::NotFound);
                }
                e => panic!("Unexpected error {:?}", e),
            }
        }
        {
            let mut out = Vec::<u8>::new();
            let bad = compress_into(
                &mut out,
                // This name should be invalid (regardless if file of such name can exist on filesystem)
                &mut files(&[&Path::new("\n\x01\x00")]),
                &mut std_errors(),
            )
            .unwrap_err();
            match bad {
                ArchiveError::InvalidDcaFilename(_, DcaFilenameError::InvalidChar('\n', 0)) => (),
                ArchiveError::BadFileIo(_, io_err)
                    if io_err.kind() == io::ErrorKind::InvalidInput =>
                {
                    ()
                }
                e => panic!("Unexpected error {:?}", e),
            }
        }
    }

    #[test]
    fn test_err_handler() {
        struct CustHandler;
        impl ErrorHandler for CustHandler {
            fn on_err(&self, err: ArchiveError) -> Result<(), ArchiveError> {
                match err {
                    ArchiveError::BadFileIo(path, io_err)
                        if path.file_name() == Some(OsStr::new("nonexistent"))
                            && io_err.kind() == io::ErrorKind::NotFound =>
                    {
                        Ok(())
                    }
                    e => panic!("Unexpected handleable error {:?}", e),
                }
            }
        }

        let dir = TempDir::new().unwrap();
        dir.child("file").write_str("data").unwrap();

        let mut out = Vec::<u8>::new();
        compress_into(
            &mut out,
            &mut files(&[dir.child("file"), dir.child("nonexistent")]),
            &mut CustHandler,
        )
        .unwrap();

        assert_eq!(out, b"DCA\nfile\n4\ndata\n");
    }

    #[test]
    /// We pass directory names instead
    fn test_invalid_name() {
        let dir = TempDir::new().unwrap();

        let mut out = Vec::<u8>::new();

        let bad = compress_into(
            &mut out,
            &mut files(&[Path::new("/"), &dir.path().join("..")]),
            &mut std_errors(),
        )
        .unwrap_err();
        match bad {
            ArchiveError::BadFileIo(path, io_err)
                if path == Path::new("/") && io_err.kind() == io::ErrorKind::NotFound =>
            {
                ()
            }
            _ => panic!("Unexpected error {:?}", bad),
        }
    }

    #[test]
    fn test_file_handler() {
        // For content just returns filenames

        struct Handler<I>(I);
        impl<'a, I: Iterator<Item = &'a &'static str>> FileHandler for Handler<I> {
            type Reader = std::io::BufReader<&'a [u8]>;
            fn add_file<Callback>(&mut self, compress: Callback) -> Result<Option<()>, ArchiveError>
            where
                Callback: FnOnce(FileDescriptor<'_, Self::Reader>) -> Result<(), ArchiveError>,
            {
                if let Some(s) = self.0.next() {
                    let reader = BufReader::new(s.as_bytes());
                    let fd = FileDescriptor {
                        path: &Path::new(s),
                        reader,
                        len: s.len() as FilePosition,
                    };
                    compress(fd)?;
                    Ok(Some(()))
                } else {
                    Ok(None)
                }
            }
        }

        let mut out = Vec::<u8>::new();
        compress_into(
            &mut out,
            &mut Handler(["foo", "bar"].iter()),
            &mut std_errors(),
        )
        .unwrap();

        assert_eq!(out, b"DCA\nfoo\n3\nfoo\nbar\n3\nbar\n");
    }
}

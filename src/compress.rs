use std::fs::{self, File};
use std::io::{self, prelude::*};
use std::path::Path;

use crate::error::{dca_filename, error, ArchiveError, Handler};

// Implemented by using standard logging
#[cfg(feature = "logging")]
struct DefaultHandler<'a> {
    archive_name: &'a Path,
}
#[cfg(feature = "logging")]
impl<'a> DefaultHandler<'a> {
    fn new(archive_name: &'a Path) -> Self {
        Self { archive_name }
    }
    fn on_fatal(&self, err: &ArchiveError) {
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
#[cfg(not(feature = "logging"))]
struct DefaultHandler<'a> {
    _dummy: std::marker::PhantomData<&'a ()>,
}
#[cfg(not(feature = "logging"))]
impl<'a> DefaultHandler<'a> {
    fn new(_name: &Path) -> Self {Self{_dummy: std::marker::PhantomData}}
    fn on_fatal(&self, _err: &ArchiveError) {}
}

impl<'a> Handler for DefaultHandler<'a> {
    fn on_err(&self, err: ArchiveError) -> Result<(), ArchiveError> {
        // All errors are fatal
        Err(err)
    }
}

pub fn compress_into<PathIter, PathIterItem>(
    writer: &mut impl Write,
    files: PathIter,
    handle: &impl Handler,
) -> Result<(), ArchiveError>
where
    PathIter: IntoIterator<Item = PathIterItem>,
    PathIterItem: AsRef<Path>,
{
    use ArchiveError as E;

    writer.write_all(b"DCA\n").map_err(E::ArchiveIo)?;
    for file in files {
        // If expression is corect, returns it
        // Otherwise, exits early if handler decides so
        // Otherwise, executes code from failblock, allowing control flow change or alternative ok result
        macro_rules! handleable {
            ($e: expr, $map_err: expr, $fail_blk: block) => {
                match $e.map_err($map_err) {
                    Ok(ok) => ok,
                    Err(err) => {
                        handle.on_err(err)?;
                        $fail_blk;
                    }
                }
            };
        }
        let file_path = handleable!(
            file.as_ref().canonicalize(),
            |e| E::BadFileIo(file.as_ref().to_owned(), e),
            { continue }
        );
        let fname = match file_path.file_name() {
            None => {
                let io_err = io::Error::new(io::ErrorKind::InvalidInput, "input is not a filename");
                handle.on_err(E::BadFileIo(file_path, io_err))?;
                continue;
            }
            Some(fname) => {
                handleable!(
                    dca_filename(fname),
                    |e| E::InvalidDcaFilename(file_path.clone(), e),
                    { continue }
                )
            }
        };

        macro_rules! file_io_err {
            ($e: expr) => {
                handleable!($e, |e| E::BadFileIo(file_path.clone(), e), { continue })
            };
        }
        let subfile = file_io_err!(File::open(&file));
        let mut reader = io::BufReader::new(subfile);
        let subfile_len = file_io_err!(reader.seek(io::SeekFrom::End(0)));
        file_io_err!(reader.seek(io::SeekFrom::Start(0)));

        writer
            .write_fmt(format_args!("{}\n{}\n", fname, subfile_len))
            .map_err(E::ArchiveIo)?;

        loop {
            let buf = reader
                .fill_buf()
                .map_err(|e| E::BadFileIo(file_path.clone(), e))?;
            if buf.is_empty() {
                break;
            }
            let bytes = writer.write(buf).map_err(E::ArchiveIo)?;
            reader.consume(bytes);
        }
        writer.write_all(b"\n").map_err(E::ArchiveIo)?;
    }

    Ok(())
}

pub fn compress_files<PathIter, PathIterItem>(
    files: PathIter,
    archive_name: &Path,
) -> Result<(), ArchiveError>
where
    PathIter: IntoIterator<Item = PathIterItem>,
    PathIterItem: AsRef<Path>,
{
    let handler = DefaultHandler::new(archive_name);

    let arch = File::create(archive_name).map_err(|e| {
        let e = ArchiveError::ArchiveIo(e);
        handler.on_fatal(&e);
        e
    })?;
    let mut writer = io::BufWriter::new(arch);
    compress_into(&mut writer, files, &handler).map_err(|e| {
        handler.on_fatal(&e);
        if let Err(io_err) = fs::remove_file(archive_name) {
            error!("Removal of incorrectly created archive {:?} failed with error {}, please remove it manually.", archive_name, io_err);
        }
        e
    })
}

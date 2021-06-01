//! Handles extraction from DCA archives

use std::fs::{self, File};
use std::io::{self, prelude::*};
use std::path::Path;

use crate::error::{dca_filename, error, handled, ArchiveError, Handler};

/// Error Handler that fails on every condition, logging each encountered problem
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
        let file_path = handled!(
            try { file.as_ref().canonicalize() }
            else if handle(|e| E::BadFileIo(file.as_ref().to_owned(), e)) {
                continue
            }
        );
        let fname = match file_path.file_name() {
            None => {
                let io_err = io::Error::new(io::ErrorKind::InvalidInput, "input is not a filename");
                handle.on_err(E::BadFileIo(file_path, io_err))?;
                continue;
            }
            Some(fname) => {
                handled!(
                    try { dca_filename(fname) }
                    else if handle(|e| E::InvalidDcaFilename(file_path.clone(), e)) {
                        continue
                    }
                )
            }
        };

        macro_rules! file_io_err {
            ($e: expr) => {
                handled!(
                    try { $e }
                    else if handle(|e| E::BadFileIo(file_path.clone(), e)) { continue }
                )
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

#[cfg(test)]
mod tests {
    use super::*;

    use crate::error::DcaFilenameError;
    use tempfile::NamedTempFile;

    fn make_outfile(contents: &[u8]) -> (NamedTempFile, String, &[u8]) {
        let mut f = NamedTempFile::new().unwrap();
        f.write_all(contents).unwrap();
        let fname = f.path().file_name().unwrap().to_str().unwrap().to_owned();
        (f, fname, contents)
    }

    #[test]
    fn test_empty() {
        let mut out = Vec::<u8>::new();
        let empty: &[&Path] = &[];
        compress_into(&mut out, empty, &DefaultHandler::new(Path::new("no-file")))
            .expect("Failed to compress file");

        assert_eq!(out, b"DCA\n");
    }

    #[test]
    fn test_single_file() {
        let (file, fname, _) = make_outfile("Hello world!".as_bytes());

        let mut out = Vec::<u8>::new();
        compress_into(&mut out, &[file.path()], &DefaultHandler::new(file.path()))
            .expect("Failed to compress file");

        assert_eq!(
            out,
            [b"DCA\n", fname.as_bytes(), b"\n12\nHello world!\n"]
                .concat()
                .to_vec()
        );
    }

    #[test]
    fn test_many_files() {
        let binary = make_outfile(b"\x00\xFF314\x10\x10");
        let empty = make_outfile(b"");
        let large = make_outfile(&[0xDEu8; 10 * 1024 * 1024]);
        let text = make_outfile(b"dumb\ncat\narchive\n");

        let mut out = Vec::<u8>::new();
        compress_into(
            &mut out,
            &[
                empty.0.path(),
                large.0.path(),
                binary.0.path(),
                text.0.path(),
            ],
            &DefaultHandler::new(Path::new("large-archive.dca")),
        )
        .expect("Failed to compress file");

        #[rustfmt::skip]
        let contents = [b"DCA\n" as &[u8],
            empty.1.as_bytes(), b"\n", empty.2.len().to_string().as_bytes(), b"\n", empty.2, b"\n",
            large.1.as_bytes(), b"\n", large.2.len().to_string().as_bytes(), b"\n", large.2, b"\n",
            binary.1.as_bytes(), b"\n", binary.2.len().to_string().as_bytes(), b"\n", binary.2, b"\n",
            text.1.as_bytes(), b"\n", text.2.len().to_string().as_bytes(), b"\n", text.2, b"\n",
        ].concat().to_vec();

        assert_eq!(out, contents);
    }

    #[test]
    fn test_errors() {
        {
            let mut out = Vec::<u8>::new();
            let bad = compress_into(
                &mut out,
                // In current working dir at least
                &[Path::new("./nonexisting")],
                &DefaultHandler::new(Path::new("")),
            )
            .unwrap_err();
            match bad {
                ArchiveError::BadFileIo(path, io_err) if path == Path::new("./nonexisting") => {
                    assert!(io_err.kind() == io::ErrorKind::NotFound);
                }
                e => panic!("Unexpected error {:?}", e),
            }
        }
        {
            let mut out = Vec::<u8>::new();
            let bad = compress_into(
                &mut out,
                // In current working dir at least
                &[Path::new("\n\x01\x00")],
                &DefaultHandler::new(Path::new("")),
            )
            .unwrap_err();
            match bad {
                ArchiveError::InvalidDcaFilename(_, DcaFilenameError::InvalidChar('\n', 0)) => (),
                ArchiveError::BadFileIo(_, io_err)
                    if io_err.kind() == io::ErrorKind::InvalidInput => (),
                e => panic!("Unexpected error {:?}", e),
            }
        }
    }

    #[test]
    fn test_handler() {
        struct LaxHandler;
        impl Handler for LaxHandler {
            fn on_err(&self, err: ArchiveError) -> Result<(), ArchiveError> {
                match err {
                    ArchiveError::BadFileIo(path, io_err)
                        if path == Path::new("./nonexistent")
                            && io_err.kind() == io::ErrorKind::NotFound =>
                    {
                        Ok(())
                    }
                    e => panic!("Unexpected handleable error {:?}", e),
                }
            }
        }
        let file1 = make_outfile("file1".as_bytes());

        let mut out = Vec::<u8>::new();
        compress_into(
            &mut out,
            &[file1.0.path(), Path::new("./nonexistent")],
            &LaxHandler,
        )
        .unwrap();
    }
}

use humansize::{file_size_opts::CONVENTIONAL as FSIZE_STYLE, FileSize};
use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use crate::decompress::{
    decompress_from, CallbackFileHandler, DefaultErrorHandler, FileDescriptor,
};
use crate::error::{ArchiveError, FilePosition};

fn fmt_file_size(pos: FilePosition) -> impl std::fmt::Display {
    // Should never be able to fail (library fails for unsigned numbers)
    pos.file_size(FSIZE_STYLE).unwrap()
}

pub fn list_files(archive_name: impl AsRef<Path>) -> Result<(), ArchiveError> {
    let archive_name = archive_name.as_ref();

    let arch = File::open(archive_name).map_err(ArchiveError::ArchiveIo)?;
    let mut reader = BufReader::new(arch);

    let ehandler = DefaultErrorHandler::new(archive_name);
    let mut fhandler = CallbackFileHandler(|name, len, _reader| {
        println!("{} | {}", name, fmt_file_size(len));
        Ok(())
    });

    decompress_from(&mut reader, &mut fhandler, &ehandler)
}

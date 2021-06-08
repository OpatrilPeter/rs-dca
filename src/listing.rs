use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use dca::decompress::DefaultErrorHandler;
use dca::entries::{archive_entries, ListingSort};
use dca::error::{ArchiveError, FilePosition};

use humansize::{file_size_opts::CONVENTIONAL as FSIZE_STYLE, FileSize};

fn fmt_file_size(pos: FilePosition) -> impl std::fmt::Display {
    // Should never be able to fail (library fails for unsigned numbers)
    pos.file_size(FSIZE_STYLE).unwrap()
}

pub fn list_files(
    archive_name: impl AsRef<Path>,
    sorting: ListingSort,
) -> Result<(), ArchiveError> {
    let archive_name = archive_name.as_ref();

    let arch = File::open(archive_name).map_err(ArchiveError::ArchiveIo)?;
    let mut reader = BufReader::new(arch);
    let ehandler = DefaultErrorHandler::new(archive_name);

    for (name, size) in archive_entries(&mut reader, sorting, &ehandler)? {
        println!("{} ({})", name, fmt_file_size(size));
    }
    Ok(())
}

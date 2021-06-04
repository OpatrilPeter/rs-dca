use humansize::{file_size_opts::CONVENTIONAL as FSIZE_STYLE, FileSize};
use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use crate::decompress::{decompress_from, CallbackFileHandler, DefaultErrorHandler};
use crate::error::{ArchiveError, FilePosition};

fn fmt_file_size(pos: FilePosition) -> impl std::fmt::Display {
    // Should never be able to fail (library fails for unsigned numbers)
    pos.file_size(FSIZE_STYLE).unwrap()
}

#[derive(Debug)]
pub enum ListingSort {
    Unsorted,
    Name,
    Size,
}
impl Default for ListingSort {
    fn default() -> Self {
        ListingSort::Unsorted
    }
}

pub fn list_files(
    archive_name: impl AsRef<Path>,
    sorting: ListingSort,
) -> Result<(), ArchiveError> {
    let archive_name = archive_name.as_ref();

    let arch = File::open(archive_name).map_err(ArchiveError::ArchiveIo)?;
    let mut reader = BufReader::new(arch);

    let mut names = Vec::<(String, FilePosition)>::new();

    let ehandler = DefaultErrorHandler::new(archive_name);
    let mut fhandler = CallbackFileHandler(|name, len, _reader| {
        names.push((name.to_owned(), len));
        Ok(())
    });

    decompress_from(&mut reader, &mut fhandler, &ehandler)?;

    match sorting {
        ListingSort::Name => names.sort_unstable_by(|(a, _), (b, _)| a.cmp(b)),
        ListingSort::Size => names.sort_unstable_by(|(_, a), (_, b)| a.cmp(b).reverse()),
        ListingSort::Unsorted => (),
    }

    for (name, size) in names {
        println!("{} ({})", name, fmt_file_size(size));
    }
    Ok(())
}

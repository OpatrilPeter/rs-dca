//! Implements archive listing CLI feature

use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use dca::decompress::DefaultErrorHandler;
use dca::entries::archive_entries;
use dca::error::{ArchiveError, FilePosition, Result};

use humansize::{file_size_opts::CONVENTIONAL as FSIZE_STYLE, FileSize};

/// Defines ordering of archive's entries
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

fn fmt_file_size(pos: FilePosition) -> impl std::fmt::Display {
    // Should never be able to fail (library fails for negative numbers)
    pos.file_size(FSIZE_STYLE).unwrap()
}

/// Applies the ListingSort sorting
///
/// Note that names of entries can be nonunique - as this was deemed a pathological case, sorting order of these
/// entries was left undefined for efficiency
fn sort<Stringlike: Eq + Ord>(names: &mut Vec<(Stringlike, FilePosition)>, sorting: ListingSort) {
    match sorting {
        ListingSort::Name => names.sort_unstable_by(|a, b| a.0.cmp(&b.0)),
        ListingSort::Size => names.sort_unstable_by(|a, b| a.1.cmp(&b.1).reverse()),
        ListingSort::Unsorted => (),
    }
}

/// Extracts, sorts and prints archive's contents to standard output
///
/// Note that names of entries can be nonunique - as this was deemed a pathological case, sorting order of these
/// entries was left undefined for efficiency
///
/// # Example
/// ```no_run
/// let _ = list_files("archive.dca", ListingSort::Size);
/// ```
/// Outputs the following (format may change)
/// ```text
/// file3 (130 B)
/// file1 (50 B)
/// file2 (5 B)
/// ```
pub fn list_files(archive_name: impl AsRef<Path>, sorting: ListingSort) -> Result<()> {
    let archive_name = archive_name.as_ref();

    let arch = File::open(archive_name).map_err(ArchiveError::ArchiveIo)?;
    let mut reader = BufReader::new(arch);
    let ehandler = DefaultErrorHandler::new(archive_name);

    let mut names = archive_entries(&mut reader, &ehandler)?;
    sort(&mut names, sorting);

    for (name, size) in names {
        println!("{} ({})", name, fmt_file_size(size));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sort() {
        let data: Vec<(&'static str, FilePosition)> =
            vec![("world", 5), ("hello", 3), ("empty", 0)];

        let mut a = data.clone();
        sort(&mut a, ListingSort::Unsorted);
        assert_eq!(a, data);

        let mut a = data.clone();
        sort(&mut a, ListingSort::Name);
        assert_eq!(a, vec![("empty", 0), ("hello", 3), ("world", 5)]);

        let mut a = data.clone();
        sort(&mut a, ListingSort::Size);
        assert_eq!(a, vec![("world", 5), ("hello", 3), ("empty", 0)]);
    }
}

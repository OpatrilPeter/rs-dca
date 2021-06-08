//! Handles extraction of metadata from

use std::io::{BufRead, Seek};

use crate::decompress::{decompress_from, CallbackFileHandler};
use crate::error::{ArchiveError, FilePosition, Handler as ErrorHandler};

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

pub fn archive_entries(
    reader: &mut (impl BufRead + Seek),
    sorting: ListingSort,
    error_handler: &impl ErrorHandler,
) -> Result<Vec<(String, FilePosition)>, ArchiveError> {
    let mut names = Vec::new();

    let mut fhandler = CallbackFileHandler(|name, len, _reader| {
        names.push((name.to_owned(), len));
        Ok(())
    });
    decompress_from(reader, &mut fhandler, error_handler)?;

    match sorting {
        ListingSort::Name => names.sort_unstable_by(|a, b| a.0.cmp(&b.0)),
        ListingSort::Size => names.sort_unstable_by(|a, b| a.1.cmp(&b.1).reverse()),
        ListingSort::Unsorted => (),
    }

    Ok(names)
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::borrow::Borrow;
    use std::fmt::Debug;
    use std::io::Cursor;
    use std::path::Path;

    use crate::decompress::DefaultErrorHandler;

    fn std_errors() -> DefaultErrorHandler<'static> {
        DefaultErrorHandler::new(Path::new("archive.dca"))
    }

    fn assert_eq_iters<Item1, Item2>(
        it1: impl Iterator<Item = Item1>,
        it2: impl Iterator<Item = Item2>,
    ) where
        Item1: PartialEq<Item2>,
        Item1: Debug,
        Item2: Debug,
    {
        for (a, b) in it1.zip(it2) {
            assert_eq!(a, b);
        }
    }

    #[test]
    fn test_basic() {
        let mut arch = Cursor::new("DCA\nhello\n3\n123\nworld\n5\n12345\nempty\n0\n\n");
        let names = archive_entries(&mut arch, ListingSort::Unsorted, &std_errors()).unwrap();
        assert_eq_iters(
            names.iter().map(|(a, b)| (a.borrow(), *b)),
            vec![("hello", 3), ("world", 5), ("empty", 0)].into_iter(),
        );

        arch.set_position(0);
        let names = archive_entries(&mut arch, ListingSort::Name, &std_errors()).unwrap();
        assert_eq_iters(
            names.iter().map(|(a, b)| (a.borrow(), *b)),
            vec![("empty", 0), ("hello", 3), ("world", 5)].into_iter(),
        );

        arch.set_position(0);
        let names = archive_entries(&mut arch, ListingSort::Size, &std_errors()).unwrap();
        assert_eq_iters(
            names.iter().map(|(a, b)| (a.borrow(), *b)),
            vec![("world", 5), ("hello", 3), ("empty", 0)].into_iter(),
        );
    }
}

//! Handles extraction of metadata from DCA archives

use std::io::{BufRead, Seek};

use crate::decompress::{decompress_from, CallbackFileHandler};
use crate::error::{FilePosition, Handler as ErrorHandler, Result};

/// Extracts list of archive's entries as name-size pairs
///
/// See also CLI's method `list_files` for high-level usage.
pub fn archive_entries(
    reader: &mut (impl BufRead + Seek),
    error_handler: &impl ErrorHandler,
) -> Result<Vec<(String, FilePosition)>> {
    let mut names = Vec::new();

    let mut fhandler = CallbackFileHandler(|name, len, _reader| {
        names.push((name.to_owned(), len));
        Ok(())
    });
    decompress_from(reader, &mut fhandler, error_handler)?;

    Ok(names)
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::borrow::Borrow;
    use std::io::Cursor;
    use std::path::Path;

    use crate::decompress::DefaultErrorHandler;
    use crate::testutils::*;

    fn std_errors() -> DefaultErrorHandler<'static> {
        DefaultErrorHandler::new(Path::new("archive.dca"))
    }

    #[test]
    fn test_basic() {
        let mut arch = Cursor::new("DCA\nhello\n3\n123\nworld\n5\n12345\nempty\n0\n\n");
        let names = archive_entries(&mut arch, &std_errors()).unwrap();
        assert_eq_iters(
            names.iter().map(|(a, b)| (a.borrow(), *b)),
            vec![("hello", 3), ("world", 5), ("empty", 0)].into_iter(),
        );
    }
}

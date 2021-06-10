//! Common helpers for unit tests

use assert_fs::TempDir;
use std::fmt::Debug;
use std::fs::read_dir;
use std::iter::ExactSizeIterator;
use std::path::Path;

pub fn assert_eq_iters<Item1, Item2>(
    it1: impl Iterator<Item = Item1> + ExactSizeIterator,
    it2: impl Iterator<Item = Item2> + ExactSizeIterator,
) where
    Item1: PartialEq<Item2>,
    Item1: Debug,
    Item2: Debug,
{
    assert_eq!(it1.len(), it2.len());
    for (a, b) in it1.zip(it2) {
        assert_eq!(a, b);
    }
}

pub fn dir_size(dir: &impl AsRef<Path>) -> usize {
    read_dir(dir.as_ref()).unwrap().into_iter().count()
}

pub fn make_dir() -> TempDir {
    TempDir::new().unwrap()
}

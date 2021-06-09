//! Library for working with Dumb Cat Archive file format.
//!
//! DCA is trivial format for concatenating multiple files into one,
//! with key feature being simplicity of implementation in any
//! programming language and environment.
//!
//! Rust implementation provides additional robustness and performance
//! benefits over naive quick solution.
//!
//! Command-line frontend is also available, as well as original python
//! implementation for comparison.
//!
//! For additional information about the format and rationale, check
//! project's README file.

pub mod compress;
pub mod decompress;
pub mod entries;
pub mod error;

#[cfg(test)]
mod testutils;

pub use compress::compress_files;
pub use decompress::decompress_files;
pub use entries::archive_entries;

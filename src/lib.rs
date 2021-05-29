//! Dumb Cat Archive format compresser/decompresser
//!
//! Binary schema is super simple:
//! Grammar:
//! archive: header '\n' file*
//! header: 'DCA\n'
//! file: filename '\n' filesize '\n' payload '\n'
//! filename: <utf8 encoded filename, must not contain / and \n>
//! filesize: <decimal utf8 payload size in bytes>
//! payload: <sequence of `filesize` bytes, original file content>

pub mod compress;
pub mod decompress;
pub mod error;

pub use compress::compress_files;
pub use decompress::decompress_files;

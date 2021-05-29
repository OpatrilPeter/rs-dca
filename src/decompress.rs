use std::cmp::min;
use std::convert::TryInto;
use std::fs::{self, File};
use std::io::{self, prelude::*, BufRead, Seek};
use std::iter::FromIterator;
use std::path::{Path, PathBuf};

use crate::error::{error, ArchiveError, DecompressionError, Handler};

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
                    "Failed read from archive {:?} due to following error: {}",
                    self.archive_name, io_err
                );
            }
            CorruptedArchive { position, section } => {
                error!(
                    "Unexpected end of archive at position {} while processing section {:?}",
                    position, section
                );
            }
            _ => {
                error!(
                    "Extraction of archive {:?} failed due to error {:?}",
                    self.archive_name, err
                );
            }
        }
    }
}

impl<'a> Handler for DefaultHandler<'a> {
    fn on_err(&self, err: ArchiveError) -> Result<(), ArchiveError> {
        use ArchiveError::*;
        match err {
            BadFileIo(fname, io_err) => {
                error!(
                    "Extraction of file {:?} failed due to following error {}, skipping.",
                    fname, io_err
                );
                Ok(())
            }
            // Other problems are fatal
            err => Err(err),
        }
    }
}

fn read_matches<const N: usize>(
    reader: &mut impl Read,
    reference: &'static [u8; N],
    position: &mut usize,
) -> Result<bool, io::Error> {
    let mut buf = [0u8; N];
    reader.read_exact(&mut buf)?;

    let res = &buf == reference;
    if res {
        *position += N;
    }
    Ok(res)
}

fn read_line<T>(
    reader: &mut impl BufRead,
    line_buf: &mut String,
    position: &mut usize,
    processor: impl FnOnce(&str) -> Result<T, ArchiveError>,
) -> Result<Option<T>, ArchiveError> {
    line_buf.truncate(0);
    reader
        .read_line(line_buf)
        .map_err(ArchiveError::ArchiveIo)?;
    if line_buf.is_empty() {
        return Ok(None);
    }
    let res = processor(&line_buf[..line_buf.len() - 1]);
    if res.is_ok() {
        *position += line_buf.len();
    }
    res.map(Some)
}

fn read_file_size(
    reader: &mut impl BufRead,
    line_buf: &mut String,
    position: &mut usize,
) -> Result<usize, ArchiveError> {
    let old_pos = *position;
    let handler = |s: &str| {
        s.parse::<usize>()
            .map_err(|_| ArchiveError::CorruptedArchive {
                position: old_pos,
                section: DecompressionError::FileSize,
            })
    };
    read_line(reader, line_buf, position, handler).and_then(|val| match val {
        Some(size) => Ok(size),
        None => Err(ArchiveError::CorruptedArchive {
            position: *position,
            section: DecompressionError::FileSize,
        }),
    })
}

fn extract_file(
    reader: &mut impl BufRead,
    count: usize,
    sink: &mut impl Write,
    sink_name: &Path,
    position: &mut usize,
) -> Result<(), ArchiveError> {
    let mut remaining_size = count;
    loop {
        let buf = reader.fill_buf().map_err(ArchiveError::ArchiveIo)?;
        let read_upto = min(remaining_size, buf.len());
        if read_upto == 0 {
            if remaining_size > 0 {
                return Err(ArchiveError::CorruptedArchive {
                    position: *position,
                    section: DecompressionError::Payload,
                });
            }
            break;
        }
        sink.write_all(&buf[..read_upto])
            .map_err(|e| ArchiveError::BadFileIo(sink_name.to_owned(), e))?;
        reader.consume(read_upto);
        *position += read_upto;
        remaining_size -= read_upto;
    }
    Ok(())
}

fn skip_file(
    reader: &mut (impl BufRead + Seek),
    count: usize,
    position: &mut usize,
) -> Result<(), ArchiveError> {
    // Size bigger than positive signed offset!
    let offset = count
        .try_into()
        .map_err(|_| ArchiveError::CorruptedArchive {
            position: *position,
            section: DecompressionError::FileSize,
        })?;
    reader
        .seek(io::SeekFrom::Current(offset))
        .map_err(ArchiveError::ArchiveIo)?;
    *position += count;
    // Footer
    match read_matches(reader, b"\n", position) {
        Ok(footer_matches) => {
            if !footer_matches {
                return Err(ArchiveError::CorruptedArchive {
                    position: *position,
                    section: DecompressionError::Footer,
                });
            }
        }
        Err(e) => {
            return Err(ArchiveError::ArchiveIo(e));
        }
    }
    Ok(())
}

pub fn decompress_from(
    reader: &mut (impl BufRead + Seek),
    work_directory: &Path,
    handle: &impl Handler,
) -> Result<(), ArchiveError> {
    use ArchiveError as E;

    let mut position = 0usize;

    // Header
    match read_matches(reader, b"DCA\n", &mut position) {
        Ok(header_matches) => {
            if !header_matches {
                return Err(E::CorruptedArchive {
                    position,
                    section: DecompressionError::Header,
                });
            }
        }
        Err(e) => {
            return Err(E::ArchiveIo(e));
        }
    }

    let mut line_buf = String::new();
    loop {
        let fname: String =
            match read_line(reader, &mut line_buf, &mut position, |s| Ok(s.to_owned()))? {
                None => {
                    // Final file
                    break;
                }
                Some(fname) => fname,
            };

        let fsize = read_file_size(reader, &mut line_buf, &mut position)?;

        let fname_buf = PathBuf::from_iter(&[work_directory, Path::new(&fname)]);
        let file = match File::create(&fname_buf).map_err(|e| E::BadFileIo(fname_buf.clone(), e)) {
            Ok(file) => file,
            Err(err) => {
                handle.on_err(err)?;
                // Handler decided that we can skip creation of this file, skip forward to the end
                skip_file(reader, fsize, &mut position)?;
                continue;
            }
        };
        enum FileExportResult {
            Done,
            Remove(ArchiveError),
        }
        use FileExportResult::*;
        let write_single_file = || -> FileExportResult {
            let mut writer = io::BufWriter::new(file);

            match extract_file(reader, fsize, &mut writer, &fname_buf, &mut position) {
                Ok(()) => Done,
                Err(e) => Remove(e),
            }
        };
        if let Remove(err) = write_single_file() {
            if let Err(del_err) = fs::remove_file(&fname_buf) {
                error!("Extraction of {:?} failed but the temporary file couldn't be deleted due to error {}. Please remove it manually.", fname_buf, del_err);
            }
            return Err(err);
        }
        // Footer
        match read_matches(reader, b"\n", &mut position) {
            Ok(footer_matches) => {
                if !footer_matches {
                    return Err(E::CorruptedArchive {
                        position,
                        section: DecompressionError::Footer,
                    });
                }
            }
            Err(e) => {
                return Err(E::ArchiveIo(e));
            }
        }
    }

    Ok(())
}

pub fn decompress_files(archive_name: &Path, work_directory: &Path) -> Result<(), ArchiveError> {
    let handler = DefaultHandler::new(archive_name);

    let arch = File::open(archive_name).map_err(|e| {
        let e = ArchiveError::ArchiveIo(e);
        handler.on_fatal(&e);
        e
    })?;
    let mut reader = io::BufReader::new(arch);

    decompress_from(&mut reader, work_directory, &handler).map_err(|e| {
        handler.on_fatal(&e);
        e
    })
}

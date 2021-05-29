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

#[allow(unused_imports)]
use log::{debug, warn, error};
use std::cmp::min;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::io::{self, prelude::*};
use std::iter::FromIterator;
use std::fs::File;
use std::process::exit;

fn parse_args() -> clap::ArgMatches<'static> {
    use clap::*;

    App::new("Dumb cat archive compressor/decompressor")
        .arg(
            Arg::from_usage("-c --compress")
        )
        .arg(
            Arg::from_usage("-d --decompress")
        )
        .arg(
            Arg::from_usage("<files>...")
                .help("If decompressing, should be ONLY name of the archive. If compressing, should be list of files.")
        )
        .arg(
            Arg::from_usage("-o --output")
                .takes_value(true)
                .help("Name of archive while compressing OR output directory while decompressing.")
        )
        .group(
            ArgGroup::with_name("modes")
                .multiple(false)
                .args(&["compress", "decompress"])
        )
        .get_matches()
}

#[derive(Debug)]
enum Mode {
    Compress,
    Decompress,
}

#[derive(Default, Debug)]
struct Options {
    mode: Option<Mode>,
    work_directory: Option<PathBuf>,
    archive_name: Option<PathBuf>,
    files: Vec<PathBuf>,
}

fn select_mode(args: &clap::ArgMatches<'_>) -> Options {
    let mut opts = Options::default();

    let output: Option<PathBuf> = args.value_of_os("output").map(|x| x.into());
    opts.files = args.values_of_os("files").unwrap_or_default().map(|x| x.into()).collect();

    if args.is_present("compress") {
        opts.mode = Some(Mode::Compress);
    }
    else if args.is_present("decompress") {
        opts.mode = Some(Mode::Decompress);
    }
    // Auto detection
    else {
        #[allow(clippy::collapsible_else_if)]
        if opts.files.len() == 1 && opts.files[0].extension().map(|ext| ext == OsStr::new("dca")).unwrap_or(false) {
            opts.mode = Some(Mode::Decompress);
        }
        else {
            opts.mode = Some(Mode::Compress);
        }
    }

    match opts.mode {
        Some(Mode::Compress) => {
            opts.archive_name = output;
            match opts.archive_name {
                None => {
                    opts.archive_name = Some({
                        if opts.files.len() == 1 {
                            let mut buf = OsString::new();
                            if let Some(file_name) = opts.files[0].file_name() {
                                buf.push(file_name);
                            }
                            buf.push(".dca");
                            PathBuf::from(buf)
                        }
                        else {
                            PathBuf::from("dca.dca")
                        }
                    });
                },
                Some(ref mut name) => {
                    if name.extension().is_none() {
                        name.set_extension("dca");
                    }
                }
            }
        },
        Some(Mode::Decompress) => {
            if opts.files.len() != 1 {
                opts.mode = None;
                return opts;
            }

            opts.work_directory = output.or_else(||Some(PathBuf::from(".")));
            opts.archive_name = std::mem::take(&mut opts.files).into_iter().next();
        },
        None => ()
    }
    opts
}

fn ok_fname(name: &OsStr) -> Option<&str> {
    let utf_name = name.to_str();
    if let Some(name) = utf_name {
        if name.contains('\n') {
            return None;
        }
    }
    utf_name
}

fn compress_files<P>(files: impl IntoIterator<Item=P>, archive_name: &Path) -> Result<(), io::Error>
    where P: AsRef<Path>,
{
    let arch = File::create(archive_name)?; // TODO: Remove file on error
    let mut writer = io::BufWriter::new(arch);

    writer.write_all(b"DCA\n")?;
    for file in files {
        let file = file.as_ref().canonicalize()?;

        let fname: &str = match file.file_name() {
            None => {
                return Err(io::Error::new(io::ErrorKind::InvalidInput, format!("argument {:?} to be compressed is not a filename", file)));
            },
            Some(fname) => {
                match ok_fname(fname) {
                    Some(fname) => fname,
                    None => {
                        return Err(io::Error::new(io::ErrorKind::InvalidInput, format!("argument {:?} to be compressed is not accepted name", file)));
                    }
                }
            }
        };

        let subfile = File::open(&file)?;
        let mut reader = io::BufReader::new(subfile);
        let subfile_len = reader.seek(io::SeekFrom::End(0))?;
        reader.seek(io::SeekFrom::Start(0))?;

        writer.write_fmt(format_args!("{}\n{}\n", fname, subfile_len))?;

        loop {
            let buf = reader.fill_buf()?;
            if buf.is_empty() {
                break;
            }
            let bytes = writer.write(buf)?;
            reader.consume(bytes);
        }
        writer.write_all(b"\n")?;
    }

    Ok(())
}

fn decompress_files(archive_name: &Path, work_directory: &Path) -> Result<(), io::Error> {
    let arch = File::open(archive_name)?;
    let mut reader = io::BufReader::new(arch);

    // Header
    {
        let mut a = [0u8;4];
        reader.read_exact(&mut a)?;
        if &a != b"DCA\n" {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "invalid archive header"));
        }
    }

    let mut string_buf = String::new();
    loop {
        string_buf.truncate(0);
        reader.read_line(&mut string_buf)?;
        if string_buf.is_empty() {
            break;
        }
        let fname = string_buf.clone();
        string_buf.truncate(0);
        reader.read_line(&mut string_buf)?;
        let fsize: usize = match string_buf[..string_buf.len()-1].parse() {
            Ok(size) => size,
            Err(_) => {
                return Err(io::Error::new(io::ErrorKind::InvalidData, format!("invalid format of size of contained file ({})", string_buf)));
            }
        };

        let fname_buf = PathBuf::from_iter(&[work_directory, Path::new(&fname)]);
        let file = File::create(fname_buf)?; // TODO: Remove files on error
        let mut writer = io::BufWriter::new(file);

        let mut remaining_size = fsize;
        loop {
            let buf = reader.fill_buf()?;
            let read_upto = min(remaining_size, buf.len());
            if read_upto == 0 {
                if remaining_size > 0 {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "premature end of contained file"
                    ))
                }
                break;
            }
            writer.write_all(&buf[..read_upto])?;
            reader.consume(read_upto);
            remaining_size -= read_upto;
        }
        // Footer
        {
            let mut a = [0u8; 1];
            reader.read_exact(&mut a)?;
            if a[0] != b'\n' {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "invalid archive footer"));
            }
        }
    }

    Ok(())
}

fn main() {
    env_logger::init();
    let args = parse_args();

    let opts = select_mode(&args);
    debug!("Collected options: {:?}", opts);
    match opts {
        Options{mode: Some(Mode::Compress), files, archive_name: Some(archive_name), ..}
            => {
                if let Err(e) = compress_files(&files, &archive_name) {
                    eprintln!("Compression failed with following error: {}\nArchive filename: {:?}\nArchive contents: {:?}", e, archive_name, files);
                    exit(1);
                }
            }
        Options{mode: Some(Mode::Decompress), archive_name: Some(archive_name), work_directory: Some(work_directory), ..}
            => {
                if let Err(e) = decompress_files(&archive_name, &work_directory) {
                    eprintln!("Decompression of archive {:?} failed with following error: {}", archive_name, e);
                    exit(1);
                }
            }
        Options{mode: None, ..} => {
            eprintln!("No valid mode selected, please select compression/decompression.\n{}", args.usage());
            exit(1);
        }
        opts => panic!("Unexpected argument combination {:?}.", opts),
    }
}

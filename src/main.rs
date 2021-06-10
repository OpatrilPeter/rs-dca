#![cfg(feature = "cli")]

#[allow(unused_imports)]
use log::{debug, error, warn};
use std::error::Error;
use std::ffi::{OsStr, OsString};
use std::fmt::{self, Display};
use std::path::PathBuf;
use std::process::exit;

use dca::*;

// CLI specific logic
mod listing;
use listing::{list_files, ListingSort};

/// Helper for printing error chains
struct ErrChain<'a>(&'a dyn Error);
impl Display for ErrChain<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        Display::fmt(self.0, f)?;
        let mut s = self.0.source();
        loop {
            match s {
                None => break,
                Some(e) => {
                    write!(f, ": {}", e)?;
                    s = e.source();
                }
            }
        }
        Ok(())
    }
}

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
            Arg::from_usage("-l --list")
                .help("Lists archive's contents.")
        )
        .arg(
            Arg::from_usage("--sort-by-name")
                .help("Sort listing by name")
                .requires("list")

        )
        .arg(
            Arg::from_usage("--sort-by-size")
                .help("Sort listing by file size")
                .requires("list")
        )
        .arg(
            Arg::from_usage("<files>...")
                .help("If decompressing or listing, should be ONLY name of the archive. If compressing, should be list of files.")
        )
        .arg(
            Arg::from_usage("-o --output")
                .takes_value(true)
                .help("Name of archive while compressing OR output directory while decompressing.")
        )
        .group(
            ArgGroup::with_name("modes")
                .multiple(false)
                .args(&["compress", "decompress", "list"])
        )
        .group(
            ArgGroup::with_name("sorting")
                .multiple(false)
                .args(&["sort-by-name", "sort-by-size"])
        )
        .get_matches()
}

#[derive(Debug)]
enum Mode {
    Compress,
    Decompress,
    Listing,
}

#[derive(Default, Debug)]
struct Options {
    mode: Option<Mode>,
    work_directory: Option<PathBuf>,
    archive_name: Option<PathBuf>,
    files: Vec<PathBuf>,
    sorting: ListingSort,
}

/// Deduces mode of operation and validates correct arguments for it
fn select_mode(args: &clap::ArgMatches<'_>) -> Options {
    let mut opts = Options::default();

    let output: Option<PathBuf> = args.value_of_os("output").map(|x| x.into());
    opts.files = args
        .values_of_os("files")
        .unwrap_or_default()
        .map(|x| x.into())
        .collect();

    #[allow(clippy::collapsible_else_if)]
    if args.is_present("compress") {
        opts.mode = Some(Mode::Compress);
    } else if args.is_present("decompress") {
        opts.mode = Some(Mode::Decompress);
    } else if args.is_present("list") {
        opts.mode = Some(Mode::Listing);
    }
    // Auto detection
    else {
        if opts.files.len() == 1
            && opts.files[0]
                .extension()
                .map(|ext| ext == OsStr::new("dca"))
                .unwrap_or(false)
        {
            opts.mode = Some(Mode::Decompress);
        } else {
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
                        } else {
                            PathBuf::from("dca.dca")
                        }
                    });
                }
                Some(ref mut name) => {
                    if name.extension().is_none() {
                        name.set_extension("dca");
                    }
                }
            }
        }
        Some(Mode::Decompress) => {
            if opts.files.len() != 1 {
                opts.mode = None;
                return opts;
            }

            opts.work_directory = output.or_else(|| Some(PathBuf::from(".")));
            opts.archive_name = std::mem::take(&mut opts.files).into_iter().next();
        }
        Some(Mode::Listing) => {
            if opts.files.len() != 1 || output.is_some() {
                opts.mode = None;
                return opts;
            }

            if args.is_present("sort-by-name") {
                opts.sorting = ListingSort::Name;
            } else if args.is_present("sort-by-size") {
                opts.sorting = ListingSort::Size;
            }
            opts.archive_name = std::mem::take(&mut opts.files).into_iter().next();
        }
        None => (),
    }
    opts
}

fn main() {
    env_logger::init();
    let args = parse_args();

    let opts = select_mode(&args);
    debug!("Collected options: {:?}", opts);
    match opts {
        Options {
            mode: Some(Mode::Compress),
            files,
            archive_name: Some(archive_name),
            ..
        } => {
            if let Err(err) = compress_files(&files, &archive_name) {
                eprintln!(
                    "Compression failed.\nArchive filename: {:?}\nArchive contents: {:?}\nProblem: {}",
                    archive_name, files, ErrChain(&err)
                );
                exit(1);
            }
        }
        Options {
            mode: Some(Mode::Decompress),
            archive_name: Some(archive_name),
            work_directory: Some(work_directory),
            ..
        } => {
            if let Err(err) = decompress_files(&archive_name, &work_directory) {
                eprintln!(
                    "Decompression of archive {:?} failed: {}",
                    archive_name,
                    ErrChain(&err)
                );
                exit(1);
            }
        }
        Options {
            mode: Some(Mode::Listing),
            archive_name: Some(archive_name),
            ..
        } => {
            if let Err(err) = list_files(&archive_name, opts.sorting) {
                eprintln!(
                    "Listing of archive {:?} failed: {}",
                    archive_name,
                    ErrChain(&err)
                );
                exit(1);
            }
        }
        Options { mode: None, .. } => {
            eprintln!(
                "No valid mode selected, please select compression/decompression.\n{}",
                args.usage()
            );
            exit(1);
        }
        opts => {
            eprintln!(
                "Unexpected argument combination {:?}.\n{}",
                opts,
                args.usage()
            );
            exit(1);
        }
    }
}

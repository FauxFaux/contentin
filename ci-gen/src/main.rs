use anyhow::{Context, Result};
use ci_capnp::Meta;
use clap::{App, Arg};
use libflate::gzip;

mod errors;
mod filetype;
mod output_capnp;
mod simple_time;
mod slist;
mod stat;
mod tee;
mod unpacker;

pub struct Options {
    content_output: bool,
    max_depth: u32,
    verbose: u8,
}

enum ArchiveReadFailure {
    Open(String),
    Read(String),
}

pub struct EntryBuilder {
    path: slist::SList<String>,
    failure: Option<ArchiveReadFailure>,
    depth: u32,
    meta: Meta,
}

fn must_fit(x: u64) -> u8 {
    if x > std::u8::MAX as u64 {
        panic!("too many something: {}", x);
    }
    x as u8
}

fn real_main() -> Result<i32> {
    let matches = App::new("contentin")
        .arg(
            Arg::with_name("verbose")
                .short('v')
                .multiple_occurrences(true)
                .help("Sets the level of verbosity (more for more)"),
        )
        .arg(
            Arg::with_name("quiet")
                .short('q')
                .multiple_occurrences(true)
                .takes_value(false)
                .help("Reduce the level of verbosity"),
        )
        .arg(
            Arg::with_name("list")
                .short('t')
                .long("list")
                .help("Show headers only, not object content"),
        )
        .arg(
            Arg::with_name("max-depth")
                .short('d')
                .long("max-depth")
                .takes_value(true)
                .use_delimiter(false)
                .default_value("256")
                .hide_default_value(true)
                .validator(|val| match val.parse::<u32>() {
                    Ok(_) => Ok(()),
                    Err(e) => Err(format!("must be valid number: {}", e)),
                })
                .help("Limit recursion. 1: like unzip. Default: lots"),
        )
        .arg(
            Arg::with_name("INPUT")
                .required(true)
                .help("File(s) to process")
                .multiple(true),
        )
        .get_matches();

    let content_output = !matches.is_present("list");

    let options = Options {
        content_output,
        max_depth: matches.value_of("max-depth").unwrap().parse().unwrap(),
        verbose: must_fit(1 + matches.occurrences_of("verbose") - matches.occurrences_of("quiet")),
    };

    for path in matches.values_of("INPUT").unwrap() {
        unpacker::process_real_path(path, &options)
            .with_context(|| format!("processing: '{}'", path))?;
    }

    Ok(0)
}

fn main() -> Result<()> {
    std::process::exit(real_main()?)
}

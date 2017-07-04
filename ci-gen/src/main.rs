extern crate ar;
extern crate bootsector;
extern crate bzip2;
extern crate capnp;
extern crate ci_capnp;
extern crate clap;
#[macro_use]
extern crate error_chain;
extern crate ext4;
extern crate libflate;
extern crate tar;
extern crate tempfile;
extern crate time as crates_time;
extern crate users;
extern crate xz2;
extern crate zip;

use clap::{Arg, App};
use ci_capnp::Meta;

use libflate::gzip;

mod errors;
mod filetype;
mod output_capnp;
mod simple_time;
mod slist;
mod stat;
mod tee;
mod unpacker;

use errors::*;

enum ListingOutput {
    None,
    Capnp,
    Find,
}

pub enum ContentOutput {
    None,
    Raw,
}

pub struct Options {
    listing_output: ListingOutput,
    content_output: ContentOutput,
    max_depth: u32,
    verbose: u8,
}

enum ArchiveReadFailure {
    Open(String),
    Read(String),
}

pub struct FileDetails {
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
        .arg(Arg::with_name("v").short("v").multiple(true).help(
            "Sets the level of verbosity (more for more)",
        ))
        .arg(
            Arg::with_name("headers")
                .short("h")
                .long("headers")
                .possible_values(&["none", "find", "capnp"])
                .default_value("find")
                .help("What format to write file metadata in"),
        )
        .arg(
            Arg::with_name("list")
                .short("t")
                .long("list")
                .conflicts_with("to-command")
                .help("Show headers only, not object content"),
        )
        .arg(
            Arg::with_name("no-listing")
                .short("n")
                .long("no-listing")
                .conflicts_with("list")
                .help("don't print the listing at all"),
        )
        .arg(
            Arg::with_name("grep")
                .short("S")
                .long("grep")
                .takes_value(true)
                .help("search for a string in all files"),
        )
        .arg(
            Arg::with_name("max-depth")
                .short("d")
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

    let mut listing_output = ListingOutput::Find;
    let content_output = if matches.is_present("list") {
        ContentOutput::None
    } else {
        ContentOutput::Raw
    };

    if matches.is_present("no-listing") {
        listing_output = ListingOutput::None;
    }

    if let Some(listing_format) = matches.value_of("headers") {
        listing_output = match listing_format {
            "none" => ListingOutput::None,
            "capnp" => ListingOutput::Capnp,
            "find" => ListingOutput::Find,
            _ => unreachable!(),
        }
    }

    let options = Options {
        listing_output,
        content_output,
        max_depth: matches.value_of("max-depth").unwrap().parse().unwrap(),
        verbose: must_fit(matches.occurrences_of("v")),
    };

    for path in matches.values_of("INPUT").unwrap() {
        unpacker::process_real_path(path, &options).chain_err(|| {
            format!("processing: '{}'", path)
        })?;
    }

    Ok(0)
}

quick_main!(real_main);

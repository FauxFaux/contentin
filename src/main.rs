extern crate ar;
extern crate bzip2;
extern crate clap;
extern crate libflate;
extern crate regex;
extern crate tar;
extern crate tempfile;
extern crate time as crates_time;
extern crate users;
extern crate xz2;
extern crate zip;

use std::error;
use std::fmt;
use std::fs;
use std::io;
use std::path;
use std::process;
use std::time;

use clap::{Arg, App};

use libflate::gzip;
use regex::Regex;
use tempfile::tempfile;

mod filetype;
use filetype::FileType;

mod slist;
use slist::SList;

mod tee;
use tee::*;

mod stat;
use stat::Stat;


// magic:
use std::io::BufRead;
use std::io::Write;

enum ListingOutput {
    None,
    Find,
}

enum ContentOutput {
    None,
    Raw,
    ToCommand(String),
    Grep(Regex),
}

struct Options {
    listing_output: ListingOutput,
    content_output: ContentOutput,
    max_depth: u32,
    verbose: u8,
}

struct FileDetails {
    path: SList<String>,
    depth: u32,
    atime: u64,
    mtime: u64,
    ctime: u64,
    btime: u64,
    uid: u32,
    gid: u32,
    user_name: String,
    group_name: String,
}

pub struct Unpacker<'a> {
    options: &'a Options,
    current: FileDetails,
}

impl<'a> Unpacker<'a> {
    fn log<T: fmt::Display, F>(&self, level: u8, msg: F) -> io::Result<()>
    where F: FnOnce() -> T
    {
        if self.options.verbose < level {
            return Ok(());
        }

        let name = match level {
            1 => "warn",
            2 => "info",
            3 => "debug",
            _ => unreachable!()
        };

        writeln!(io::stderr(), "{}: {}", name, msg()).map(|_|())
    }

    fn complete(&self, mut file: Box<Tee>) -> io::Result<()> {
        let size = file.len_and_reset()?;
        self.complete_details(file, size)
    }

    fn complete_details<R: io::Read>(&self, mut src: R, size: u64) -> io::Result<()> {
        let stdout = io::stdout();
        let mut stdout = stdout.lock();

        let current_path = || self.current.path.to_vec().join(" / ");

        match self.options.listing_output {
            ListingOutput::None => {},
            ListingOutput::Find => {
                writeln!(stdout, "{} {} {} {} {} {} {} {} {} {}",
                         current_path(),
                         size,
                         self.current.atime, self.current.mtime, self.current.ctime, self.current.btime,
                         self.current.uid, self.current.gid, self.current.user_name, self.current.group_name,
                )?;
            },
        }

        match self.options.content_output {
            ContentOutput::None => Ok(()),
            ContentOutput::Raw => {
                io::copy(&mut src, &mut stdout).and_then(move |written|
                    if written != size {
                        Err(io::Error::new(
                            io::ErrorKind::UnexpectedEof,
                            format!("expecting to write {} but wrote {}", size, written)))
                    } else {
                        Ok(())
                    })
            },
            ContentOutput::ToCommand(ref cmd) => {
                let mut child = process::Command::new("sh")
                    .args(&["-c", cmd])
                    .env("TAR_REALNAME", current_path())
                    .env("TAR_SIZE", format!("{}", size))
                    .stdin(process::Stdio::piped())
                    .stdout(process::Stdio::inherit())
                    .stderr(process::Stdio::inherit())
                    .spawn()?;

                assert_eq!(size, io::copy(&mut src, &mut child.stdin.as_mut().unwrap())?);

                assert!(child.wait()?.success());

                Ok(())
            },
            ContentOutput::Grep(ref expr) => {
                let current_path = current_path();
                let reader = io::BufReader::new(src);
                for (no, line) in reader.lines().enumerate() {
                    if line.is_err() {
                        self.log(1, || format!("non-utf-8 file ignored: {}", current_path))?;
                        break;
                    }
                    let line = line?;
                    if expr.is_match(line.as_str()) {
                        println!("{}:{}:{}", current_path, no+1, line);
                    }
                }
                Ok(())
            }
        }
    }

    fn from_file<'b>(path: &str, fd: &fs::File, options: &'b Options) -> io::Result<Unpacker<'b>> {
        let meta = fd.metadata()?;
        let stat = Stat::from(&meta);
        Ok(Unpacker {
            options,
            current: FileDetails {
                depth: 0,
                path: SList::head(path.to_string()),
                atime: meta.accessed().map(simple_time_sys)?,
                mtime: meta.modified().map(simple_time_sys)?,
                ctime: simple_time_ctime(&stat),
                btime: simple_time_btime(&meta)?,
                uid: stat.uid,
                gid: stat.gid,
                user_name: users::get_user_by_uid(stat.uid)
                    .map(|user| user.name().to_string())
                    .unwrap_or(String::new()),
                group_name: users::get_group_by_gid(stat.gid)
                    .map(|group| group.name().to_string())
                    .unwrap_or(String::new()),
            },
        })
    }

    fn with_path(&self, path: &str) -> Unpacker {
        Unpacker {
            options: self.options,
            current: FileDetails {
                path: self.current.path.plus(path.to_string()),
                depth: self.current.depth + 1,
                atime: 0,
                mtime: 0,
                ctime: 0,
                btime: 0,
                uid: 0,
                gid: 0,
                user_name: String::new(),
                group_name: String::new(),
            },
        }
    }

    fn strip_compression_suffix(&self, suffix: &str) -> &str {
        let our_name = self.current.path.inner().as_str();
        if our_name.ends_with(suffix) {
            &our_name[..our_name.len() - suffix.len()]
        } else {
            ""
        }
    }
}

fn simple_time(dur: time::Duration) -> u64 {
    dur.as_secs().checked_mul(1_000_000_000)
        .map_or(0, |nanos| nanos + dur.subsec_nanos() as u64)
}

fn simple_time_sys(val: time::SystemTime) -> u64 {
    val.duration_since(time::UNIX_EPOCH).map(simple_time).unwrap_or(0)
}


fn simple_time_tm(val: crates_time::Tm) -> u64 {
    let timespec = val.to_timespec();
    simple_time(time::Duration::new(timespec.sec as u64, timespec.nsec as u32))
}

fn simple_time_btime(val: &fs::Metadata) -> io::Result<u64> {
    match val.created() {
        Ok(time) => Ok(simple_time_sys(time)),
        // "Other" is how "unsupported" is represented here; ew.
        Err(ref e) if e.kind() == io::ErrorKind::Other => Ok(0),
        Err(other) => Err(other),
    }
}

fn simple_time_epoch_seconds(seconds: u64) -> u64 {
    seconds.checked_mul(1_000_000_000).unwrap_or(0)
}

fn simple_time_ctime(val: &stat::Stat) -> u64 {
    if val.ctime <= 0 {
        0
    } else {
        (val.ctime as u64).checked_mul(1_000_000_000).unwrap_or(0) + (val.ctime_nano as u64)
    }
}



#[derive(Clone, Copy, Debug)]
struct Rewind {
}

impl error::Error for Rewind {
    fn description(&self) -> &str {
        "rewind"
    }
}

impl fmt::Display for Rewind {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(f, "rewind")
    }
}

enum FormatErrorType {
    Rewind,
    Other,
}

fn is_format_error(e: &io::Error) -> Option<FormatErrorType> {
    // if there's an actual error code (regardless of what it is),
    // it's probably not from a library
    if e.raw_os_error().is_some() {
        return None;
    }

    match e.kind() {
        io::ErrorKind::Other => {
            if let Some(ref obj) = e.get_ref() {

                // our marker error for backtracking
                if obj.is::<Rewind>() {
                    return Some(FormatErrorType::Rewind);
                }


                if obj.is::<xz2::stream::Error>() {
                    return Some(FormatErrorType::Other);
                }
            }
        },
        io::ErrorKind::BrokenPipe
        | io::ErrorKind::NotFound
        | io::ErrorKind::PermissionDenied
        => return None,
        _ => {},
    }

    None
}

impl<'a> Unpacker<'a> {
    fn process_zip<T>(&self, from: T) -> io::Result<()>
        where T: io::Read + io::Seek {
        let mut zip = zip::ZipArchive::new(from)?;

        for i in 0..zip.len() {
            let res = {
                let entry = zip.by_index(i)?;
                let mut unpacker = self.with_path(entry.name());

                unpacker.current.mtime = simple_time_tm(entry.last_modified());
                let mut failing: Box<Tee> = Box::new(FailingTee::new(entry));
                unpacker.unpack_or_die(&mut failing)
            };

            if self.is_format_error_result(&res)? {
                let new_entry = zip.by_index(i)?;
                let size = new_entry.size();
                self.complete_details(new_entry, size)?;
                continue;
            }

            // scope based borrow sigh; same block as above
            if res.is_err() {
                return res;
            }
        }
        Ok(())
    }

    fn with_gzip(&self, header: &gzip::Header) -> io::Result<Unpacker> {
        let mtime = simple_time_epoch_seconds(header.modification_time() as u64);
        let name = match header.filename() {
            Some(ref c_str) => c_str.to_str().map_err(
                |not_utf8| io::Error::new(io::ErrorKind::InvalidData,
                                          format!("gzip member's name must be valid utf-8: {} {:?}",
                                                  not_utf8, c_str.as_bytes())))?,
            None => self.strip_compression_suffix(".gz"),
        };

        let mut unpacker = self.with_path(name);
        unpacker.current.mtime = mtime;
        Ok(unpacker)
    }

    fn unpack_or_die<'b>(&self, mut fd: &mut Box<Tee + 'b>) -> io::Result<()> {
        if self.current.depth >= self.options.max_depth {
            return Err(io::Error::new(io::ErrorKind::Other, Rewind {}));
        }

        let identity = FileType::identify(fd.fill_buf()?);
        self.log(2, || format!("identified '{}' as {}", self.current.path.inner(), identity))?;
        match identity {
            FileType::GZip => {
                let (attempt, unpacker) = {
                    let br = BoxReader { inner: fd };
                    let dec = gzip::Decoder::new(br)?;

                    let unpacker = self.with_gzip(dec.header())?;

                    let mut failing: Box<Tee> = Box::new(FailingTee::new(dec));
                    (unpacker.unpack_or_die(&mut failing), unpacker)
                };


                if self.is_format_error_result(&attempt)? {
                    fd.reset()?;
                    unpacker.complete(TempFileTee::if_necessary(gzip::Decoder::new(fd)?, &unpacker)?)?;
                    Ok(())
                } else {
                    attempt
                }
            },

            // xz and bzip2 have *nothing* in their header; no mtime, no name, no source OS, no nothing.
            FileType::Xz => {
                self.with_path(self.strip_compression_suffix(".xz"))
                    .unpack_stream_xz(fd)
            },
            FileType::BZip2 => {
                self.with_path(self.strip_compression_suffix(".bz2"))
                    .unpack_stream_bz2(fd)
            }

            FileType::Deb => {
                let mut decoder = ar::Archive::new(fd);
                while let Some(entry) = decoder.next_entry() {
                    let entry = entry?;
                    let unpacker = self.with_path(entry.header().identifier());
                    unpacker.unpack(TempFileTee::if_necessary(entry, &unpacker)?)?;
                }
                Ok(())
            },
            FileType::Tar => {
                let mut decoder = tar::Archive::new(fd);
                for entry in decoder.entries()? {
                    let entry = entry?;
                    let mut unpacker = self.with_path(entry.path()?.to_str()
                        .ok_or_else(|| io::Error::new(io::ErrorKind::Other,
                                                      format!("tar path contains invalid utf-8: {:?}",
                                                              entry.path_bytes())))?);
                    {
                        let current = &mut unpacker.current;
                        let header = entry.header();
                        current.uid = header.uid()?;
                        current.gid = header.gid()?;
                        current.mtime = simple_time_epoch_seconds(header.mtime()?);
                        if let Some(found) = header.username()
                            .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput,
                                                        format!("invalid username in tar: {} {:?}", e, header.username_bytes())))? {
                            current.user_name = found.to_string();
                        }
                        if let Some(found) = header.groupname()
                            .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput,
                                                        format!("invalid groupname in tar: {} {:?}", e, header.groupname_bytes())))? {
                            current.group_name = found.to_string();
                        }
                    }
                    unpacker.unpack(TempFileTee::if_necessary(entry, &unpacker)?)?;
                }
                Ok(())
            },
            FileType::Zip => {
                if let Some(seekable) = fd.as_seekable() {
                    return self.process_zip(seekable);
                }

                let mut temp = tempfile()?;
                io::copy(&mut fd, &mut temp)?;
                self.process_zip(temp)
            },
            FileType::Other => {
                Err(io::Error::new(io::ErrorKind::Other, Rewind {}))
            },
        }
    }

    // TODO: Work out how to generic these copy-pastes
    fn unpack_stream_xz<'c>(&self, fd: &mut Box<Tee + 'c>) -> io::Result<()> {
        let attempt = {
            let br = BoxReader { inner: fd };
            let mut failing: Box<Tee> = Box::new(FailingTee::new(xz2::bufread::XzDecoder::new(br)));
            self.unpack_or_die(&mut failing)
        };

        if self.is_format_error_result(&attempt)? {
            fd.reset()?;
            self.complete(TempFileTee::if_necessary(xz2::bufread::XzDecoder::new(fd), self)?)?;
            Ok(())
        } else {
            attempt
        }
    }

    // TODO: copy-paste of unpack_stream_xz
    fn unpack_stream_bz2<'c>(&self, fd: &mut Box<Tee + 'c>) -> io::Result<()> {
        let attempt = {
            let br = BoxReader { inner: fd };
            let mut failing: Box<Tee> = Box::new(FailingTee::new(bzip2::read::BzDecoder::new(br)));
            self.unpack_or_die(&mut failing)
        };

        if self.is_format_error_result(&attempt)? {
            fd.reset()?;
            self.complete(TempFileTee::if_necessary(bzip2::read::BzDecoder::new(fd), self)?)?;
            Ok(())
        } else {
            attempt
        }
    }

    fn is_format_error_result<T>(&self, res: &io::Result<T>) -> io::Result<bool> {
        if res.is_ok() {
            return Ok(false);
        }

        // TODO: Ew:
        let error = res.as_ref().err().unwrap();

        if let Some(specific) = is_format_error(&error) {
            match specific {
                FormatErrorType::Other => {
                    self.log(1, || format!(
                        "thought we could unpack '{}' but we couldn't: {:?} {}",
                        self.current.path, error, error))?;
                },
                FormatErrorType::Rewind => {},
            }

            return Ok(true);
        }
        return Ok(false);
    }

    fn unpack(&self, mut fd: Box<Tee>) -> io::Result<()> {
        let res = self.unpack_or_die(&mut fd);

        if self.is_format_error_result(&res)? {
            self.complete(fd)?;
            return Ok(());
        }

        res
    }
}

fn process_real_path<P: AsRef<path::Path>>(path: P, options: &Options) -> io::Result<()> {
    let path = path.as_ref();

    if !path.is_dir() {
        let file = fs::File::open(path)?;

        let unpacker = Unpacker::from_file(
            path.to_str()
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput,
                                              format!("non-utf-8 filename found: {:?}", path)))?,
            &file,
            &options,
        )?;

        return unpacker.unpack(Box::new(BufReaderTee::new(file)));
    }

    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let path = entry.path();
        process_real_path(path, &options)?;
    }
    Ok(())
}

fn must_fit(x: u64) -> u8 {
    if x > std::u8::MAX as u64 {
        panic!("too many something: {}", x);
    }
    x as u8
}

fn real_main() -> u8 {

    let matches = App::new("contentin")
                    .arg(Arg::with_name("v")
                        .short("v")
                        .multiple(true)
                        .help("Sets the level of verbosity (more for more)"))
                    .arg(Arg::with_name("list")
                        .short("t")
                        .long("list")
                        .conflicts_with("to-command")
                        .help("Show headers only, not object content"))
                    .arg(Arg::with_name("no-listing")
                        .short("n")
                        .long("no-listing")
                        .conflicts_with("list")
                        .help("don't print the listing at all"))
                    .arg(Arg::with_name("to-command")
                        .long("to-command")
                        .takes_value(true)
                        .use_delimiter(false)
                        .help("Execute a command for each file; contents on stdin. See man:tar(1)"))
//                    .arg(Arg::with_name("command-failure")
//                        .long("command-failure")
//                        .takes_value(true)
//                        .use_delimiter(false)
//                        .default_value("fatal")
//                        .possible_values(&[
//                            "fatal",
//                            "ignore",
//                        ])
//                        .requires("to-command"))
                    .arg(Arg::with_name("grep")
                        .short("S")
                        .long("grep")
                        .takes_value(true)
                        .conflicts_with("to-command")
                        .help("search for a string in all files"))
                    .arg(Arg::with_name("max-depth")
                        .short("d")
                        .long("max-depth")
                        .takes_value(true)
                        .use_delimiter(false)
                        .default_value("256")
                        .hide_default_value(true)
                        .validator(|val| match val.parse::<u32>() {
                            Ok(_) => Ok(()),
                            Err(e) => Err(format!("must be valid number: {}", e))
                        })
                        .help("Limit recursion. 1: like unzip. Default: lots"))
                    .arg(Arg::with_name("INPUT")
                        .required(true)
                        .help("File(s) to process")
                        .multiple(true))
                    .get_matches();

    let mut listing_output = ListingOutput::Find;
    let mut content_output = ContentOutput::Raw;

    if matches.is_present("list") {
        content_output = ContentOutput::None;
    }

    if matches.is_present("no-listing") {
        listing_output = ListingOutput::None;
    }

    if let Some(cmd) = matches.value_of("to-command") {
        content_output = ContentOutput::ToCommand(cmd.to_string());
    }

    if let Some(expr) = matches.value_of("grep") {
        content_output = ContentOutput::Grep(Regex::new(expr).expect("valid regex"));
    }

    let options = Options {
        listing_output,
        content_output,
        max_depth: matches.value_of("max-depth").unwrap().parse().unwrap(),
        verbose: must_fit(matches.occurrences_of("v")),
    };

    for path in matches.values_of("INPUT").unwrap() {
        if let Err(e) = process_real_path(path, &options) {
            if let Err(_) = writeln!(io::stderr(), "fatal: processing '{}': {:?}", path, e) {
                return 6;
            }

            return 4;
        }
    }
    return 0;
}


fn main() {
    std::process::exit(real_main() as i32)
}

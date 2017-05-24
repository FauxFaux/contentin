extern crate ar;
extern crate bzip2;
extern crate clap;
extern crate libflate;
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
use std::time;

use std::rc::Rc;

use clap::{Arg, App};

use libflate::gzip;
use tempfile::tempfile;

mod filetype;
use filetype::FileType;

mod slist;
use slist::Node;

mod tee;
use tee::*;

mod stat;
use stat::Stat;


// magic:
use std::io::Write;

struct Options {
    list: bool,
    max_depth: u32,
    verbose: u8,
}

struct FileDetails {
    path: Rc<Node<String>>,
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

struct Unpacker<'a> {
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
        writeln!(stdout, "{:?} {} {} {} {} {} {} {} {} {}",
                 self.current.path.to_vec(),
                 size,
                 self.current.atime, self.current.mtime, self.current.ctime, self.current.btime,
                 self.current.uid, self.current.gid, self.current.user_name, self.current.group_name,
        )?;

        if self.options.list {
            return Ok(())
        }

        io::copy(&mut src, &mut stdout).and_then(move |written|
            if written != size {
                Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    format!("expecting to write {} but wrote {}", size, written)))
            } else {
                Ok(())
            })
    }

    fn from_file<'b>(path: &str, fd: &fs::File, options: &'b Options) -> io::Result<Unpacker<'b>> {
        let meta = fd.metadata()?;
        let stat = Stat::from(&meta);
        Ok(Unpacker {
            options,
            current: FileDetails {
                depth: 0,
                path: Node::head(path.to_string()),
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
                path: Node::plus(&self.current.path, path.to_string()),
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
            }
        },
        io::ErrorKind::BrokenPipe
        | io::ErrorKind::NotFound
        | io::ErrorKind::PermissionDenied
        => return None,
        _ => {},
    }

    panic!("don't know if {:?} / {:?} / {:?} is a format error", e, e.kind(), e.get_ref())
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

            if let Err(ref error) = res {
                if self.is_format_error_result(error)? {
                    let new_entry = zip.by_index(i)?;
                    let size = new_entry.size();
                    self.complete_details(new_entry, size)?;
                    continue;
                }
            }

            // scope based borrow sigh; same block as above
            if res.is_err() {
                return res;
            }
        }
        Ok(())
    }

    fn unpack_or_die<'b>(&self, mut fd: &mut Box<Tee + 'b>) -> io::Result<()> {
        if self.current.depth >= self.options.max_depth {
            return Err(io::Error::new(io::ErrorKind::Other, Rewind {}));
        }

        let identity = FileType::identify(fd.fill_buf()?);
        self.log(2, || format!("identified as {}", identity))?;
        match identity {
            FileType::GZip => {
                let dec = gzip::Decoder::new(fd)?;
                let unpacker = {
                    let header = dec.header();
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
                    unpacker
                };
                let mut failing: Box<Tee> = Box::new(FailingTee::new(dec));
                unpacker.unpack_or_die(&mut failing)
            },

            // xz and bzip2 have *nothing* in their header; no mtime, no name, no source OS, no nothing.
            FileType::Xz => {
                let unpacker = self.with_path(self.strip_compression_suffix(".xz"));
                let mut failing: Box<Tee> = Box::new(FailingTee::new(xz2::bufread::XzDecoder::new(fd)));
                unpacker.unpack_or_die(&mut failing)
            },
            FileType::BZip2 => {
                let unpacker = self.with_path(self.strip_compression_suffix(".bz2"));
                let mut failing: Box<Tee> = Box::new(FailingTee::new(bzip2::read::BzDecoder::new(fd)));
                unpacker.unpack_or_die(&mut failing)
            }

            FileType::Deb => {
                let mut decoder = ar::Archive::new(fd);
                while let Some(entry) = decoder.next_entry() {
                    let entry = entry?;
                    let unpacker = self.with_path(entry.header().identifier());
                    unpacker.unpack(Box::new(TempFileTee::new(entry)?))?;
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
                    unpacker.unpack(Box::new(TempFileTee::new(entry)?))?;
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


    fn is_format_error_result(&self, error: &io::Error) -> io::Result<bool> {
        if let Some(specific) = is_format_error(error) {
            match specific {
                FormatErrorType::Other => {
                    self.log(1, || format!(
                        "thought we could unpack '{:?}' but we couldn't: {}",
                        self.current.path, error))?;
                },
                FormatErrorType::Rewind => {},
            }

            return Ok(true);
        }
        return Ok(false);
    }

    fn unpack(&self, mut fd: Box<Tee>) -> io::Result<()> {
        let res = self.unpack_or_die(&mut fd);

        if let Err(ref error) = res {
            if self.is_format_error_result(&error)? {
                self.complete(fd)?;
                return Ok(());
            }
        }

        res
    }
}

fn process_real_path(path: &str, options: &Options) -> io::Result<()> {
    let file = fs::File::open(path)?;
    let unpacker = Unpacker::from_file(
        path,
        &file,
        &options,
    )?;

    unpacker.unpack(Box::new(BufReaderTee::new(file)))
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
                        .help("Show headers only, not object content"))
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

    let options = Options {
        list: matches.is_present("list"),
        max_depth: matches.value_of("max-depth").unwrap().parse().unwrap(),
        verbose: must_fit(matches.occurrences_of("v")),
    };

    for path in matches.values_of("INPUT").unwrap() {
        if let Err(e) = process_real_path(path, &options) {
            if let Err(_) = writeln!(io::stderr(), "fatal: processing '{}': {}", path, e) {
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

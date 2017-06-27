extern crate ar;
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

use std::fmt;
use std::fs;
use std::io;
use std::path;
use std::time;

use clap::{Arg, App};

use libflate::gzip;

mod errors;
mod filetype;
mod output_capnp;
mod slist;
mod stat;
mod tee;

use errors::*;
use filetype::FileType;
use slist::SList;
use stat::Stat;
use tee::*;

// magic:
use std::io::BufRead;
use std::io::Write;

enum ListingOutput {
    None,
    Capnp,
    Find,
}

pub enum ContentOutput {
    None,
    Raw,
}

struct Options {
    listing_output: ListingOutput,
    content_output: ContentOutput,
    max_depth: u32,
    verbose: u8,
}

enum ArchiveReadFailure {
    Open(String),
    Read(String),
}

#[derive(Debug)]
pub enum ItemType {
    // TODO: Magic value "Unknown", or an Option, or..?
    Unknown,
    RegularFile,
    Directory,
    Fifo,
    Socket,
    /// A symlink, with its destination.
    SymbolicLink(String),
    /// A 'c' device.
    CharacterDevice { major: u32, minor: u32 },
    /// A 'b' device.
    BlockDevice { major: u32, minor: u32 },
}

pub struct FileDetails {
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
    mode: u32,
    item_type: ItemType,
    failure: Option<ArchiveReadFailure>,
}

pub struct Unpacker<'a> {
    options: &'a Options,
    current: FileDetails,
}

impl<'a> Unpacker<'a> {
    fn log<T: fmt::Display, F>(&self, level: u8, msg: F) -> Result<()>
    where
        F: FnOnce() -> T,
    {
        if self.options.verbose < level {
            return Ok(());
        }

        let name = match level {
            0 => "error",
            1 => "warn",
            2 => "info",
            3 => "debug",
            _ => unreachable!(),
        };

        writeln!(io::stderr(), "{}: {}", name, msg()).map(|_| ())?;
        Ok(())
    }

    fn complete(&self, mut file: Box<Tee>) -> Result<()> {
        let size = file.len_and_reset()?;
        self.complete_details(file, size)
    }

    fn complete_details<R: io::Read>(&self, mut src: R, size: u64) -> Result<()> {
        let stdout = io::stdout();
        let mut stdout = stdout.lock();

        let current_path = || self.current.path.to_vec().join(" / ");

        match self.options.listing_output {
            ListingOutput::None => {}
            ListingOutput::Find => {
                writeln!(stdout, "{} {} {} {} {} {} {} {} {} {}",
                         current_path(),
                         size,
                         self.current.atime, self.current.mtime, self.current.ctime, self.current.btime,
                         self.current.uid, self.current.gid, self.current.user_name, self.current.group_name,
                )?;
            }
            ListingOutput::Capnp => {
                output_capnp::write_capnp(
                    &mut stdout,
                    &self.current,
                    &self.options.content_output,
                    size,
                )?;
            }
        }

        match self.options.content_output {
            ContentOutput::None => Ok(()),
            ContentOutput::Raw => {
                io::copy(&mut src, &mut stdout).and_then(
                    move |written| if written !=
                        size
                    {
                        Err(io::Error::new(
                            io::ErrorKind::UnexpectedEof,
                            format!("expecting to write {} but wrote {}", size, written),
                        ))
                    } else {
                        Ok(())
                    },
                )?;
                Ok(())
            }
        }
    }

    fn from_file<'b>(path: &str, meta: fs::Metadata, options: &'b Options) -> Result<Unpacker<'b>> {
        let stat: Stat = Stat::from(&meta);

        let item_type = if meta.is_dir() {
            unreachable!()
        } else if meta.file_type().is_symlink() {
            match fs::read_link(path)?.to_str() {
                Some(dest) => ItemType::SymbolicLink(dest.to_string()),
                None => {
                    bail!(ErrorKind::UnsupportedFeature(format!(
                        "{} is a symlink to an invaild utf-8 sequence",
                        path
                    )))
                }
            }
        } else if meta.is_file() {
            ItemType::RegularFile
        } else {
            if 0 == stat.mode {
                // "mode" will be zero on platforms that don't do modes (i.e. Windows).
                // TODO: detect symlinks (and DEVices) on Windows?
                ItemType::Unknown
            } else {
                // TODO: use libc here, or assume the constants are the same everywhere?
                let mode_type = (stat.mode >> 12) & 0b1111;
                match mode_type {
                    0b0100 => unreachable!(), // S_IFDIR
                    0b1000 => unreachable!(), // S_IFREG
                    0b1010 => unreachable!(), // S_IFLNK

                    0b0001 => ItemType::Fifo, // S_IFIFO
                    0b1100 => ItemType::Socket, // S_IFSOCK

                    0b0010 => panic!("TODO: char device"), // S_IFCHR
                    0b0110 => panic!("TODO: block device"), // S_IFBLK

                    _ => {
                        bail!(ErrorKind::UnsupportedFeature(
                            format!("unrecognised unix mode type {:b}", mode_type),
                        ))
                    }
                }
            }
        };

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
                mode: stat.mode,
                item_type,
                failure: None,
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
                mode: 0,
                item_type: ItemType::Unknown,
                failure: None,
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
    dur.as_secs().checked_mul(1_000_000_000).map_or(
        0,
        |nanos| {
            nanos + dur.subsec_nanos() as u64
        },
    )
}

fn simple_time_sys(val: time::SystemTime) -> u64 {
    val.duration_since(time::UNIX_EPOCH)
        .map(simple_time)
        .unwrap_or(0)
}


fn simple_time_tm(val: crates_time::Tm) -> u64 {
    let timespec = val.to_timespec();
    simple_time(time::Duration::new(
        timespec.sec as u64,
        timespec.nsec as u32,
    ))
}

fn simple_time_btime(val: &fs::Metadata) -> Result<u64> {
    match val.created() {
        Ok(time) => Ok(simple_time_sys(time)),
        // "Other" is how "unsupported" is represented here; ew.
        Err(ref e) if e.kind() == io::ErrorKind::Other => Ok(0),
        Err(other) => Err(other).chain_err(|| "loading btime"),
    }
}

fn simple_time_ext4(val: &ext4::Time) -> u64 {
    let nanos = val.nanos.unwrap_or(0);
    if nanos > 1_000_000_000 {
        // TODO: there are some extra bits here for us, which I'm too lazy to implement
        return 0;
    }

    if val.epoch_secs > 0x7fff_ffff {
        // Negative time, which we're actually not supporting?
        return 0;
    }

    (val.epoch_secs as u64) * 1_000_000_000 + nanos as u64
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

impl<'a> Unpacker<'a> {
    fn process_zip<T>(&self, from: T) -> Result<()>
    where
        T: io::Read + io::Seek,
    {
        let mut zip = zip::ZipArchive::new(from).chain_err(|| "opening zip")?;

        for i in 0..zip.len() {
            let unpacker = {
                let entry = zip.by_index(i).chain_err(|| format!("opening entry {}", i))?;
                let mut unpacker = self.with_path(entry.name());

                unpacker.current.mtime = simple_time_tm(entry.last_modified());
                unpacker.current.mode = entry.unix_mode().unwrap_or(0);
                unpacker
            };

            let res = {
                let entry = zip.by_index(i)?;
                let mut failing: Box<Tee> = Box::new(FailingTee::new(entry));
                unpacker.unpack_or_die(&mut failing)
            };

            if self.is_format_error_result(&res)? {
                let new_entry = zip.by_index(i)?;
                let size = new_entry.size();
                unpacker.complete_details(new_entry, size).chain_err(
                    || "..after rollback",
                )?;
                continue;
            }

            if res.is_err() {
                return res;
            }
        }
        Ok(())
    }

    fn process_partition<T>(&self, inner: T) -> Result<bool>
    where
        T: io::Read + io::Seek,
    {
        let mut failed = false;
        let mut settings = ext4::Options::default();
        settings.checksums = ext4::Checksums::Enabled;
        let mut fs = ext4::SuperBlock::new_with_options(inner, &settings)
            .chain_err(|| "opening filesystem")?;
        let root = &fs.root().chain_err(|| "loading root")?;
        fs.walk(
            root,
            "".to_string(),
            &mut |fs, path, inode, enhanced| {
                use ext4::Enhanced::*;
                match *enhanced {
                    Directory(_) => {}
                    RegularFile => {
                        let mut unpacker = self.with_path(path);
                        {
                            let current = &mut unpacker.current;
                            let stat: &ext4::Stat = &inode.stat;
                            current.uid = stat.uid;
                            current.gid = stat.gid;
                            current.atime = simple_time_ext4(&stat.atime);
                            current.mtime = simple_time_ext4(&stat.mtime);
                            current.ctime = simple_time_ext4(&stat.ctime);
                            current.btime = match stat.btime.as_ref() {
                                Some(btime) => simple_time_ext4(btime),
                                None => 0,
                            };
                            current.mode = stat.file_mode as u32
                        }

                        // TODO: this should be a BufReaderTee, but BORROWS. HORRIBLE INEFFICIENCY
                        let tee = TempFileTee::if_necessary(fs.open(inode)?, &unpacker)
                            .map_err(|e| ext4::Error::with_chain(e, "tee"))?;

                        unpacker.unpack(tee).map_err(|e| {
                            ext4::Error::with_chain(e, "unpacking")
                        })?
                    }
                    _ => {
                        failed = true;
                        self.log(1, || {
                            format!("unimplemented filesystem entry: {} {:?}", path, enhanced)
                        }).map_err(|e| ext4::Error::with_chain(e, "logging"))?;
                    }
                }
                Ok(true)
            },
        )?;

        Ok(false)
    }


    fn process_tar<'c>(&self, fd: &mut Box<Tee + 'c>) -> Result<()> {
        let mut decoder = tar::Archive::new(fd);
        for entry in decoder.entries()? {
            let entry = entry.map_err(tar_err).chain_err(|| "parsing header")?;
            let mut unpacker =
                self.with_path(entry.path().map_err(tar_err)?.to_str().ok_or_else(|| {
                    ErrorKind::UnsupportedFeature(
                        format!("invalid path utf-8: {:?}", entry.path_bytes()),
                    )
                })?);

            {
                let current = &mut unpacker.current;
                let header = entry.header();

                current.uid = header.uid().map_err(tar_err).chain_err(|| "reading uid")?;

                current.gid = header.gid().map_err(tar_err).chain_err(|| "reading gid")?;

                current.mtime =
                    simple_time_epoch_seconds(header.mtime().map_err(tar_err).chain_err(
                        || "reading mtime",
                    )?);

                if let Some(found) = header.username().map_err(|e| {
                    ErrorKind::UnsupportedFeature(format!(
                        "invalid username utf-8: {} {:?}",
                        e,
                        header.username_bytes()
                    ))
                })?
                {
                    current.user_name = found.to_string();
                }
                if let Some(found) = header.groupname().map_err(|e| {
                    ErrorKind::UnsupportedFeature(format!(
                        "invalid groupname utf-8: {} {:?}",
                        e,
                        header.groupname_bytes()
                    ))
                })?
                {
                    current.group_name = found.to_string();
                }

                current.mode = header.mode()?;
            }

            unpacker
                .unpack(TempFileTee::if_necessary(entry, &unpacker)?)
                .chain_err(|| {
                    format!("processing tar entry: {}", unpacker.current.path.inner())
                })?;
        }
        Ok(())
    }

    fn with_gzip(&self, header: &gzip::Header) -> Result<Unpacker> {
        let mtime = simple_time_epoch_seconds(header.modification_time() as u64);
        let name = match header.filename() {
            Some(ref c_str) => {
                c_str.to_str().map_err(|not_utf8| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!(
                            "gzip member's name must be valid utf-8: {} {:?}",
                            not_utf8,
                            c_str.as_bytes()
                        ),
                    )
                })?
            }
            None => self.strip_compression_suffix(".gz"),
        };

        let mut unpacker = self.with_path(name);
        unpacker.current.mtime = mtime;
        Ok(unpacker)
    }

    fn unpack_or_die<'b>(&self, mut fd: &mut Box<Tee + 'b>) -> Result<()> {
        if self.current.depth >= self.options.max_depth {
            bail!(ErrorKind::Rewind);
        }

        let identity = FileType::identify(fd.fill_buf()?);
        self.log(2, || {
            format!("identified '{}' as {}", self.current.path.inner(), identity)
        })?;
        match identity {
            FileType::GZip => {
                let (attempt, unpacker) = {
                    let br = BoxReader { inner: fd };
                    let dec = gzip::Decoder::new(br)?;

                    let unpacker = self.with_gzip(dec.header())?;

                    let mut failing: Box<Tee> = Box::new(FailingTee::new(dec));
                    (
                        unpacker.unpack_or_die(&mut failing).chain_err(
                            || "streaming gzip",
                        ),
                        unpacker,
                    )
                };


                if self.is_format_error_result(&attempt)? {
                    fd.reset()?;
                    unpacker.complete(TempFileTee::if_necessary(
                        gzip::Decoder::new(fd)?,
                        &unpacker,
                    )?)?;
                    Ok(())
                } else {
                    attempt
                }
            }

            // xz and bzip2 have *nothing* in their header; no mtime, no name, no source OS, no nothing.
            FileType::Xz => {
                self.with_path(self.strip_compression_suffix(".xz"))
                    .unpack_stream_xz(fd)
                    .chain_err(|| "unpacking xz")
            }
            FileType::BZip2 => {
                self.with_path(self.strip_compression_suffix(".bz2"))
                    .unpack_stream_bz2(fd)
                    .chain_err(|| "unpacking bz2")
            }

            FileType::Deb => {
                let mut decoder = ar::Archive::new(fd);
                while let Some(entry) = decoder.next_entry() {
                    let entry = entry?;
                    let unpacker = self.with_path(entry.header().identifier());
                    unpacker
                        .unpack(TempFileTee::if_necessary(entry, &unpacker)?)
                        .chain_err(|| format!("unpacking deb entry {}", unpacker.current.path))?;
                }
                Ok(())
            }
            FileType::Tar => self.process_tar(fd).chain_err(|| "unpacking tar"),
            FileType::Zip => {
                self.process_zip(fd.as_seekable()?).chain_err(
                    || "reading zip file",
                )
            }
            FileType::Other => Err(ErrorKind::Rewind.into()),
            FileType::MBR => {
                let mut fd = fd.as_seekable()?;
                let mut failed = false;
                for partition in ext4::mbr::read_partition_table(&mut fd)? {
                    let inner = ext4::mbr::read_partition(&mut fd, &partition)?;
                    failed |= self.process_partition(inner)?;
                }
                if failed {
                    Err(ErrorKind::Rewind.into())
                } else {
                    Ok(())
                }
            }
        }
    }

    // TODO: Work out how to generic these copy-pastes
    fn unpack_stream_xz<'c>(&self, fd: &mut Box<Tee + 'c>) -> Result<()> {
        let attempt = {
            let br = BoxReader { inner: fd };
            let mut failing: Box<Tee> = Box::new(FailingTee::new(xz2::bufread::XzDecoder::new(br)));
            self.unpack_or_die(&mut failing)
        };

        if self.is_format_error_result(&attempt)? {
            fd.reset()?;
            self.complete(TempFileTee::if_necessary(
                xz2::bufread::XzDecoder::new(fd),
                self,
            )?)?;
            Ok(())
        } else {
            attempt
        }
    }

    // TODO: copy-paste of unpack_stream_xz
    fn unpack_stream_bz2<'c>(&self, fd: &mut Box<Tee + 'c>) -> Result<()> {
        let attempt = {
            let br = BoxReader { inner: fd };
            let mut failing: Box<Tee> = Box::new(FailingTee::new(bzip2::read::BzDecoder::new(br)));
            self.unpack_or_die(&mut failing)
        };

        if self.is_format_error_result(&attempt)? {
            fd.reset()?;
            self.complete(TempFileTee::if_necessary(
                bzip2::read::BzDecoder::new(fd),
                self,
            )?)?;
            Ok(())
        } else {
            attempt
        }
    }

    fn is_format_error_result<T>(&self, res: &Result<T>) -> Result<bool> {

        let problem = errors::classify_format_error_result(res);

        if let Some(specific) = problem {
            match specific {
                FormatErrorType::Other => {
                    let error = res.as_ref().err().unwrap();
                    self.log(1, || {
                        format!(
                            "thought we could unpack '{}' but we couldn't: {:?} {}",
                            self.current.path,
                            error,
                            error
                        )
                    })?;
                }
                FormatErrorType::Rewind => {}
            }

            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn unpack(&self, mut fd: Box<Tee>) -> Result<()> {
        let res = self.unpack_or_die(&mut fd).chain_err(|| "unpacking failed");

        if self.is_format_error_result(&res)? {
            self.complete(fd)?;
            return Ok(());
        }

        res
    }
}

fn process_real_path<P: AsRef<path::Path>>(path: P, options: &Options) -> Result<()> {
    let path = path.as_ref();

    if !path.is_dir() {
        let metadata = fs::symlink_metadata(path)?;

        let unpacker = Unpacker::from_file(
            path.to_str().ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("non-utf-8 filename found: {:?}", path),
                )
            })?,
            metadata,
            &options,
        )?;

        return match unpacker.current.item_type {
            ItemType::Directory => unreachable!(),

            ItemType::SymbolicLink(_) |
            ItemType::CharacterDevice { major: _, minor: _ } |
            ItemType::BlockDevice { major: _, minor: _ } |
            ItemType::Fifo |
            ItemType::Socket => {
                // can't actually read from these guys
                unpacker.complete_details(io::Cursor::new(&[]), 0)
            }

            ItemType::Unknown | ItemType::RegularFile => {
                let file = fs::File::open(path)?;
                unpacker.unpack(Box::new(BufReaderTee::new(file)))
            }
        };
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
    let mut content_output = ContentOutput::Raw;

    if matches.is_present("list") {
        content_output = ContentOutput::None;
    }

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
        process_real_path(path, &options).chain_err(|| {
            format!("processing: '{}'", path)
        })?;
    }

    return Ok(0);
}

fn tar_err(e: io::Error) -> Error {
    if io::ErrorKind::Other != e.kind() {
        return Error::with_chain(e, "reading tar");
    }

    ErrorKind::Tar(format!("{}", e)).into()
}

quick_main!(real_main);

extern crate ar;
extern crate bzip2;
extern crate clap;
extern crate libflate;
extern crate tar;
extern crate tempfile;
extern crate time as crates_time;
extern crate xz2;
extern crate zip;

use std::error;
use std::fmt;
use std::fs;
use std::io;
use std::time;

use std::rc::Rc;
use std::vec::Vec;

use clap::{Arg, App};

use libflate::gzip;
use tempfile::tempfile;

// magic:
use std::io::Seek;
use std::io::Write;

#[derive(Debug)]
struct Node<T> {
    next: Option<Rc<Node<T>>>,
    value: T,
}

impl<T: Clone> Node<T> {
    fn head(obj: T) -> Rc<Node<T>> {
        Rc::new(Node {
            next: None,
            value: obj,
        })
    }

    fn plus(what: &Rc<Node<T>>, obj: T) -> Rc<Node<T>> {
        Rc::new(Node {
            next: Some(what.clone()),
            value: obj
        })
    }

    fn to_vec(&self) -> Vec<T> {
        let mut ret = Vec::new();
        let mut val = self;
        loop {
            ret.push(val.value.clone());
            if let Some(ref next) = val.next {
                val = next;
            } else {
                break;
            }
        }
        ret.reverse();
        ret
    }
}

struct Options {
    list: bool,
    max_depth: u32,
    verbose: u8,
}

struct OutputTo<'a> {
    options: &'a Options,
    path: Rc<Node<String>>,
    depth: u32,
    atime: u64,
    mtime: u64,
    ctime: u64,
    btime: u64,
}

impl<'a> OutputTo<'a> {
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

    fn raw(&self, mut file: Box<Tee>) -> io::Result<()> {
        let stdout = io::stdout();
        let mut stdout = stdout.lock();
        let size = file.len_and_reset()?;
        writeln!(stdout, "{:?} {} {} {} {} {}",
            self.path.to_vec(),
            size,
            self.atime, self.mtime, self.ctime, self.btime
        )?;

        if self.options.list {
            return Ok(())
        }

        io::copy(&mut file, &mut stdout).and_then(move |written|
            if written != size {
                Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    format!("expecting to write {} but wrote {}", size, written)))
            } else {
                Ok(())
            })
    }

    fn from_file<'b>(path: &str, fd: &fs::File, options: &'b Options) -> io::Result<OutputTo<'b>> {
        let meta = fd.metadata()?;
        Ok(OutputTo {
            options,
            depth: 0,
            path: Node::head(path.to_string()),
            atime: meta.accessed().map(simple_time_sys)?,
            mtime: meta.modified().map(simple_time_sys)?,
            ctime: simple_time_ctime(&meta),
            btime: simple_time_btime(&meta)?,
        })
    }

    fn with_path(&self, path: &str) -> OutputTo {
        OutputTo {
            options: self.options,
            path: Node::plus(&self.path, path.to_string()),
            depth: self.depth + 1,
            atime: 0,
            mtime: 0,
            ctime: 0,
            btime: 0,
        }
    }

    fn strip_compression_suffix(&self, suffix: &str) -> &str {
        let our_name = self.path.value.as_str();
        if our_name.ends_with(suffix) {
            &our_name[..our_name.len() - suffix.len()]
        } else {
            ""
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FileType {
    GZip,
    Zip,
    Tar,
    BZip2,
    Xz,
    Deb,
    Other,
}

impl fmt::Display for FileType {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}

fn read_octal(bytes: &[u8]) -> Option<u32> {
    let mut start = 0;
    while start < bytes.len() && b' ' == bytes[start] {
        start += 1;
    }

    let mut end = bytes.len() - 1;
    while end > start && (b' ' == bytes[end] || 0 == bytes[end]) {
        end -= 1;
    }

    if let Ok(string) = std::str::from_utf8(&bytes[start..(end+1)]) {
        if let Ok(val) = u32::from_str_radix(string, 8) {
            return Some(val);
        }
    }
    None
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

fn simple_time_epoch_seconds(seconds: u32) -> u64 {
    (seconds as u64).checked_mul(1_000_000_000).unwrap_or(0)
}

// TODO: I really feel this should be exposed by Rust, without cfg.
fn simple_time_ctime(val: &fs::Metadata) -> u64 {
    #[cfg(target_os = "linux")] {
        use std::os::linux::fs::MetadataExt;
        let ctime: i64 = val.st_ctime();
        if ctime <= 0 {
            0
        } else {
            ctime as u64
        }
    }

    #[cfg(not(target_os="linux"))] {
        0
    }
}

fn is_probably_tar(header: &[u8]) -> bool {
    if header.len() < 512 {
        return false;
    }

    if let Some(checksum) = read_octal(&header[148..156]) {
        let mut sum: u32 = (b' ' as u32) * 8;
        for i in 0..148 {
            sum += header[i] as u32;
        }
        for i in 156..512 {
            sum += header[i] as u32;
        }

        if checksum == sum {
            return true;
        }
    }

    return false;
}

const DEB_PREFIX: &[u8] = b"!<arch>\ndebian-binary ";

fn identify<'a>(header: &[u8]) -> FileType {
    if header.len() >= 20
        && 0x1f == header[0] && 0x8b == header[1] {
        FileType::GZip
   } else if header.len() >= 152
        && b'P' == header[0] && b'K' == header[1]
        && 0x03 == header[2] && 0x04 == header[3] {
        FileType::Zip
    } else if header.len() > 257 + 10
        && b'u' == header[257] && b's' == header[258]
        && b't' == header[259] && b'a' == header[260]
        && b'r' == header[261]
        && (
            (0 == header[262] && b'0' == header[263] && b'0' == header[264]) ||
            (b' ' == header[262] && b' ' == header[263] && 0 == header[264])
        ) {
        FileType::Tar
    } else if header.len() > 70
        && header[0..DEB_PREFIX.len()] == DEB_PREFIX[..]
        && header[66..70] == b"`\n2."[..] {
        FileType::Deb
    } else if header.len() > 40
        && b'B' == header[0] && b'Z' == header[1]
        && b'h' == header[2] && 0x31 == header[3]
        && 0x41 == header[4] && 0x59 == header[5]
        && 0x26 == header[6] {
        FileType::BZip2
    } else if header.len() > 6
        && 0xfd == header[0] && b'7' == header[1]
        && b'z' == header[2] && b'X' == header[3]
        && b'Z' == header[4] && 0 == header[5] {
        FileType::Xz
    } else if is_probably_tar(header) {
        FileType::Tar
    } else {
        FileType::Other
    }
}

trait Tee: io::BufRead {
    fn reset(&mut self) -> io::Result<()>;
    fn len_and_reset(&mut self) -> io::Result<u64>;
    fn mut_ref(&mut self) -> &mut io::BufRead;
}

struct TempFileTee {
    inner: io::BufReader<fs::File>,
}

impl TempFileTee {
    fn new<U: io::Read>(from: U) -> io::Result<TempFileTee> {
        // TODO: take a size hint, and consider using memory, or shm,
        // TODO: or take a temp file path, or..
        let mut tmp = tempfile()?;

        {
            let mut reader = io::BufReader::new(from);
            let mut writer = io::BufWriter::new(&tmp);
            io::copy(&mut reader, &mut writer)?;
        }

        tmp.seek(BEGINNING)?;

        Ok(TempFileTee {
            inner: io::BufReader::new(tmp),
        })
    }
}

const BEGINNING: io::SeekFrom = io::SeekFrom::Start(0);
const END: io::SeekFrom = io::SeekFrom::End(0);

impl Tee for TempFileTee {
    fn reset(&mut self) -> io::Result<()> {
        self.inner.seek(BEGINNING).map(|_| ())
    }

    fn len_and_reset(&mut self) -> io::Result<u64> {
        let len = self.inner.seek(END)?;
        self.reset()?;
        Ok(len)
    }

    fn mut_ref(&mut self) -> &mut io::BufRead {
        &mut self.inner
    }
}

// Look, I didn't want to implement these. I wanted to return the implementation.
// But I couldn't make it compile, and I might care enough eventually.
impl io::Read for TempFileTee {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.inner.read(buf)
    }
}

impl io::BufRead for TempFileTee {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        self.inner.fill_buf()
    }

    fn consume(&mut self, amt: usize) {
        self.inner.consume(amt)
    }
}

struct BufReaderTee<T> {
    inner: Box<T>,
}

impl<U: io::Read> BufReaderTee<io::BufReader<U>> {
    fn new(from: U) -> Self {
        BufReaderTee {
            inner: Box::new(io::BufReader::new(from))
        }
    }
}

impl<T> Tee for BufReaderTee<T>
    where T: io::BufRead + io::Seek + 'static
{
    fn reset(&mut self) -> io::Result<()> {
        self.inner.seek(io::SeekFrom::Start(0)).map(|_|())
    }

    fn len_and_reset(&mut self) -> io::Result<u64> {
        let len = self.inner.seek(END)?;
        self.reset()?;
        Ok(len)
    }

    fn mut_ref(&mut self) -> &mut io::BufRead {
        &mut self.inner
    }
}

impl<T> io::Read for BufReaderTee<T>
    where T: io::Read {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.inner.read(buf)
    }
}

impl<T> io::BufRead for BufReaderTee<T>
    where T: io::BufRead {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        self.inner.fill_buf()
    }

    fn consume(&mut self, amt: usize) {
        self.inner.consume(amt)
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

fn unpack_or_die<'a>(mut fd: &mut io::BufRead, output: &OutputTo) -> io::Result<()> {
    let identity = identify(fd.fill_buf()?);
    output.log(2, || format!("identified as {}", identity))?;
    match identity {
        FileType::GZip => {
            let dec = gzip::Decoder::new(fd)?;
            let new_output = {
                let header = dec.header();
                let mtime = simple_time_epoch_seconds(header.modification_time());
                let name = match header.filename() {
                    Some(ref c_str) => c_str.to_str().map_err(
                        |not_utf8| io::Error::new(io::ErrorKind::InvalidData,
                                  format!("gzip member's name must be valid utf-8: {} {:?}",
                                          not_utf8, c_str.as_bytes())))?,
                    None => output.strip_compression_suffix(".gz"),
                };

                let mut new_output = output.with_path(name);
                new_output.mtime = mtime;
                new_output
            };

            unpack_or_die(&mut io::BufReader::new(dec), &new_output)
        },

        // xz and bzip2 have *nothing* in their header; no mtime, no name, no source OS, no nothing.
        FileType::Xz => {
            let new_output = output.with_path(output.strip_compression_suffix(".xz"));
            unpack_or_die(&mut io::BufReader::new(xz2::bufread::XzDecoder::new(fd)), &new_output)
        },
        FileType::BZip2 => {
            let new_output = output.with_path(output.strip_compression_suffix(".bz2"));
            unpack_or_die(&mut io::BufReader::new(bzip2::read::BzDecoder::new(fd)), &new_output)
        }

        FileType::Deb => {
            let mut decoder = ar::Archive::new(fd);
            while let Some(entry) = decoder.next_entry() {
                let entry = entry?;
                let new_output = output.with_path(entry.header().identifier());
                unpack(Box::new(TempFileTee::new(entry)?), &new_output)?;
            }
            Ok(())
        },
        FileType::Tar => {
            let mut decoder = tar::Archive::new(fd);
            for entry in decoder.entries()? {
                let entry = entry?;
                let new_output = output.with_path(entry.path()?.to_str()
                    .ok_or_else(|| io::Error::new(io::ErrorKind::Other,
                                      format!("tar path contains invalid utf-8: {:?}",
                                              entry.path_bytes())))?);

                unpack(Box::new(TempFileTee::new(entry)?), &new_output)?;
            }
            Ok(())
        },
        FileType::Zip => {
            // TODO: teach Tee to expose its Seekable if it can, otherwise tempfile
            let mut temp = tempfile()?;
            io::copy(&mut fd, &mut temp)?;
            let mut zip = zip::ZipArchive::new(temp)?;

            for i in 0..zip.len() {

                // well, this has gone rather poorly
                let mut new_output = output.with_path(zip.by_index(i)?.name());

                let entry = zip.by_index(i)?;

                new_output.mtime = simple_time_tm(entry.last_modified());
                let reader = Box::new(TempFileTee::new(entry)?);
                unpack(reader, &new_output)?;
            }
            Ok(())
        },
        FileType::Other => {
            Err(io::Error::new(io::ErrorKind::Other, Rewind {}))
        },
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

fn unpack(mut fd: Box<Tee>, output: &OutputTo) -> io::Result<()> {
    if output.depth >= output.options.max_depth {
        output.raw(fd)?;
        return Ok(());
    }

    let res = unpack_or_die(fd.mut_ref(), output);
    if let Err(ref raw_error) = res {
        if let Some(specific) = is_format_error(raw_error) {
            match specific {
                FormatErrorType::Other => {
                    output.log(1, || format!(
                        "thought we could unpack '{:?}' but we couldn't: {}",
                        output.path, raw_error))?;
                },
                FormatErrorType::Rewind => {},
            }

            output.raw(fd)?;
            return Ok(());
        }
    }

    res
}

fn process_real_path(path: &str, options: &Options) -> io::Result<()> {
    let file = fs::File::open(path)?;
    let output = OutputTo::from_file(
        path,
        &file,
        &options,
    )?;

    unpack(Box::new(BufReaderTee::new(file)), &output)
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

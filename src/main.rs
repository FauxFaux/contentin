extern crate ar;
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
    verbose: u8,
}

struct OutputTo<'a> {
    options: &'a Options,
    path: Rc<Node<String>>,
    atime: u64,
    mtime: u64,
    ctime: u64,
    btime: u64,
}

impl<'a> OutputTo<'a> {
    fn log<T: ?Sized>(&self, level: u8, msg: &T) -> io::Result<()>
    where T: fmt::Display
    {
        if self.options.verbose < level {
            return Ok(());
        }

        let name = match level {
            1 => "warn".to_string(),
            2 => "info".to_string(),
            3 => "debug".to_string(),
            _ => format!("v{}", level),
        };

        writeln!(io::stderr(), "{}: {}", name, msg).map(|_|())
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
            atime: 0,
            mtime: 0,
            ctime: 0,
            btime: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FileType {
    GZip,
    Zip,
    Tar,
    Ar,
    BZip2,
    Xz,
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

// TODO: I really feel this should be exposed by Rust, without cfg.
fn simple_time_ctime(val: &fs::Metadata) -> u64 {
    #[cfg(linux)] {
        use std::os::linux::fs::MetadataExt;
        let ctime: i64 = val.st_ctime();
        if ctime <= 0 {
            0
        } else {
            ctime as u64
        }
    }

    #[cfg(not(linux))] {
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

fn identify<'a>(fd: &mut Box<Tee>, output: &OutputTo) -> io::Result<FileType> {
    let header = fd.fill_buf()?;

//    if header.len() >= 3 {
//        output.log(3, &format!("{} {} {}", header[0], header[1], header[2]))?;
//    }

    if header.len() >= 20
        && 0x1f == header[0] && 0x8b == header[1] {
        Ok(FileType::GZip)
   } else if header.len() >= 152
        && b'P' == header[0] && b'K' == header[1]
        && 0x03 == header[2] && 0x04 == header[3] {
        Ok(FileType::Zip)
    } else if header.len() > 257 + 10
        && b'u' == header[257] && b's' == header[258]
        && b't' == header[259] && b'a' == header[260]
        && b'r' == header[261]
        && (
            (0 == header[262] && b'0' == header[263] && b'0' == header[264]) ||
            (b' ' == header[262] && b' ' == header[263] && 0 == header[264])
        ) {
        Ok(FileType::Tar)
    } else if header.len() > 8
        && b'!' == header[0] && b'<' == header[1]
        && b'a' == header[2] && b'r' == header[3]
        && b'c' == header[4] && b'h' == header[5]
        && b'>' == header[6] && b'\n' == header[7] {
        Ok(FileType::Ar)
    } else if header.len() > 40
        && b'B' == header[0] && b'Z' == header[1]
        && b'h' == header[2] && 0x31 == header[3]
        && 0x41 == header[4] && 0x59 == header[5]
        && 0x26 == header[6] {
        Ok(FileType::BZip2)
    } else if header.len() > 6
        && 0xfd == header[0] && b'7' == header[1]
        && b'z' == header[2] && b'X' == header[3]
        && b'Z' == header[4] && 0 == header[5] {
        Ok(FileType::Xz)
    } else if is_probably_tar(header) {
        Ok(FileType::Tar)
    } else {
        Ok(FileType::Other)
    }
}

trait Tee: io::BufRead {
    fn reset(&mut self) -> io::Result<()>;
    fn len_and_reset(&mut self) -> io::Result<u64>;
}

struct TempFileTee {
    tmp: io::BufReader<fs::File>,
}

impl TempFileTee {
    fn new<U: io::Read>(mut from: U) -> io::Result<TempFileTee> {
        let mut tmp = tempfile()?;

        {
            let mut reader = io::BufReader::new(from);
            let mut writer = io::BufWriter::new(&tmp);
            io::copy(&mut reader, &mut writer)?;
        }

        tmp.seek(BEGINNING)?;

        Ok(TempFileTee {
            tmp: io::BufReader::new(tmp),
        })
    }
}

const BEGINNING: io::SeekFrom = io::SeekFrom::Start(0);
const END: io::SeekFrom = io::SeekFrom::End(0);

impl Tee for TempFileTee {
    fn reset(&mut self) -> io::Result<()> {
        self.tmp.seek(BEGINNING).map(|_| ())
    }

    fn len_and_reset(&mut self) -> io::Result<u64> {
        let len = self.tmp.seek(END)?;
        self.reset()?;
        Ok(len)
    }
}

// Look, I didn't want to implement these. I wanted to return the implementation.
// But I couldn't make it compile, and I might care enough eventually.
impl io::Read for TempFileTee {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.tmp.read(buf)
    }
}

impl io::BufRead for TempFileTee {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        self.tmp.fill_buf()
    }

    fn consume(&mut self, amt: usize) {
        self.tmp.consume(amt)
    }
}

struct BufReaderTee<T> {
    fp: Box<T>,
}

impl<U: io::Read> BufReaderTee<io::BufReader<U>> {
    fn new(from: U) -> Self {
        BufReaderTee {
            fp: Box::new(io::BufReader::new(from))
        }
    }
}

impl<T> Tee for BufReaderTee<T>
    where T: io::BufRead + io::Seek + 'static
{
    fn reset(&mut self) -> io::Result<()> {
        self.fp.seek(io::SeekFrom::Start(0)).map(|_|())
    }

    fn len_and_reset(&mut self) -> io::Result<u64> {
        let len = self.fp.seek(END)?;
        self.reset()?;
        Ok(len)
    }
}

impl<T> io::Read for BufReaderTee<T>
    where T: io::Read {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.fp.read(buf)
    }
}

impl<T> io::BufRead for BufReaderTee<T>
    where T: io::BufRead {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        self.fp.fill_buf()
    }

    fn consume(&mut self, amt: usize) {
        self.fp.consume(amt)
    }
}


fn count_bytes<'a, R: io::Read>(mut fd: &mut R) -> io::Result<u64> {
    let mut block = [0u8; 4096];
    let mut count = 0u64;
    loop {
        let found = fd.read(&mut block[..])?;
        if 0 == found {
            break;
        }
        count += found as u64;
    }
    Ok(count)
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

fn unpack_or_die<'a>(mut fd: &mut Box<Tee>, output: &OutputTo) -> io::Result<()> {
    let identity = identify(&mut fd, output)?;
    output.log(2, &format!("identified as {}", identity))?;
    match identity {
        FileType::GZip => {
            unpack(Box::new(TempFileTee::new(gzip::Decoder::new(fd)?)?), output)
        },
        FileType::Xz => {
            unpack(Box::new(TempFileTee::new(xz2::bufread::XzDecoder::new(fd))?), output)
        },
        FileType::Ar if output.path.value.ends_with(".deb") => {
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
                let new_output = output.with_path(entry.path()?.to_str().expect("valid utf-8"));
                unpack(Box::new(TempFileTee::new(entry)?), &new_output)?;
            }
            Ok(())
        },
        FileType::Zip => {
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

        // TODO: unimplemented!()
        _ => {
            Err(io::Error::new(io::ErrorKind::Other, Rewind {}))
        },
    }
}

enum FormatErrorType {
    Rewind,
    Other,
}

fn is_format_error(e: &io::Error) -> Option<FormatErrorType> {
    if io::ErrorKind::Other == e.kind() {
        if let Some(ref obj) = e.get_ref() {
            if obj.is::<Rewind>() {
                return Some(FormatErrorType::Rewind);
            }
        }
    }

    panic!("don't know if {:?} / {:?} is a format error", e, e.get_ref())
}

fn unpack(mut fd: Box<Tee>, output: &OutputTo) -> io::Result<()> {
    let res = unpack_or_die(&mut fd, output);
    if let Err(ref raw_error) = res {
        if let Some(specific) = is_format_error(raw_error) {
            match specific {
                FormatErrorType::Other => {
                    output.log(1, &format!(
                        "thought we could unpack '{:?}' but we couldn't: {}",
                        output.path, raw_error));
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
                    .arg(Arg::with_name("INPUT")
                         .required(true)
                        .help("File(s) to process")
                         .multiple(true))
                    .get_matches();

    let options = Options {
        list: matches.is_present("list"),
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

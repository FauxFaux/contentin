extern crate ar;
extern crate clap;
extern crate libflate;
extern crate tar;
extern crate tempfile;
extern crate time as crates_time;
extern crate xz2;
extern crate zip;

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

#[derive(Debug)]
struct Node<T> {
    next: Option<Rc<Node<T>>>,
    value: T,
}

impl<T> Node<T> {
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
}

struct OutputTo {
    path: Rc<Node<String>>,
    size: u64,
    atime: u64,
    mtime: u64,
    ctime: u64,
    btime: u64,
}

impl OutputTo {
    fn warn(&self, msg: String) {
        println!("TODO: {}", msg);
    }

    fn raw<F: io::Read>(&self, file: F) -> io::Result<()> {
        unimplemented!();
    }
}

impl OutputTo {
    fn from_file(path: &str, fd: &fs::File) -> io::Result<OutputTo> {
        let meta = fd.metadata()?;
        Ok(OutputTo {
            path: Node::head(path.to_string()),
            size: meta.len(),
            atime: meta.accessed().map(simple_time_sys)?,
            mtime: meta.modified().map(simple_time_sys)?,
            ctime: simple_time_ctime(&meta),
            btime: meta.created().map(simple_time_sys)?,
        })
    }

    fn with_path(&self, path: String) -> OutputTo {
        OutputTo {
            path: Node::plus(&self.path, path),
            size: 0,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Status {
    Done,
    Rewind,
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

fn identify<'a>(fd: &mut Box<io::BufRead + 'a>) -> io::Result<FileType> {
    let header = fd.fill_buf()?;
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

struct BoxReader<'a> {
    fd: Box<io::BufRead + 'a>
}

impl<'a> io::Read for BoxReader<'a> {
    fn read(&mut self, mut buf: &mut [u8]) -> io::Result<usize> {
        self.fd.read(&mut buf)
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

fn unpack_or_raw<'a, F>(
    fd: Box<io::BufRead + 'a>,
    output: &OutputTo,
    fun: F) -> io::Result<Status>
where F: FnOnce(Box<io::BufRead + 'a>) -> io::Result<Box<io::BufRead + 'a>>
{
    match fun(fd) {
        Ok(inner) => unpack(inner, output),
        Err(e) => {
            output.warn(format!(
                    "thought we could unpack '{:?}' but we couldn't: {}",
                    output.path, e));
            Ok(Status::Rewind)
        }
    }
}

fn unpack<'a>(mut fd: Box<io::BufRead + 'a>, output: &OutputTo) -> io::Result<Status> {
    match identify(&mut fd)? {
        FileType::GZip => {
            unpack_or_raw(fd, output,
                |fd| gzip::Decoder::new(fd).map(
                    |dec| Box::new(io::BufReader::new(dec)) as Box<io::BufRead>))
        },
        FileType::Xz => {
            unpack_or_raw(fd, output,
                |fd| Ok(Box::new(io::BufReader::new(xz2::bufread::XzDecoder::new(fd)))))
        },
        FileType::Ar if output.path.value.ends_with(".deb") => {
            let mut decoder = ar::Archive::new(fd);
            while let Some(entry) = decoder.next_entry() {
                let entry = entry?;
                let new_output = output.with_path(entry.header().identifier().to_string());
                unpack(Box::new(io::BufReader::new(entry)), &new_output)?;
            }
            Ok(Status::Done)
        },
        FileType::Tar => {
            let mut decoder = tar::Archive::new(fd);
            for entry in decoder.entries()? {
                let entry = entry?;
                let new_output = output.with_path(entry.path()?.to_str().expect("valid utf-8").to_string());
                unpack(Box::new(io::BufReader::new(entry)), &new_output)?;
            }
            Ok(Status::Done)
        },
        FileType::Zip => {
            let mut temp = tempfile()?;
            io::copy(&mut BoxReader { fd }, &mut temp)?;
            let mut zip = zip::ZipArchive::new(temp)?;

            for i in 0..zip.len() {
                let file_result = {
                    let entry = zip.by_index(i)?;
                    let mut new_output = output.with_path((entry.name()).to_string());
                    new_output.size = entry.size();
                    new_output.mtime = simple_time_tm(entry.last_modified());
                    let reader = Box::new(io::BufReader::new(entry));
                    unpack(reader, &new_output)?
                };
                match file_result {
                    Status::Done => (),
                    Status::Rewind => {
                        // TODO: update output? ? handling? ???
                        output.raw(zip.by_index(i)?)?
                    },
                };
            }

            Ok(Status::Done)
        },
        other => {
            println!("{:?}: {:?} {}", output.path, other, count_bytes(&mut BoxReader { fd })?);
            Ok(Status::Done)
        },
    }
}

fn main() {
    let matches = App::new("contentin")
                    .arg(Arg::with_name("to-tar")
                         .long("to-tar")
                         .help("emit a tar file")
                         .required(true))
                    .arg(Arg::with_name("INPUT")
                         .required(true)
                         .multiple(true))
                    .get_matches();
    for path in matches.values_of("INPUT").unwrap() {
        let file = fs::File::open(path).unwrap();
        let output = OutputTo::from_file(
            path,
            &file
        ).expect("input file metadata");
        let read = io::BufReader::new(file);
        assert_eq!(Status::Done, unpack(Box::new(read), &output).unwrap());
    }
}


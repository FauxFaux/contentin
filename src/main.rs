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
        writeln!(io::stderr(), "TODO: {}", msg).expect("stderr");
    }

    fn raw<F: io::Read>(&self, mut file: &mut F) -> io::Result<()> {
        let stdout = io::stdout();
        let mut stdout = stdout.lock();
        writeln!(stdout, "{:?} {} {} {} {} {}",
            self.path.to_vec(),
            self.size,
            self.atime, self.mtime, self.ctime, self.btime
        )?;

        io::copy(&mut file, &mut stdout).and_then(move |written|
            if written != self.size {
                Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    format!("expecting to write {} but wrote {}", self.size, written)))
            } else {
                Ok(())
            })
    }

    fn from_file(path: &str, fd: &fs::File) -> io::Result<OutputTo> {
        let meta = fd.metadata()?;
        Ok(OutputTo {
            path: Node::head(path.to_string()),
            size: meta.len(),
            atime: meta.accessed().map(simple_time_sys)?,
            mtime: meta.modified().map(simple_time_sys)?,
            ctime: simple_time_ctime(&meta),
            btime: simple_time_btime(&meta)?,
        })
    }

    fn with_path(&self, path: &str) -> OutputTo {
        OutputTo {
            path: Node::plus(&self.path, path.to_string()),
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

fn identify<'a, T: io::BufRead + ?Sized>(fd: &mut T) -> io::Result<FileType> {
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

trait Tee {
    fn normal(&mut self) -> &mut io::BufRead;
    fn abort(&mut self) -> io::Result<&io::BufRead>;
}

struct TempFileTee<'a> {
    fp: Box<io::BufRead + 'a>,
    tmp: fs::File,
}

impl<'a> TempFileTee<'a> {
    fn new<U: 'a + io::Read>(from: U) -> io::Result<TempFileTee<'a>> {
        Ok(TempFileTee {
            fp: Box::new(io::BufReader::new(from)),
            tmp: tempfile()?
        })
    }
}

impl<'a> Tee for TempFileTee<'a> {
    fn normal(&mut self) -> &mut io::BufRead {
        &mut self.fp
    }
    fn abort(&mut self) -> io::Result<&io::BufRead> {
        unimplemented!();
    }
}

struct BufReaderTee<T> {
    fp: Box<T>,
}

impl<T> BufReaderTee<T> {
    // TODO: return type?
    fn new<U: io::Read>(from: U) -> BufReaderTee<io::BufReader<U>> {
        BufReaderTee {
            fp: Box::new(io::BufReader::new(from))
        }
    }
}

impl<T> Tee for BufReaderTee<T>
    where T: io::BufRead + io::Seek
{
    fn normal(&mut self) -> &mut io::BufRead {
        &mut self.fp
    }

    fn abort(&mut self) -> io::Result<&io::BufRead> {
        self.fp.seek(io::SeekFrom::Start(0))?;
        Ok(self.normal())
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
//
//fn unpack_or_raw<F>(
//    fd: Box<Tee>,
//    output: &OutputTo,
//    fun: F) -> io::Result<Status>
//where F: FnOnce(Box<Tee>) -> io::Result<Box<Tee>>
//{
//    match fun(fd) {
//        Ok(inner) => unpack(inner.normal(), output),
//        Err(e) => {
//            output.warn(format!(
//                    "thought we could unpack '{:?}' but we couldn't: {}",
//                    output.path, e));
//            Ok(Status::Rewind)
//        }
//    }
//}

fn unpack<'a>(mut fd: Box<Tee + 'a>, output: &OutputTo) -> io::Result<Status> {
    match identify(fd.normal())? {
//        FileType::GZip => {
//            unpack_or_raw(fd, output,
//                |fd| gzip::Decoder::new(fd).map(
//                    |dec| Box::new(io::BufReader::new(dec)) as Box<Tee>))
//        },
//        FileType::Xz => {
//            unpack_or_raw(fd, output,
//                |fd| Ok(Box::new(io::BufReader::new(xz2::bufread::XzDecoder::new(fd)))))
//        },
        FileType::Ar if output.path.value.ends_with(".deb") => {
            let mut decoder = ar::Archive::new(fd.normal());
            while let Some(entry) = decoder.next_entry() {
                let entry = entry?;
                let new_output = output.with_path(entry.header().identifier());
                unpack(Box::new(TempFileTee::new(entry)?), &new_output)?;
            }
            Ok(Status::Done)
        },
        FileType::Tar => {
            let mut decoder = tar::Archive::new(fd.normal());
            for entry in decoder.entries()? {
                let entry = entry?;
                let new_output = output.with_path(entry.path()?.to_str().expect("valid utf-8"));
                unpack(Box::new(TempFileTee::new(entry)?), &new_output)?;
            }
            Ok(Status::Done)
        },
        FileType::Zip => {
            let mut temp = tempfile()?;
            io::copy(fd.normal(), &mut temp)?;
            let mut zip = zip::ZipArchive::new(temp)?;

            for i in 0..zip.len() {

                // well, this has gone rather poorly
                let mut new_output = output.with_path(zip.by_index(i)?.name());

                let file_result = {
                    let entry = zip.by_index(i)?;

                    new_output.size = entry.size();
                    new_output.mtime = simple_time_tm(entry.last_modified());
                    let reader = Box::new(TempFileTee::new(entry)?);
                    unpack(reader, &new_output)?
                };
                match file_result {
                    Status::Done => (),
                    Status::Rewind => {
                        // TODO: update output? ? handling? ???
                        new_output.raw(&mut zip.by_index(i)?)?
                    },
                };
            }

            Ok(Status::Done)
        },

        // TODO: unimplemented!()
        _ => {
            Ok(Status::Rewind)
        },
    }
}

fn output_raw(file: &mut fs::File, output: &OutputTo) -> io::Result<()> {
    file.seek(io::SeekFrom::Start(0))?;
    output.raw(file)?;
    Ok(())
}

fn process_real_path(path: &str) -> io::Result<()> {
    let mut file = fs::File::open(path).unwrap();
    let output = OutputTo::from_file(
        path,
        &file
    )?;

    let attempt = {
//        let read = BufReaderTee::new::<BufReaderTee<io::BufReader<fs::File>>>(file);
        let read = TempFileTee::new(&file)?;
        unpack(Box::new(read), &output)
    };

    match attempt {
        Ok(Status::Done) => Ok(()),
        Ok(Status::Rewind) => output_raw(&mut file, &output),
        Err(e) => Err(e)
    }
}

fn real_main() -> u8 {

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
        if let Err(e) = process_real_path(path) {
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

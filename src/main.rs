extern crate ar;
extern crate clap;
extern crate libflate;
extern crate tar;
extern crate xz2;
extern crate zip;

use std::fs;
use std::io;
use std::vec::Vec;

use clap::{Arg, App};

use libflate::gzip;

struct OutputTo {
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
    } else if header.len() > 3
        && b'B' == header[0] && b'Z' == header[1] {
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

fn plus<T: Clone>(vec: &Vec<T>, thing: T) -> Vec<T> {
    let mut ret = vec.clone();
    ret.push(thing);
    ret
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

fn unpack<'a>(mut fd: Box<io::BufRead + 'a>, path: Vec<String>, output: &OutputTo) -> io::Result<()> {
    match identify(&mut fd)? {
        FileType::GZip => {
            let decoder = gzip::Decoder::new(fd)?;
            unpack(Box::new(io::BufReader::new(decoder)), path, output)
        },
        FileType::Ar if path.last().unwrap().ends_with(".deb") => {
            let mut decoder = ar::Archive::new(fd);
            while let Some(entry) = decoder.next_entry() {
                let entry = entry?;
                let new_path = plus(&path, entry.header().identifier().to_string());
                unpack(Box::new(io::BufReader::new(entry)), new_path, output)?;
            }
            Ok(())
        },
        FileType::Tar => {
            let mut decoder = tar::Archive::new(fd);
            for entry in decoder.entries()? {
                let entry = entry?;
                let new_path = plus(&path, entry.path()?.to_str().expect("valid utf-8").to_string());
                unpack(Box::new(io::BufReader::new(entry)), new_path, output)?;
            }
            Ok(())
        },
        FileType::Xz => {
            unpack(Box::new(io::BufReader::new(xz2::bufread::XzDecoder::new(fd))), path, output)
        },
        FileType::Zip => {
            let mut angry = BoxReader { fd };
            while let Some(mut entry) = zip::read::read_single(&mut angry)? {
                let new_path = plus(&path, (&*entry.name).to_string());
                let reader = entry.get_reader();
                let unpacked = unpack(Box::new(io::BufReader::new(reader)), new_path, output);
                if let Err(e) = unpacked {
                    if e.kind() == io::ErrorKind::InvalidInput {
                        break;
                    } else {
                        return Err(e)
                    };
                }
            }
            let bytes_at_end = count_bytes(&mut angry)?;
            if 0 != bytes_at_end {
                println!("{:?}: discarding trailing bytes: {}", path, bytes_at_end);
            }
            Ok(())
        },
        other => {
            println!("{:?}: {:?} {}", path, other, count_bytes(&mut BoxReader { fd })?);
            Ok(())
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
    let output = OutputTo {};
    for path in matches.values_of("INPUT").unwrap() {
        let file = fs::File::open(path).unwrap();
        let read = io::BufReader::new(file);
        unpack(Box::new(read), vec![path.to_string()], &output).unwrap();
    }
}


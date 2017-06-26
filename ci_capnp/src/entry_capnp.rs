#![allow(unused)]
include!(concat!(env!("OUT_DIR"), "/../entry_capnp.rs"));

use std;
use std::io;

use capnp;

pub enum Ownership {
    Unknown,
    Posix(String, String),
}

#[derive(Clone, Debug)]
pub struct FileEntry {
    pub len: u64,
    pub paths: Vec<String>,
    pub atime: u64,
    pub mtime: u64,
    pub ctime: u64,
    pub btime: u64,
    pub normal_file: bool,
    pub content_follows: bool,
}

struct PeekyRead<'a, R: io::Read + 'a> {
    inner: &'a mut R,
    peeked: Option<u8>,
}

impl<'a, R: io::Read + 'a> io::Read for PeekyRead<'a, R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self.peeked {
            Some(c) => {
                buf[0] = c;
                self.peeked = None;
                Ok(1)
            }
            None => self.inner.read(buf),
        }
    }
}

impl<'a, R: io::Read> PeekyRead<'a, R> {
    fn new(inner: &mut R) -> PeekyRead<R> {
        PeekyRead {
            inner,
            peeked: None,
        }
    }

    fn check_eof(&mut self) -> io::Result<bool> {
        if self.peeked.is_some() {
            // we have something more to return; read won't return 0 bytes
            return Ok(false);
        }

        let mut buf = [0; 1];
        Ok(match self.inner.read(&mut buf)? {
            0 => true,
            1 => {
                self.peeked = Some(buf[0]);
                false
            }
            _ => unreachable!(),
        })
    }
}

pub fn read_entry<'a, R: io::Read>(mut from: &mut R) -> capnp::Result<Option<FileEntry>> {
    let mut from = PeekyRead::new(from);

    if from.check_eof()? {
        return Ok(None);
    }

    let message = capnp::serialize::read_message(&mut from, capnp::message::ReaderOptions::new())?;

    let entry = message.get_root::<entry::Reader>()?;

    if 0x0100C1C1 != entry.get_magic() {
        return Err(capnp::Error::failed(
            "invalid magic after decoding; invalid stream?".to_string(),
        ));
    }

    let entry_paths = entry.get_paths()?;
    let entry_paths_len = entry_paths.len();

    let mut paths = Vec::with_capacity(entry_paths_len as usize);

    for i in 0..entry_paths_len {
        paths.push(entry_paths.get(i)?.to_string());
    }

    Ok(Some(FileEntry {
        len: entry.get_len(),
        atime: entry.get_atime(),
        mtime: entry.get_mtime(),
        ctime: entry.get_ctime(),
        btime: entry.get_btime(),
        paths,
        normal_file: match entry.get_type().which()? {
            entry::type_::Which::Normal(()) => true,
            _ => false,
        },
        content_follows: match entry.get_content().which()? {
            entry::content::Which::Follows(()) => true,
            _ => false,
        },
    }))
}

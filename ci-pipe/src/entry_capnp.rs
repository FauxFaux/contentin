
#![allow(unused)]
include!(concat!(env!("OUT_DIR"), "/../entry_capnp.rs"));

use std;
use std::io;

use capnp;

pub enum Ownership {
    Unknown,
    Posix(String, String),
}

#[derive(Copy, Clone, Debug)]
pub struct FileEntry {
    len: u64,
    atime: u64,
    mtime: u64,
    ctime: u64,
    btime: u64,

}

pub fn read_entry<'a, R: io::Read>(mut from: &mut R) -> capnp::Result<Option<FileEntry>> {
    let message = capnp::serialize::read_message(&mut from, capnp::message::ReaderOptions::new())?;

    let entry = message.get_root::<entry::Reader>()?;

    Ok(Some(FileEntry {
        len: entry.get_len(),
        atime: entry.get_atime(),
        mtime: entry.get_mtime(),
        ctime: entry.get_ctime(),
        btime: entry.get_btime(),
    }))
}
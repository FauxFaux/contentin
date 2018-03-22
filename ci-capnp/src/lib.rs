extern crate capnp;
extern crate iowrap;

use std::collections::HashMap;

mod entry_capnp;
mod read;
mod write;

/// The actual generated module for `Entry`:
pub use entry_capnp::entry;
pub use read::read_entry;
pub use write::write_meta;

#[derive(Clone, Debug)]
pub struct PosixEntity {
    pub id: u32,
    pub name: String,
}

#[derive(Clone, Debug)]
pub enum Ownership {
    Unknown,
    Posix {
        user: Option<PosixEntity>,
        group: Option<PosixEntity>,
        mode: u32,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum ItemType {
    // TODO: Magic value "Unknown", or an Option, or..?
    Unknown,
    RegularFile,
    Directory,
    Fifo,
    Socket,
    /// A symlink, with its destination.
    SymbolicLink(String),
    /// A hardlink, with its destination.
    HardLink(String),
    /// A 'c' device.
    CharacterDevice {
        major: u32,
        minor: u32,
    },
    /// A 'b' device.
    BlockDevice {
        major: u32,
        minor: u32,
    },
}

#[derive(Clone, Debug)]
pub enum Container {
    Unrecognised,
    Included,
    OpenError(String),
    ReadError(String),
}

#[derive(Clone, Debug)]
pub struct FileEntry {
    pub len: u64,
    pub paths: Vec<String>,
    pub content_follows: bool,
    pub meta: Meta,
}

#[derive(Clone, Debug)]
pub struct Meta {
    pub atime: u64,
    pub mtime: u64,
    pub ctime: u64,
    pub btime: u64,
    pub ownership: Ownership,
    pub item_type: ItemType,
    pub container: Container,
    pub xattrs: HashMap<String, Vec<u8>>,
}

#![allow(unused)]
include!(concat!(env!("OUT_DIR"), "/../entry_capnp.rs"));

use std;
use std::io;

use capnp;
use peeky_read::PeekyRead;

#[derive(Clone, Debug)]
pub struct PosixEntity {
    pub id: u32,
    pub name: String,
}

#[derive(Clone, Debug)]
pub enum Ownership {
    Unknown,
    Posix{ user: PosixEntity, group: PosixEntity, mode: u32 },
}

#[derive(Clone, Debug)]
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
    CharacterDevice { major: u32, minor: u32 },
    /// A 'b' device.
    BlockDevice { major: u32, minor: u32 },
}

#[derive(Clone, Debug)]
pub struct FileEntry {
    pub len: u64,
    pub paths: Vec<String>,
    pub atime: u64,
    pub mtime: u64,
    pub ctime: u64,
    pub btime: u64,
    pub ownership: Ownership,
    pub item_type: ItemType,
    pub content_follows: bool,
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
        paths,
        atime: entry.get_atime(),
        mtime: entry.get_mtime(),
        ctime: entry.get_ctime(),
        btime: entry.get_btime(),
        ownership: match entry.get_ownership().which()? {
            entry::ownership::Which::Unknown(()) => Ownership::Unknown,
            entry::ownership::Which::Posix(tuple) => {
                let user = tuple.get_user()?;
                let group = tuple.get_group()?;
                Ownership::Posix {
                    user: PosixEntity {
                        id: user.get_id(),
                        name: user.get_name()?.to_string(),
                    },
                    group: PosixEntity {
                        id: group.get_id(),
                        name: group.get_name()?.to_string(),
                    },
                    mode: tuple.get_mode(),
                }
            },
        },
        item_type: match entry.get_type().which()? {
            entry::type_::Which::Normal(()) => ItemType::RegularFile,
            entry::type_::Which::Directory(()) => ItemType::Directory,
            entry::type_::Which::Fifo(()) => ItemType::Fifo,
            entry::type_::Which::Socket(()) => ItemType::Socket,
            entry::type_::Which::SoftLinkTo(dest) => ItemType::SymbolicLink(dest?.to_string()),
            entry::type_::Which::HardLinkTo(dest) => ItemType::HardLink(dest?.to_string()),
            entry::type_::Which::CharDevice(numbers) => {
                let numbers = numbers?;
                ItemType::CharacterDevice {
                    major: numbers.get_major(),
                    minor: numbers.get_minor(),
                }
            }
            entry::type_::Which::BlockDevice(numbers) => {
                let numbers = numbers?;
                ItemType::BlockDevice {
                    major: numbers.get_major(),
                    minor: numbers.get_minor(),
                }
            }
            // _ => ItemType::Unknown,
        },
        content_follows: match entry.get_content().which()? {
            entry::content::Which::Follows(()) => true,
            _ => false,
        },
    }))
}

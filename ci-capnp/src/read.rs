use std::io;

use std::collections::HashMap;

use capnp;
use iowrap::Eof;

use super::*;

pub fn read_entry<'a, R: io::Read>(from: R) -> capnp::Result<Option<FileEntry>> {
    let mut from = Eof::new(from);

    if from.eof()? {
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

    let entry_xattrs = entry.get_xattrs()?;
    let entry_xattrs_len = entry_xattrs.len();

    let mut xattrs = HashMap::with_capacity(entry_paths_len as usize);

    for i in 0..entry_xattrs_len {
        let xattr = entry_xattrs.get(i);
        xattrs.insert(xattr.get_name()?.to_string(), xattr.get_value()?.to_vec());
    }
    let meta = Meta {
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
                    user: Some(PosixEntity {
                        id: u64::from(user.get_id()),
                        name: user.get_name()?.to_string(),
                    }),
                    group: Some(PosixEntity {
                        id: u64::from(group.get_id()),
                        name: group.get_name()?.to_string(),
                    }),
                    mode: tuple.get_mode(),
                }
            }
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
            } // _ => ItemType::Unknown,
        },
        container: match entry.get_container().which()? {
            entry::container::Which::Unrecognised(()) => Container::Unrecognised,
            entry::container::Which::Included(()) => Container::Included,
            entry::container::Which::OpenError(msg) => Container::OpenError(msg?.to_string()),
            entry::container::Which::ReadError(msg) => Container::ReadError(msg?.to_string()),
        },
        xattrs,
    };

    Ok(Some(FileEntry {
        len: entry.get_len(),
        paths,
        meta,
        content_follows: match entry.get_content().which()? {
            entry::content::Which::Follows(()) => true,
            _ => false,
        },
    }))
}

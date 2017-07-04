
use std;
use std::io;

use capnp;
use ci_capnp::entry;
use ci_capnp::Ownership;

pub fn write_capnp<W: io::Write>(
    to: &mut W,
    current: &::EntryBuilder,
    content_output: bool,
    size: u64,
) -> io::Result<()> {

    let mut message = capnp::message::Builder::new_default();
    {
        let mut entry = message.init_root::<entry::Builder>();
        entry.set_magic(0x0100C1C1);
        entry.set_len(size);

        {
            let mut paths = entry.borrow().init_paths(current.depth + 1);
            for (i, path) in current.path.iter().enumerate() {
                assert!(i < std::u32::MAX as usize);
                paths.set(i as u32, path.as_str());
            }
        }

        entry.set_atime(current.meta.atime);
        entry.set_mtime(current.meta.mtime);
        entry.set_ctime(current.meta.ctime);
        entry.set_btime(current.meta.btime);

        match current.meta.ownership {
            Ownership::Posix {
                ref user,
                ref group,
                mode,
            } => {
                let mut posix = entry.borrow().get_ownership().init_posix();
                if let &Some(ref user) = user {
                    let mut out = posix.borrow().init_user();
                    out.set_id(user.id);
                    out.set_name(user.name.as_str());
                }
                if let &Some(ref group) = group {
                    let mut out = posix.borrow().init_group();
                    out.set_id(group.id);
                    out.set_name(group.name.as_str());
                }

                posix.set_mode(mode);
            }

            Ownership::Unknown => {
                entry.borrow().get_ownership().set_unknown(());
            }
        }

        {
            let mut type_ = entry.borrow().get_type();

            use ci_capnp::ItemType::*;
            match current.meta.item_type {
                Unknown => {
                    match size {
                        0 => type_.set_directory(()),
                        _ => type_.set_normal(()),
                    }
                }
                RegularFile => type_.set_normal(()),
                Directory => type_.set_directory(()),
                Fifo => type_.set_fifo(()),
                Socket => type_.set_socket(()),
                SymbolicLink(ref dest) => type_.set_soft_link_to(dest.as_str()),
                HardLink(ref dest) => type_.set_hard_link_to(dest.as_str()),
                CharacterDevice { major, minor } => {
                    let mut dev = type_.borrow().init_char_device();
                    dev.set_major(major);
                    dev.set_minor(minor);
                }
                BlockDevice { major, minor } => {
                    let mut dev = type_.borrow().init_block_device();
                    dev.set_major(major);
                    dev.set_minor(minor);
                }
            }
        }

        {
            let mut content = entry.borrow().get_content();
            if content_output {
                content.set_follows(());
            } else {
                content.set_absent(());
            }
        }
    }
    capnp::serialize::write_message(to, &message)
}


use std;
use std::io;

use capnp;
use ci_capnp::entry;

pub fn write_capnp<W: io::Write>(
    to: &mut W,
    current: &::FileDetails,
    content_output: &::ContentOutput,
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

        entry.set_atime(current.atime);
        entry.set_mtime(current.mtime);
        entry.set_ctime(current.ctime);
        entry.set_btime(current.btime);

        {
            let mut posix = entry.borrow().get_ownership().init_posix();
            {
                let mut user = posix.borrow().init_user();
                user.set_id(current.uid);
                user.set_name(current.user_name.as_str());
            }
            {
                let mut group = posix.borrow().init_group();
                group.set_id(current.gid);
                group.set_name(current.group_name.as_str());
            }

            posix.set_mode(current.mode);
        }

        {
            let mut type_ = entry.borrow().get_type();

            use ci_capnp::ItemType::*;
            match current.item_type {
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
            match *content_output {
                ::ContentOutput::None => {
                    content.set_absent(());
                }
                ::ContentOutput::Raw => {
                    content.set_follows(());
                }
            }
        }
    }
    capnp::serialize::write_message(to, &message)
}

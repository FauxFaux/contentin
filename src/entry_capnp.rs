
#![allow(unused)]
include!(concat!(env!("OUT_DIR"), "/entry_capnp.rs"));

use std;
use std::io;

use capnp;

pub fn write_capnp<W: io::Write>(
    to: &mut W,
    current: &::FileDetails,
    content_output: &::ContentOutput,
    size: u64) -> io::Result<()> {

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

            // TODO: mode
        }

        {
            let mut type_ = entry.borrow().get_type();

            // TODO: other file types / proper tracking of this
            match size {
                0 => type_.set_directory(()),
                _ => type_.set_normal(()),
            }
        }

        {
            let mut content = entry.borrow().get_content();
            match *content_output {
                ::ContentOutput::None => {
                    content.set_absent(());
                },
                ::ContentOutput::Raw => {
                    content.set_follows(());
                },
                ::ContentOutput::Grep(_) | ::ContentOutput::ToCommand(_) => {
                    unreachable!();
                },
            }
        }
    }
    capnp::serialize::write_message(to, &message)
}

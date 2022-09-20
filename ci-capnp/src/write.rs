use anyhow::Result;
use std;
use std::convert::TryInto;

use crate::entry;
use crate::Ownership;

pub fn write_meta(meta: &crate::Meta, entry: &mut entry::Builder, size: u64) -> Result<()> {
    entry.set_atime(meta.atime);
    entry.set_mtime(meta.mtime);
    entry.set_ctime(meta.ctime);
    entry.set_btime(meta.btime);
    match meta.ownership {
        Ownership::Posix {
            ref user,
            ref group,
            mode,
        } => {
            let mut posix = entry.reborrow().get_ownership().init_posix();
            if let &Some(ref user) = user {
                let mut out = posix.reborrow().init_user();
                out.set_id(user.id.try_into()?);
                out.set_name(user.name.as_str());
            }
            if let &Some(ref group) = group {
                let mut out = posix.reborrow().init_group();
                out.set_id(group.id.try_into()?);
                out.set_name(group.name.as_str());
            }

            posix.set_mode(mode);
        }

        Ownership::Unknown => {
            entry.reborrow().get_ownership().set_unknown(());
        }
    }

    {
        let mut type_ = entry.reborrow().get_type();

        use crate::ItemType::*;
        match meta.item_type {
            Unknown => match size {
                0 => type_.set_directory(()),
                _ => type_.set_normal(()),
            },
            RegularFile => type_.set_normal(()),
            Directory => type_.set_directory(()),
            Fifo => type_.set_fifo(()),
            Socket => type_.set_socket(()),
            SymbolicLink(ref dest) => type_.set_soft_link_to(dest.as_str()),
            HardLink(ref dest) => type_.set_hard_link_to(dest.as_str()),
            CharacterDevice { major, minor } => {
                let mut dev = type_.reborrow().init_char_device();
                dev.set_major(major);
                dev.set_minor(minor);
            }
            BlockDevice { major, minor } => {
                let mut dev = type_.reborrow().init_block_device();
                dev.set_major(major);
                dev.set_minor(minor);
            }
        }
    }

    {
        assert!(meta.xattrs.len() <= std::u32::MAX as usize);
        let mut xattrs = entry.reborrow().init_xattrs(meta.xattrs.len() as u32);
        let mut names: Vec<&String> = meta.xattrs.keys().collect();
        names.sort();
        for (i, name) in names.into_iter().enumerate() {
            let mut row = xattrs.reborrow().get(i as u32);
            row.set_name(name);
            row.set_value(&meta.xattrs[name]);
        }
    }

    Ok(())
}

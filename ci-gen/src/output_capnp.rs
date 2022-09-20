use std;
use std::io;

use capnp;
use ci_capnp;
use ci_capnp::entry;

pub fn write_capnp<W: io::Write>(
    to: &mut W,
    current: &crate::EntryBuilder,
    content_output: bool,
    size: u64,
) -> io::Result<()> {
    let mut message = capnp::message::Builder::new_default();
    {
        let mut entry = message.init_root::<entry::Builder>();
        entry.set_magic(0x0100C1C1);
        entry.set_len(size);

        {
            let mut paths = entry.reborrow().init_paths(current.depth + 1);
            for (i, path) in current.path.iter().enumerate() {
                assert!(i < std::u32::MAX as usize);
                paths.set(i as u32, path.as_str());
            }
        }

        {
            let mut content = entry.reborrow().get_content();
            if content_output {
                content.set_follows(());
            } else {
                content.set_absent(());
            }
        }

        ci_capnp::write_meta(&current.meta, &mut entry, size);
    }
    capnp::serialize::write_message(to, &message)
}

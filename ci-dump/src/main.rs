extern crate crc;
extern crate ci_capnp;
extern crate chrono;
extern crate clap;

use std::io;

use crc::crc32;

use ci_capnp::FileEntry;

use std::io::Read;

use chrono::TimeZone;

struct WithCrc {
    entry: FileEntry,
    crc: u32,
}

fn main() {
    let matches = clap::App::new("ci-dump").arg(
        clap::Arg::with_name("drop-local-fs-details")
            .long("drop-local-fs-details")
            .help("drop times, ownership, ... from top-level items (i.e. that vary between machines)")
    ).get_matches();

    let drop_local_fs_details = matches.is_present("drop-local-fs-details");

    let input = io::stdin();
    let mut input = &mut input.lock();

    let mut all = Vec::new();

    loop {
        let entry: FileEntry = match ci_capnp::read_entry(input).expect("reading heder") {
            Some(x) => x,
            None => break,
        };

        let mut crc = 0;

        if entry.content_follows {
            let mut limited = input.take(entry.len);
            loop {
                let mut buf = [0u8; 4096];
                let found = limited.read(&mut buf).expect("reading data");
                if 0 == found {
                    break;
                }

                crc = crc32::update(crc, &crc32::CASTAGNOLI_TABLE, &buf[0..found]);
            }
        }

        all.push(WithCrc { entry, crc });
    }

    all.sort_by(|left, right| {
        use std::cmp::min;
        use std::cmp::Ordering;
        let left = &left.entry.paths;
        let right = &right.entry.paths;
        for i in 1..1 + min(left.len(), right.len()) {
            match left[left.len() - i].cmp(&right[right.len() - i]) {
                Ordering::Equal => {}
                other => return other,
            }
        }

        left.len().cmp(&right.len())
    });

    for item in all {
        let entry: FileEntry = item.entry;

        let transients = !(drop_local_fs_details && 1 == entry.paths.len());

        println!(" - paths:");
        for path in entry.paths {
            println!("          - {}", path);
        }

        println!("   type:  {:?}", entry.item_type);

        if 0 != entry.len {
            println!("   wrap:  {:?}", entry.container);
            println!("   data:  {:?}", entry.content_follows);
            println!("   size:  {}", entry.len);
            println!("   crc:   {:08x}", item.crc);
        }

        if transients {
            date("atime", entry.atime);
            date("mtime", entry.mtime);
            date("ctime", entry.ctime);
            date("btime", entry.btime);

            use ci_capnp::Ownership;
            match entry.ownership {
                Ownership::Unknown => {}
                Ownership::Posix { user, group, mode } => {
                    println!("   uid:   {}", user.id);
                    println!("   gid:   {}", group.id);
                    if !user.name.is_empty() {
                        println!("   user:  {}", user.name);
                    }
                    if !group.name.is_empty() {
                        println!("   group: {}", group.name);
                    }

                    println!("   mode:  0o{:04o}", mode);
                }
            }
        }

        if !entry.xattrs.is_empty() {
            println!("   xattrs:");
            let mut keys: Vec<&String> = entry.xattrs.keys().collect();
            keys.sort();
            for key in keys {
                println!("     {}: {:?}", key, entry.xattrs[key]);
            }
        }
    }
}

fn date(whence: &str, nanos: u64) {
    if 0 != nanos {
        println!(
            "   {}: {}",
            whence,
            chrono::Utc.timestamp(
                (nanos / 1_000_000_000) as i64,
                (nanos % 1_000_000_000) as u32,
            )
        );
    }
}

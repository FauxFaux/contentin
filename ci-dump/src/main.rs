extern crate crc;
extern crate ci_capnp;
extern crate chrono;

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
        use std::cmp::Ordering;
        let left = &left.entry.paths;
        let right = &right.entry.paths;
        match left.len().cmp(&right.len()) {
            Ordering::Equal => {}
            other => return other,
        };

        for i in 0..left.len() {
            match left[i].cmp(&right[i]) {
                Ordering::Equal => {}
                other => return other,
            }
        }

        return Ordering::Equal;
    });

    for item in all {
        let entry: FileEntry = item.entry;
        println!(" - paths:");
        for path in entry.paths {
            println!("          - {}", path);
        }

        if 0 != entry.len {
            println!("   size:  {}", entry.len);
        }

        date("atime", entry.atime);
        date("mtime", entry.mtime);
        date("ctime", entry.ctime);
        date("btime", entry.btime);

        if 0 != entry.len {
            println!("   crc:   {:08x}", item.crc);
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

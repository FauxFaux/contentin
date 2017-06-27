extern crate crc;
extern crate ci_capnp;

use std::io;
use std::process;

use ci_capnp::FileEntry;

use std::io::Read;

const PROG: &str = "../target/debug/ci-gen";

#[derive(Debug)]
pub struct TestEntry {
    pub crc: u32,
    pub entry: FileEntry,
}

pub fn entries(name: &str) -> io::Result<Vec<TestEntry>> {
    let mut prog = process::Command::new(PROG)
        .arg("-h")
        .arg("capnp")
        .arg(name)
        .stdin(process::Stdio::null())
        .stdout(process::Stdio::piped())
        .stderr(process::Stdio::inherit())
        .spawn()?;

    let mut res = Vec::new();

    {
        let mut out = prog.stdout.as_mut().unwrap();
        loop {
            let entry: FileEntry = match ci_capnp::read_entry(&mut out).unwrap() {
                Some(x) => x,
                None => break,
            };

            assert!(entry.content_follows);

            let mut limited = out.take(entry.len);
            let mut crc = 0;
            loop {
                let mut buf = [0u8; 4096];
                let found = limited.read(&mut buf)?;
                if 0 == found {
                    break;
                }

                crc = crc::crc32::update(crc, &crc::crc32::CASTAGNOLI_TABLE, &buf[0..found]);
            }

            res.push(TestEntry { crc, entry });
        }
    }

    assert!(prog.wait()?.success());

    res.sort_by(|left, right| {
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

    Ok(res)
}
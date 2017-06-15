extern crate crc;
extern crate ci_capnp;

use std::io;
use std::process;

use std::io::Read;

use ci_capnp::FileEntry;


const PROG: &str = "../target/debug/ci-gen";

#[derive(Debug)]
struct TestEntry {
    crc: u32,
    entry: FileEntry,
}

fn entries(name: &str) -> io::Result<Vec<TestEntry>> {
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

            res.push(TestEntry {
                crc,
                entry
            });
        }
    }

    assert!(prog.wait()?.success());

    res.sort_by(|left, right| {
        use std::cmp::Ordering;
        let left = &left.entry.paths;
        let right = &right.entry.paths;
        match left.len().cmp(&right.len()) {
            Ordering::Equal => {},
            other => return other,
        };

        for i in 0..left.len() {
            match left[i].cmp(&right[i]) {
                Ordering::Equal => {},
                other => return other,
            }
        }

        return Ordering::Equal;
    });

    Ok(res)
}

struct SimpleTest {
    paths: &'static [&'static str],
    normal_file: bool,
    crc: u32,
    len: u64,
}

const SIMPLE_EXPECTATIONS: &[SimpleTest] = &[
    SimpleTest { paths: &["a/"], normal_file: false, crc: 0, len: 0 },
    SimpleTest { paths: &["a/b/"], normal_file: false, crc: 0, len: 0 },
    SimpleTest { paths: &["a/b/c/"], normal_file: false, crc: 0, len: 0 },
    SimpleTest { paths: &["a/bar"], normal_file: true, crc: 0xe3069283, len: 9 },
    SimpleTest { paths: &["foo"], normal_file: true, crc: 0xe3069283, len: 9 },
];

fn dump(actual: &[TestEntry]) {
    for item in actual {
        println!("{} {} {} {:?}", item.entry.normal_file, item.entry.len, item.crc, item.entry.paths);
    }
}

fn check_simple(path: &str, extra_path_component: Option<&str>) {
    let res = entries(path).unwrap();
    if res.len() != SIMPLE_EXPECTATIONS.len() {
        dump(&res);
        panic!("wrong number of entries: {} should be {}", res.len(), SIMPLE_EXPECTATIONS.len());
    }

    for i in 0..SIMPLE_EXPECTATIONS.len() {
        let exp = &SIMPLE_EXPECTATIONS[i];
        let act = &res[i];
        let mut exp_paths = exp.paths.iter().map(|x| x.to_string()).collect::<Vec<String>>();
        if let Some(component) = extra_path_component {
            exp_paths.push(component.to_string());
        }
        exp_paths.push(path.to_string());
        assert_eq!(exp_paths, act.entry.paths);
        assert_eq!(exp.normal_file, act.entry.normal_file);
        assert_eq!(exp.crc, act.crc, "{:08x} != {:08x}", exp.crc, act.crc);
        assert_eq!(exp.len, act.entry.len);
    }
}

fn check_byte_flip(path: &str, extra_path_component: Option<&str>) {
    let res = entries(path).unwrap();
    dump(&res);
    assert_eq!(2, res.len());
}

fn round_trips(test_path: &str) {
    let entries = entries(test_path).unwrap();
    assert_eq!(1, entries.len());
    assert_eq!(1, entries[0].entry.paths.len());
    assert_eq!(test_path, entries[0].entry.paths[0]);
}

#[test]
fn simple_tar() {
    check_simple("tests/simple.tar", None)
}
#[test]
fn simple_tar_bz2() {
    check_simple("tests/simple.tar.bz2", Some("tests/simple.tar"))
}
#[test]
fn simple_tar_gz() {
    check_simple("tests/simple.tar.gz", Some("tests/simple.tar"))
}
#[test]
fn simple_tar_xz() {
    check_simple("tests/simple.tar.xz", Some("tests/simple.tar"))
}
#[test]
fn simple_zip() {
    check_simple("tests/simple.zip", None)
}

/// tar has no error checking, and file data gets corrupted, so we don't detect this.
#[test]
fn byte_flip_tar() {
    let entries = entries("tests/byte_flip.tar").unwrap();
    assert_eq!(2, entries.len());
    // this crc should be correct
    assert_eq!(2806881067, entries[0].crc);

    // this crc should be wrong...
    assert_eq!(3502720051, entries[1].crc);
}
#[test]
fn byte_flip_tar_bz2() {
    round_trips("tests/byte_flip.tar.bz2")
}

/// GZIP detects the failure, but only at the end, so we rollback and output the whole archive again
#[test]
fn byte_flip_tar_gz() {
    let test_path = "tests/byte_flip.tar.gz";
    let entries = entries(test_path).unwrap();
    assert_eq!(3, entries.len());

    // sorting in test means the archive comes out first (the order is undefined anyway...)
    assert_eq!(1, entries[0].entry.paths.len());
    assert_eq!(test_path, entries[0].entry.paths[0]);

    // this crc should be correct
    assert_eq!(2806881067, entries[1].crc);

    // this crc should be wrong...
    assert_eq!(1897953606, entries[2].crc);
}
#[test]
fn byte_flip_tar_xz() {
    round_trips("tests/byte_flip.tar.xz");
}
#[test]
fn byte_flip_zip() {
    check_byte_flip("tests/byte_flip.zip", None)
}
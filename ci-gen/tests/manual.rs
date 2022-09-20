extern crate ci_capnp;
extern crate crc;

mod entries;
use crate::entries::*;

use ci_capnp::ItemType;

struct SimpleTest {
    paths: &'static [&'static str],
    normal_file: bool,
    crc: u32,
    len: u64,
}

#[cfg_attr(rustfmt, rustfmt_skip)]
const SIMPLE_EXPECTATIONS: &[SimpleTest] = &[
    SimpleTest { paths: &["a/"], normal_file: false, crc: 0, len: 0 },
    SimpleTest { paths: &["a/b/"], normal_file: false, crc: 0, len: 0 },
    SimpleTest { paths: &["a/b/c/"], normal_file: false, crc: 0, len: 0 },
    SimpleTest { paths: &["a/bar"], normal_file: true, crc: 0xe3069283, len: 9 },
    SimpleTest { paths: &["foo"], normal_file: true, crc: 0xe3069283, len: 9 },
];

fn dump(actual: &[TestEntry]) {
    for item in actual {
        println!(
            "{:?} {} {} {:?}",
            item.entry.meta.item_type, item.entry.len, item.crc, item.entry.paths
        );
    }
}

fn check_simple(path: &str, extra_path_component: Option<&str>) {
    let res = entries(path).unwrap();
    if res.len() != SIMPLE_EXPECTATIONS.len() {
        dump(&res);
        panic!(
            "wrong number of entries: {} should be {}",
            res.len(),
            SIMPLE_EXPECTATIONS.len()
        );
    }

    for i in 0..SIMPLE_EXPECTATIONS.len() {
        let exp = &SIMPLE_EXPECTATIONS[i];
        let act = &res[i];
        let mut exp_paths = exp
            .paths
            .iter()
            .map(|x| x.to_string())
            .collect::<Vec<String>>();
        if let Some(component) = extra_path_component {
            exp_paths.push(component.to_string());
        }
        exp_paths.push(path.to_string());
        assert_eq!(exp_paths, act.entry.paths);
        assert_eq!(
            exp.normal_file,
            ItemType::RegularFile == act.entry.meta.item_type
        );
        assert_eq!(exp.crc, act.crc, "{:08x} != {:08x}", exp.crc, act.crc);
        assert_eq!(exp.len, act.entry.len);
    }
}

fn round_trips(test_path: &str) {
    let entries = entries(test_path).unwrap();
    assert_eq!(1, entries.len());
    assert_eq!(1, entries[0].entry.paths.len());
    assert_eq!(test_path, entries[0].entry.paths[0]);
}

#[test]
fn simple_tar() {
    check_simple("tests/examples/simple.tar", None)
}
#[test]
fn simple_tar_bz2() {
    check_simple(
        "tests/examples/simple.tar.bz2",
        Some("tests/examples/simple.tar"),
    )
}
#[test]
fn simple_tar_gz() {
    check_simple(
        "tests/examples/simple.tar.gz",
        Some("tests/examples/simple.tar"),
    )
}
#[test]
fn simple_tar_xz() {
    check_simple(
        "tests/examples/simple.tar.xz",
        Some("tests/examples/simple.tar"),
    )
}
#[test]
fn simple_zip() {
    check_simple("tests/examples/simple.zip", None)
}

/// tar has no error checking, and file data gets corrupted, so we don't detect this.
#[test]
fn byte_flip_tar() {
    let entries = entries("tests/examples/byte_flip.tar").unwrap();
    assert_eq!(2, entries.len());
    // this crc should be correct
    assert_eq!(2806881067, entries[0].crc);

    // this crc should be wrong...
    assert_eq!(3502720051, entries[1].crc);
}
#[test]
fn byte_flip_tar_bz2() {
    round_trips("tests/examples/byte_flip.tar.bz2")
}

/// GZIP detects the failure, but only at the end, so we rollback and output the whole archive again
#[test]
fn byte_flip_tar_gz() {
    let test_path = "tests/examples/byte_flip.tar.gz";
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

/// ZIP fails exactly like gzip, much to my chagrin.
#[test]
fn byte_flip_zip() {
    let test_path = "tests/examples/byte_flip.zip";
    let entries = entries(test_path).unwrap();
    assert_eq!(3, entries.len());

    // sorting in test means the archive comes out first (the order is undefined anyway...)
    assert_eq!(1, entries[0].entry.paths.len());
    assert_eq!(test_path, entries[0].entry.paths[0]);

    // this crc should be correct
    assert_eq!(2806881067, entries[1].crc);

    // this crc should be wrong...
    assert_eq!(3055781517, entries[2].crc);
}

#[test]
fn byte_flip_tar_xz() {
    round_trips("tests/examples/byte_flip.tar.xz");
}

#[test]
fn zip_cd_files() {
    round_trips("tests/real/broken_cd.zip");
    round_trips("tests/real/incons-cdoffset.zip");
}

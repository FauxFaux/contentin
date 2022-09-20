extern crate ci_capnp;
extern crate crc;
extern crate tempdir;

use std::process;

mod entries;
use crate::entries::*;

use ci_capnp::ItemType;

#[test]
fn special_files() {
    let dir = tempdir::TempDir::new("ci-special-files").unwrap();
    let mut fifo = dir.path().to_path_buf().clone();
    fifo.push("fifo");
    let fifo = fifo.to_str().expect("utf-8");

    assert!(process::Command::new("/usr/bin/mkfifo")
        .arg(fifo)
        .status()
        .expect("mkfifo")
        .success());
    let output = entries(fifo).expect("entries");
    assert_eq!(1, output.len());
    let entry = &output[0];
    assert_eq!(0, entry.entry.len);
    assert_eq!(ItemType::Fifo, entry.entry.meta.item_type);
}

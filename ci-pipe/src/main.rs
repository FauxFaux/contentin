extern crate capnp;

use std::io;

mod entry_capnp;

fn process_entry<R: io::Read>(mut from: &mut R) -> io::Result<bool> {

    let entry = entry_capnp::read_entry(&mut from).expect("TODO: error type mapping");
    if entry.is_none() {
        return Ok(false);
    }
    let entry: entry_capnp::FileEntry = entry.unwrap();
    println!("{:?}", entry);
    return Ok(false);
}

fn main() {
    let stdin = io::stdin();
    let entry = process_entry(&mut stdin.lock()).expect("TODO: handle error");

}

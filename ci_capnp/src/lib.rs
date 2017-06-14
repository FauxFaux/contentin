extern crate capnp;

mod entry_capnp;

pub use entry_capnp::FileEntry;
pub use entry_capnp::read_entry;

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
    }
}

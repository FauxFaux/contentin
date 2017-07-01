extern crate capnp;
extern crate peeky_read;

mod entry_capnp;

/// The actual generated module for `Entry`:
pub use entry_capnp::entry;
pub use entry_capnp::FileEntry;
pub use entry_capnp::read_entry;
pub use entry_capnp::ItemType;
pub use entry_capnp::Ownership;
pub use entry_capnp::PosixEntity;

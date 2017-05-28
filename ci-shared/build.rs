extern crate capnpc;

fn main() {
    ::capnpc::CompilerCommand::new()
        .file("../entry.capnp")
        .run()
        .expect("compiling schema");
}

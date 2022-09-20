fn main() {
    ::capnpc::CompilerCommand::new()
        .src_prefix("../")
        .file("../entry.capnp")
        .run()
        .expect("compiling schema");
}

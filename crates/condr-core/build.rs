//! Compiles `proto/condr/v1` into the `protocol::pb` module (ADR 0028). `protox` is a
//! pure-Rust protobuf compiler, so no system `protoc` is needed.

fn main() {
    let proto = concat!(env!("CARGO_MANIFEST_DIR"), "/../../proto");
    println!("cargo:rerun-if-changed={proto}");
    let files = ["handshake", "session", "terminal", "messages"]
        .map(|name| format!("{proto}/condr/v1/{name}.proto"));
    let descriptors = protox::compile(&files, [proto]).expect("proto/condr/v1 compiles");
    prost_build::Config::new()
        .compile_fds(descriptors)
        .expect("generated protocol code is written");
}

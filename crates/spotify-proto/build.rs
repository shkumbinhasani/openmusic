fn main() {
    let files = [
        "metadata.proto",
        "extended_metadata.proto",
        "extension_kind.proto",
        "audio_files_extension.proto",
        "connect.proto",
        "player.proto",
        "context.proto",
        "context_page.proto",
        "context_track.proto",
        "storage-resolve.proto",
    ];
    for file in files {
        println!("cargo:rerun-if-changed=proto/{file}");
    }
    let descriptors = protox::compile(files, ["proto"]).expect("compile Spotify protobuf schemas");
    prost_build::Config::new()
        .compile_fds(descriptors)
        .expect("generate Rust protobuf types");
}

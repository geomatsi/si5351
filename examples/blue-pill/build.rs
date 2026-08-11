use std::env;
use std::fs::File;
use std::io::Write;
use std::path::PathBuf;

fn main() {
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());

    File::create(out.join("memory.x"))
        .unwrap()
        .write_all(include_bytes!("memory.x"))
        .unwrap();

    println!("cargo:rustc-link-search={}", out.display());
    println!("cargo:rerun-if-changed=memory.x");

    // Here rather than in `.cargo/config.toml`, because a `RUSTFLAGS` in the
    // environment replaces the config's `rustflags` wholesale. Losing
    // `-Tlink.x` that way does not fail the link — it produces a binary with
    // no vector table, which the linker then garbage-collects to nothing.
    println!("cargo:rustc-link-arg-bins=-Tlink.x");
    println!("cargo:rerun-if-changed=build.rs");
}

fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        // Debug CLI dispatch and argument parsing can exceed the 1 MiB MSVC default.
        println!("cargo::rustc-link-arg-bin=iolink-cli=/STACK:4194304");
    }
}

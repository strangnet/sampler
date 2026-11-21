fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        // ScreenCaptureKit's Swift bridge links against @rpath/libswift_Concurrency.dylib.
        // Cargo does not propagate `rustc-link-arg` from dependencies to downstream binaries.
        println!("cargo:rustc-link-arg=-Wl,-rpath,/usr/lib/swift");
    }
}

fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        // capture-core links Swift's runtime through screencapturekit, and a
        // dependency's rpath link args do not reach this binary.
        println!("cargo:rustc-link-arg=-Wl,-rpath,/usr/lib/swift");
        compile_permission_guide();
    }
}

fn compile_permission_guide() {
    println!("cargo:rerun-if-changed=native/permission_guide.m");
    for framework in ["AppKit", "CoreGraphics", "CoreImage", "QuartzCore"] {
        println!("cargo:rustc-link-lib=framework={framework}");
    }
    cc::Build::new()
        .file("native/permission_guide.m")
        .flag("-fobjc-arc")
        .compile("lens_permission_guide");
}

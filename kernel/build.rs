use std::env;
use std::path::PathBuf;

fn main() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let workspace = manifest.parent().unwrap();
    let arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap();
    let rpi5 = env::var("CARGO_FEATURE_RPI5").is_ok();

    let ld = match arch.as_str() {
        "aarch64" if rpi5 => workspace.join("arch/aarch64/cyanos-rpi5.ld"),
        "aarch64"         => workspace.join("arch/aarch64/cyanos.ld"),
        "x86_64"          => workspace.join("arch/x86_64/cyanos.ld"),
        other             => panic!("Cyanos: unsupported target architecture '{other}'"),
    };

    println!("cargo:rustc-link-arg=-T{}", ld.display());
    println!("cargo:rustc-link-arg=--entry=_start");
    println!("cargo:rerun-if-changed={}", ld.display());
    // Rerun if either linker script changes (feature switch may not flip).
    println!("cargo:rerun-if-changed={}", workspace.join("arch/aarch64/cyanos.ld").display());
    println!("cargo:rerun-if-changed={}", workspace.join("arch/aarch64/cyanos-rpi5.ld").display());
    println!("cargo:rerun-if-changed=src/entry_aarch64.s");
    println!("cargo:rerun-if-changed=src/entry_x86_64.s");
}

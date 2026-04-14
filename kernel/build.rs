use std::env;
use std::path::PathBuf;

fn main() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let workspace = manifest.parent().unwrap();
    let arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap();

    let ld = match arch.as_str() {
        "aarch64" => workspace.join("arch/aarch64/cyanos.ld"),
        "x86_64"  => workspace.join("arch/x86_64/cyanos.ld"),
        other     => panic!("Cyanos: unsupported target architecture '{other}'"),
    };

    println!("cargo:rustc-link-arg=-T{}", ld.display());
    println!("cargo:rustc-link-arg=--entry=_start");
    println!("cargo:rerun-if-changed={}", ld.display());
    println!("cargo:rerun-if-changed=src/entry_aarch64.s");
    println!("cargo:rerun-if-changed=src/entry_x86_64.s");
}

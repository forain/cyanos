use std::env;

fn main() {
    let arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap();

    match arch.as_str() {
        "x86_64" => {
            println!("cargo:rustc-link-arg=-Tlinkers/x86_64.ld");
        }
        "aarch64" => {
            println!("cargo:rustc-link-arg=-Tlinkers/aarch64.ld");
        }
        _ => {}
    }

    println!("cargo:rerun-if-changed=linkers/x86_64.ld");
    println!("cargo:rerun-if-changed=linkers/aarch64.ld");
}
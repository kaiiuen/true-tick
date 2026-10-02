use std::{env, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=windows/true-tick.manifest");

    let target = env::var("TARGET").expect("Cargo always supplies TARGET to build scripts");
    if !target.ends_with("-pc-windows-msvc") {
        return;
    }

    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("windows/true-tick.manifest");
    println!("cargo:rustc-link-arg-bin=true-tick=/MANIFEST:EMBED");
    println!(
        "cargo:rustc-link-arg-bin=true-tick=/MANIFESTINPUT:{}",
        manifest.display()
    );
}

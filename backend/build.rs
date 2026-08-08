use std::process::Command;
use std::path::Path;

fn main() {
    let frontend_dir = "../frontend";
    if !Path::new(frontend_dir).exists() {
        return;
    }

    let status = Command::new("npm")
        .current_dir(frontend_dir)
        .args(&["run", "build"])
        .status()
        .expect("Failed to run npm build");

    if !status.success() {
        panic!("Frontend build failed");
    }

    println!("cargo:rerun-if-changed=../frontend/src");
    println!("cargo:rerun-if-changed=../frontend/package.json");
}

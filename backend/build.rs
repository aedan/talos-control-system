use std::path::Path;
use std::process::Command;

fn main() {
    emit_build_metadata();

    let frontend_build = Path::new("../frontend/build/index.html");
    if frontend_build.exists() {
        println!("cargo:rerun-if-changed=../frontend/build/index.html");
        return;
    }

    let frontend_dir = "../frontend";
    if !Path::new(frontend_dir).exists() {
        return;
    }

    let status = Command::new("npm")
        .current_dir(frontend_dir)
        .args(["run", "build"])
        .status()
        .expect("Failed to run npm build");

    if !status.success() {
        panic!("Frontend build failed");
    }

    println!("cargo:rerun-if-changed=../frontend/src");
    println!("cargo:rerun-if-changed=../frontend/package.json");
}

fn emit_build_metadata() {
    println!("cargo:rerun-if-env-changed=GIT_HASH");
    println!("cargo:rerun-if-env-changed=BUILD_TIME");
    println!("cargo:rerun-if-env-changed=SOURCE_DATE_EPOCH");

    let commit = std::env::var("GIT_HASH").unwrap_or_else(|_| {
        Command::new("git")
            .args(["rev-parse", "--short=12", "HEAD"])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "unknown".to_string())
    });

    let build_time = std::env::var("BUILD_TIME").unwrap_or_else(|_| {
        // Prefer SOURCE_DATE_EPOCH for reproducible builds; else UTC now.
        if let Ok(epoch) = std::env::var("SOURCE_DATE_EPOCH") {
            return epoch;
        }
        // Fall back to a simple ISO-ish stamp from `date` when available.
        Command::new("date")
            .args(["-u", "+%Y-%m-%dT%H:%M:%SZ"])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "unknown".to_string())
    });

    println!("cargo:rustc-env=GIT_HASH={commit}");
    println!("cargo:rustc-env=BUILD_TIME={build_time}");
}

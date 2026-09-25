//! Build provenance for the Android library, matching skate-game's build.rs
//! without the Windows icon resource step.
use std::{env, process::Command};

fn main() {
    for name in ["HEAD", "index", "refs"] {
        if let Ok(output) = Command::new("git").args(["rev-parse", "--git-path", name]).output() {
            if output.status.success() { println!("cargo:rerun-if-changed={}", String::from_utf8_lossy(&output.stdout).trim()); }
        }
    }
    for path in ["../skate-game/src", "../skate-core/src", "../skate-data/src", "../skate-net/src", "../../Cargo.lock", "../../Cargo.toml", "Cargo.toml", "../../vendor/bevy_pbr", "../../vendor/bevy_core_pipeline"] {
        println!("cargo:rerun-if-changed={path}");
    }
    let git = |args: &[&str]| Command::new("git").args(args).output().ok().filter(|o| o.status.success()).map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned());
    let revision = git(&["rev-parse", "HEAD"]).unwrap_or_else(|| "revision-unavailable".into());
    println!("cargo:rerun-if-env-changed=GITHUB_RUN_NUMBER");
    println!("cargo:rustc-env=SKATE_RELEASE_REVISION={revision}");
    println!("cargo:rustc-env=SKATE_RELEASE_BUILD={}", env::var("GITHUB_RUN_NUMBER").unwrap_or_else(|_| "0".into()));
    let dirty = git(&["status", "--porcelain", "--untracked-files=normal"]).map(|s| !s.is_empty());
    let compiler = Command::new(env::var_os("RUSTC").unwrap_or_else(|| "rustc".into())).arg("--version").output().ok().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned()).unwrap_or_default();
    println!("cargo:rustc-env=SKATE_BUILD_ID={} revision={} dirty={dirty:?} target={} profile={} dynamic=false compiler={compiler}", env::var("CARGO_PKG_VERSION").unwrap(), revision, env::var("TARGET").unwrap(), env::var("PROFILE").unwrap());
}

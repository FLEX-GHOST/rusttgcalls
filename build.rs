//! Build script for rusttgcalls.
//! Automatically checks environment, system tools, and configures target platform settings.

use std::env;
use std::process::Command;

fn main() {
    // Re-run this build script only if build.rs or Cargo.toml changes
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=Cargo.toml");

    // Inform cargo about custom cfg flags
    println!("cargo:rustc-check-cfg=cfg(has_ffmpeg)");

    // Probe if ffmpeg is present on the host system at build time
    let has_ffmpeg = Command::new("ffmpeg")
        .arg("-version")
        .output()
        .map(|out| out.status.success())
        .unwrap_or(false);

    if has_ffmpeg {
        println!("cargo:rustc-cfg=has_ffmpeg");
    }

    // Target OS specific link flags and configurations
    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    match target_os.as_str() {
        "windows" => {
            println!("cargo:rustc-link-lib=ws2_32");
            println!("cargo:rustc-link-lib=crypt32");
        }
        "macos" => {
            println!("cargo:rustc-link-lib=framework=Security");
        }
        "linux" | "android" => {
            // Linux standard runtime settings
        }
        _ => {}
    }
}

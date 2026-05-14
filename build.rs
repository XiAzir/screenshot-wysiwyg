use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=Kumokiri.ico");
    println!("cargo:rerun-if-changed=build.rs");

    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let manifest_dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let icon_path = manifest_dir.join("Kumokiri.ico");
    if !icon_path.exists() {
        println!("cargo:warning=Kumokiri.ico not found; application icon was not embedded");
        return;
    }

    let out_dir = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let rc_path = out_dir.join("app_icon.rc");
    let icon_path = icon_path.to_string_lossy().replace('\\', "/");

    fs::write(&rc_path, format!("1 ICON \"{}\"\n", icon_path))
        .expect("failed to write app icon resource script");

    let resource_path =
        if let Some(rc) = find_program("rc.exe").or_else(|| find_program("llvm-rc.exe")) {
            compile_with_rc(&rc, &rc_path, &out_dir)
        } else if let Some(windres) = find_program("windres.exe") {
            compile_with_windres(&windres, &rc_path, &out_dir)
        } else {
            println!(
            "cargo:warning=no Windows resource compiler found; application icon was not embedded"
        );
            return;
        };

    println!("cargo:rustc-link-arg-bins={}", resource_path.display());
}

fn find_program(name: &str) -> Option<PathBuf> {
    let path = env::var_os("PATH")?;
    env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
}

fn compile_with_rc(rc: &Path, rc_path: &Path, out_dir: &Path) -> PathBuf {
    let output = out_dir.join("app_icon_resource.res");
    let status = Command::new(rc)
        .arg("/nologo")
        .arg(format!("/fo{}", output.display()))
        .arg(rc_path)
        .status()
        .expect("failed to run Windows resource compiler");
    if !status.success() {
        panic!("Windows resource compiler failed with status {status}");
    }
    output
}

fn compile_with_windres(windres: &Path, rc_path: &Path, out_dir: &Path) -> PathBuf {
    if env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        let output = out_dir.join("app_icon_resource.res");
        let status = Command::new(windres)
            .arg("--input-format=rc")
            .arg("--output-format=res")
            .arg("-i")
            .arg(rc_path)
            .arg("-o")
            .arg(&output)
            .status()
            .expect("failed to run windres");
        if !status.success() {
            panic!("windres failed with status {status}");
        }
        return output;
    }

    let output = out_dir.join("app_icon_resource.obj");
    let target = match env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
        Ok("x86") => "pe-i386",
        Ok("x86_64") => "pe-x86-64",
        Ok("aarch64") => "pe-aarch64-little",
        _ => "pe-x86-64",
    };
    let status = Command::new(windres)
        .arg("--input-format=rc")
        .arg("--output-format=coff")
        .arg(format!("--target={target}"))
        .arg("-i")
        .arg(rc_path)
        .arg("-o")
        .arg(&output)
        .status()
        .expect("failed to run windres");
    if !status.success() {
        panic!("windres failed with status {status}");
    }
    output
}

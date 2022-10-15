use std::env;
use std::fs;
use std::path::Path;
use std::process::Command;

fn main() {
    let out_dir = env::var("OUT_DIR").unwrap();
    let build_info_path = Path::new(&out_dir).join("build_info");
    let cwd = env!("CARGO_MANIFEST_DIR");

    let build_info_string = format!(
        "{} {}{}",
        env::var("PROFILE").unwrap(),
        commit_hash(cwd),
        status_suffix(cwd),
    );

    fs::write(build_info_path, build_info_string).unwrap();
}

fn commit_hash(cwd: &str) -> String {
    let output = Command::new("git")
        .arg("rev-parse")
        .arg("HEAD")
        .current_dir(cwd)
        .output()
        .unwrap();

    String::from_utf8(output.stdout)
        .expect("Failed to parse commit hash")
        .trim_end()
        .to_string()
}

fn status_suffix(cwd: &str) -> &'static str {
    let status = Command::new("git")
        .arg("diff")
        .arg("--quiet")
        .arg("--exit-code")
        .current_dir(cwd)
        .status()
        .unwrap();

    if status.success() {
        ""
    } else {
        "+"
    }
}

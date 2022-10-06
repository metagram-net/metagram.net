use std::env;
use std::fs;
use std::path::Path;
use std::process::Command;

fn main() {
    let out_dir = env::var("OUT_DIR").unwrap();
    let version_path = Path::new(&out_dir).join("version");
    let cwd = env!("CARGO_MANIFEST_DIR");

    let version_string = format!(
        "{} {} ({}{}, {})",
        env!("CARGO_PKG_NAME"),
        env!("CARGO_PKG_VERSION"),
        commit_hash(cwd),
        status_suffix(cwd),
        env::var("PROFILE").unwrap(),
    );

    fs::write(version_path, version_string).unwrap();
}

fn commit_hash(cwd: &str) -> String {
    let output = Command::new("git")
        .arg("rev-parse")
        .arg("HEAD")
        .current_dir(cwd)
        .output()
        .expect("Failed to execute command");

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
        .expect("Failed to execute command");

    if status.success() {
        ""
    } else {
        "+"
    }
}

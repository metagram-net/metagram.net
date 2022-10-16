use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    let out_dir = PathBuf::from(&env::var("OUT_DIR").unwrap());

    fs::write(out_dir.join("build_profile"), env::var("PROFILE").unwrap()).unwrap();

    fs::write(
        out_dir.join("commit_hash"),
        commit_hash(env!("CARGO_MANIFEST_DIR")),
    )
    .unwrap();
}

fn commit_hash(cwd: &str) -> String {
    let hash = {
        let output = Command::new("git")
            .arg("rev-parse")
            .arg("HEAD")
            .current_dir(cwd)
            .output()
            .unwrap();

        String::from_utf8(output.stdout).unwrap().trim().to_string()
    };

    let suffix = {
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
    };

    hash + suffix
}

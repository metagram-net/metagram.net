use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    let out_dir = PathBuf::from(&env::var("OUT_DIR").unwrap());

    fs::write(out_dir.join("build_profile"), env::var("PROFILE").unwrap()).unwrap();

    let commit_hash = match git_hash(env!("CARGO_MANIFEST_DIR")) {
        Some(hash) => hash,
        None => env::var("METAGRAM_COMMIT_HASH").unwrap(),
    };

    fs::write(out_dir.join("commit_hash"), commit_hash).unwrap();
}

fn git_hash(cwd: &str) -> Option<String> {
    let hash = {
        let output = Command::new("git")
            .arg("rev-parse")
            .arg("HEAD")
            .current_dir(cwd)
            .output()
            .ok()?;

        if output.status.success() {
            String::from_utf8(output.stdout).unwrap().trim().to_string()
        } else {
            return None;
        }
    };

    let suffix = {
        let status = Command::new("git")
            .arg("diff")
            .arg("--quiet")
            .arg("--exit-code")
            .current_dir(cwd)
            .status()
            .ok()?;

        if status.success() {
            ""
        } else {
            "+"
        }
    };

    Some(hash + suffix)
}

//! Small helpers shared by the xtask commands.

use std::path::PathBuf;
use std::process::Command;

use anyhow::{Context, Result, bail};

/// The repository root (the parent of `xtask/`).
pub fn repo_root() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .parent()
        .map(PathBuf::from)
        .unwrap_or(manifest_dir)
}

/// Runs a command from the repo root, streaming its output. Fails if it fails.
pub fn run(program: &str, args: &[&str]) -> Result<()> {
    let status = Command::new(program)
        .args(args)
        .current_dir(repo_root())
        .status()
        .with_context(|| format!("could not start `{program}`"))?;
    if !status.success() {
        bail!("`{program} {}` failed ({status})", args.join(" "));
    }
    Ok(())
}

/// Runs a command from the repo root and returns its stdout.
pub fn output(program: &str, args: &[&str]) -> Result<String> {
    let out = Command::new(program)
        .args(args)
        .current_dir(repo_root())
        .output()
        .with_context(|| format!("could not start `{program}`"))?;
    if !out.status.success() {
        bail!(
            "`{program} {}` failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Path relative to the repo root, with forward slashes, for messages.
pub fn display_path(path: &std::path::Path) -> String {
    let root = repo_root();
    path.strip_prefix(&root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

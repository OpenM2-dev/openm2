//! `just check` / `just quick`: the same steps CI runs, in order, stopping at the
//! first failure with the recipe to rerun it alone.

use std::collections::BTreeSet;
use std::process::Command;

use anyhow::{Result, bail};

use crate::util::{output, repo_root, run};
use crate::{checks, codegen, notices};

struct Tool {
    name: &'static str,
    program: &'static str,
    args: &'static [&'static str],
    install: &'static str,
    needed_for_quick: bool,
}

const TOOLS: &[Tool] = &[
    Tool {
        name: "git",
        program: "git",
        args: &["--version"],
        install: "https://git-scm.com/downloads",
        needed_for_quick: true,
    },
    Tool {
        name: "rustfmt",
        program: "rustfmt",
        args: &["--version"],
        install: "rustup component add rustfmt (rust-toolchain.toml lists it)",
        needed_for_quick: true,
    },
    Tool {
        name: "cargo-deny",
        program: "cargo",
        args: &["deny", "--version"],
        install: "cargo install --locked cargo-deny",
        needed_for_quick: false,
    },
];

/// Checks the tools `just check` needs. Returns an error listing what is missing.
pub fn preflight(full: bool) -> Result<()> {
    let mut missing = Vec::new();
    for tool in TOOLS.iter().filter(|t| full || t.needed_for_quick) {
        let ok = Command::new(tool.program)
            .args(tool.args)
            .current_dir(repo_root())
            .output()
            .is_ok_and(|out| out.status.success());
        if !ok {
            missing.push(format!("{}: install with `{}`", tool.name, tool.install));
        }
    }
    if !missing.is_empty() {
        bail!(
            "missing tools (ask the human to install them):\n  - {}",
            missing.join("\n  - ")
        );
    }
    println!("all tools present");
    Ok(())
}

fn step(name: &str, recipe: &str, f: impl FnOnce() -> Result<()>) -> Result<()> {
    println!("\n==> {name}");
    if let Err(err) = f() {
        bail!("{name} failed; rerun it alone with `{recipe}`\n\n{err:?}");
    }
    Ok(())
}

/// Workspace crates touched by uncommitted changes, or None if a change affects
/// every crate (root manifests, toolchain, config) or there is no commit yet.
fn changed_packages() -> Result<Option<BTreeSet<String>>> {
    if output("git", &["rev-parse", "--verify", "HEAD"]).is_err() {
        return Ok(None);
    }
    let status = output("git", &["status", "--porcelain=v1", "-uall"])?;
    let mut packages = BTreeSet::new();
    for line in status.lines() {
        let path = line.get(3..).unwrap_or_default();
        let path = path.rsplit(" -> ").next().unwrap_or(path).trim_matches('"');
        let mut parts = path.split('/');
        match (parts.next(), parts.next()) {
            (Some("crates"), Some(name)) => {
                packages.insert(name.to_owned());
            }
            (Some("xtask"), _) => {
                packages.insert("xtask".to_owned());
            }
            (Some("docs" | ".claude" | ".github"), _) => {}
            (Some(file), None) if file.ends_with(".md") => {}
            _ => return Ok(None),
        }
    }
    Ok(Some(packages))
}

/// `just check` (quick = false) and `just quick` (quick = true).
pub fn ci(quick: bool) -> Result<()> {
    step("preflight", "just preflight", || preflight(!quick))?;
    step("format", "just fmt", || {
        run("cargo", &["fmt", "--all", "--", "--check"])
    })?;

    if quick {
        let packages = changed_packages()?;
        let mut args: Vec<String> = Vec::new();
        match &packages {
            None => args.push("--workspace".to_owned()),
            Some(set) => {
                if set.is_empty() {
                    println!("\nno crate changes; skipping clippy and tests");
                }
                for name in set {
                    args.push("--package".to_owned());
                    args.push(name.clone());
                }
            }
        }
        if !args.is_empty() {
            let selection: Vec<&str> = args.iter().map(String::as_str).collect();
            step("clippy", "just lint", || {
                let mut clippy = vec!["clippy"];
                clippy.extend(&selection);
                clippy.extend(["--all-targets", "--locked", "--", "-D", "warnings"]);
                run("cargo", &clippy)
            })?;
            step("tests", "just test", || {
                let mut test = vec!["test"];
                test.extend(&selection);
                test.push("--locked");
                run("cargo", &test)
            })?;
        }
        step("generated files", "just drift", codegen::drift)?;
        println!("\nquick check passed (run `just check` before handing off)");
        return Ok(());
    }

    step("clippy", "just lint", || {
        run(
            "cargo",
            &[
                "clippy",
                "--workspace",
                "--all-targets",
                "--locked",
                "--",
                "-D",
                "warnings",
            ],
        )
    })?;
    step("tests", "just test", || {
        run("cargo", &["test", "--workspace", "--locked"])
    })?;
    step("generated files", "just drift", codegen::drift)?;
    step("crate dependencies", "just check-deps", checks::check_deps)?;
    step("no assets", "just no-assets", checks::no_assets)?;
    step(
        "snapshots",
        "cargo insta review",
        checks::snapshot_leftovers,
    )?;
    // Every package in the release's programs has a licence text to ship.
    step("third-party notices", "just notices", || {
        notices::write(
            notices::WINDOWS_PACKAGES,
            notices::WINDOWS_TARGET,
            &repo_root().join("target/THIRD-PARTY-NOTICES.txt"),
        )
    })?;
    step("dependencies (cargo-deny)", "just deny", || {
        run("cargo", &["deny", "--locked", "check"])
    })?;
    println!("\nall checks passed");
    Ok(())
}

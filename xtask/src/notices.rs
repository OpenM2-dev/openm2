//! `just notices`: the third-party notices that travel with a release's
//! programs. Every package they are built from, other than the workspace's
//! own: its licence, where its source is, and the licence texts it ships (or a
//! standard text when it ships none), from `cargo metadata` and the packages'
//! own folders.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde_json::Value;

use crate::util::{display_path, output, repo_root};

/// The packages whose programs the Windows release ships.
pub const WINDOWS_PACKAGES: &[&str] = &["m2-cli", "m2-launcher"];
/// The target the Windows release is built for.
pub const WINDOWS_TARGET: &str = "x86_64-pc-windows-msvc";

/// A package's licence texts are the files whose names start with one of
/// these, in any case, and every file in a `LICENSES` folder. Code a package
/// carries from elsewhere keeps its own texts in its folder (aws-lc-sys's
/// C library, ring's), so folders below count too, to `MAX_DEPTH`.
const TEXT_PREFIXES: &[&str] = &[
    "LICENSE",
    "LICENCE",
    "COPYING",
    "COPYRIGHT",
    "NOTICE",
    "UNLICENSE",
];

/// Folders whose files never end up in the programs.
const SKIPPED_FOLDERS: &[&str] = &[
    "tests", "test", "testdata", "benches", "examples", "fuzz", "docs", "doc", "target", ".git",
];

/// How deep below a package's folder licence texts are looked for.
const MAX_DEPTH: usize = 4;

/// The largest licence file read: a bigger one is a mistake.
const MAX_TEXT: u64 = 1 << 20;

/// One third-party package and its licence texts.
#[derive(Debug, PartialEq)]
struct Package {
    name: String,
    version: String,
    license: String,
    repository: Option<String>,
    texts: Vec<String>,
    /// The package ships no licence text: `texts` holds a standard one.
    standard: bool,
}

/// Writes the notices for `packages`' programs built for `target` to `out`.
pub fn write(packages: &[&str], target: &str, out: &Path) -> Result<()> {
    let json = output(
        "cargo",
        &[
            "metadata",
            "--format-version",
            "1",
            "--locked",
            "--filter-platform",
            target,
        ],
    )?;
    let metadata: Value = serde_json::from_str(&json).context("cargo metadata's output")?;
    let mut found = Vec::new();
    for package in third_party(&metadata, packages)? {
        found.push(read(package)?);
    }
    found.sort_by(|a, b| (&a.name, &a.version).cmp(&(&b.name, &b.version)));
    if let Some(parent) = out.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    fs::write(out, render(packages, target, &found))
        .with_context(|| format!("writing {}", out.display()))?;
    println!(
        "{} third-party packages' notices written to {}",
        found.len(),
        display_path(out)
    );
    Ok(())
}

/// The packages `roots` are built from, other than the workspace's own:
/// normal dependencies only, followed all the way (build scripts' and tests'
/// dependencies don't end up in the programs).
fn third_party<'a>(metadata: &'a Value, roots: &[&str]) -> Result<Vec<&'a Value>> {
    let packages: BTreeMap<&str, &Value> = metadata["packages"]
        .as_array()
        .context("cargo metadata has no packages")?
        .iter()
        .filter_map(|p| Some((p["id"].as_str()?, p)))
        .collect();
    let nodes: BTreeMap<&str, &Value> = metadata["resolve"]["nodes"]
        .as_array()
        .context("cargo metadata has no resolve graph")?
        .iter()
        .filter_map(|n| Some((n["id"].as_str()?, n)))
        .collect();

    let mut todo = Vec::new();
    for root in roots {
        let id = packages
            .iter()
            .find(|(_, p)| p["name"] == *root && p["source"].is_null())
            .map(|(id, _)| *id)
            .with_context(|| format!("no workspace package named {root}"))?;
        todo.push(id);
    }
    let mut seen = BTreeSet::new();
    while let Some(id) = todo.pop() {
        if !seen.insert(id) {
            continue;
        }
        let node = nodes
            .get(id)
            .with_context(|| format!("{id} is missing from the resolve graph"))?;
        for dep in node["deps"].as_array().into_iter().flatten() {
            let normal = dep["dep_kinds"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|kind| kind["kind"].is_null());
            if let (true, Some(pkg)) = (normal, dep["pkg"].as_str()) {
                todo.push(pkg);
            }
        }
    }
    Ok(seen
        .into_iter()
        .filter_map(|id| packages.get(id).copied())
        .filter(|p| !p["source"].is_null())
        .collect())
}

/// A package's licence and its texts, from its folder.
fn read(package: &Value) -> Result<Package> {
    let field = |key: &str| package[key].as_str().map(str::to_owned);
    let name = field("name").context("a package without a name")?;
    let version = field("version").context("a package without a version")?;
    let license = field("license").unwrap_or_else(|| "see its licence text".to_owned());
    let manifest = field("manifest_path").context("a package without a manifest")?;
    let folder = Path::new(&manifest)
        .parent()
        .with_context(|| format!("{name}'s manifest has no folder"))?;

    let mut files = BTreeSet::new();
    find_texts(folder, 0, false, &mut files)?;
    if let Some(file) = field("license_file") {
        let path = folder.join(file);
        if path.is_file() {
            files.insert(path);
        }
    }

    let mut texts = Vec::new();
    for file in &files {
        let text = read_text(file)?;
        if !text.is_empty() && !texts.contains(&text) {
            texts.push(text);
        }
    }
    let standard = texts.is_empty();
    if standard {
        let authors: Vec<String> = package["authors"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|a| a.as_str().map(str::to_owned))
            .collect();
        texts.push(
            standard_text(&name, &license, &authors)
                .with_context(|| format!("{name} {version} ships no licence text"))?,
        );
    }
    Ok(Package {
        name,
        version,
        license,
        repository: field("repository"),
        texts,
        standard,
    })
}

/// The licence texts in `folder` and the folders below it (`TEXT_PREFIXES`).
fn find_texts(
    folder: &Path,
    depth: usize,
    licenses: bool,
    files: &mut BTreeSet<PathBuf>,
) -> Result<()> {
    for entry in fs::read_dir(folder).with_context(|| format!("reading {}", folder.display()))? {
        let path = entry?.path();
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let upper = name.to_uppercase();
        if path.is_file() {
            if licenses || TEXT_PREFIXES.iter().any(|prefix| upper.starts_with(prefix)) {
                files.insert(path);
            }
        } else if path.is_dir()
            && depth < MAX_DEPTH
            && !SKIPPED_FOLDERS.contains(&name.to_lowercase().as_str())
        {
            find_texts(&path, depth + 1, upper == "LICENSES", files)?;
        }
    }
    Ok(())
}

/// A licence file's text, with Unix line ends and no blank lines around it.
fn read_text(file: &Path) -> Result<String> {
    let size = fs::metadata(file)?.len();
    if size > MAX_TEXT {
        bail!("{} is {size} bytes: not a licence text", file.display());
    }
    let bytes = fs::read(file).with_context(|| format!("reading {}", file.display()))?;
    Ok(String::from_utf8_lossy(&bytes)
        .replace("\r\n", "\n")
        .trim()
        .to_owned())
}

/// The text of a licence the package may be used under, for a package that
/// ships none: MIT with its authors, else Apache-2.0 (the repo's own copy),
/// else the Boost licence.
fn standard_text(name: &str, license: &str, authors: &[String]) -> Result<String> {
    let choices: Vec<&str> = license
        .split('/')
        .flat_map(|part| part.split(" OR "))
        .map(str::trim)
        .collect();
    if choices.contains(&"MIT") {
        let holders = if authors.is_empty() {
            format!("the {name} authors")
        } else {
            authors.join(", ")
        };
        return Ok(format!(
            "MIT License\n\nCopyright (c) {holders}\n\n{MIT_TERMS}"
        ));
    }
    if choices.contains(&"Apache-2.0") {
        return read_text(&repo_root().join("LICENSE-APACHE"));
    }
    if choices.contains(&"BSL-1.0") {
        return Ok(BOOST_TEXT.to_owned());
    }
    bail!("no standard text for its licence, {license}: add one to xtask/src/notices.rs")
}

/// The Boost Software License's text, which names no holder.
const BOOST_TEXT: &str = "\
Boost Software License - Version 1.0 - August 17th, 2003

Permission is hereby granted, free of charge, to any person or organization
obtaining a copy of the software and accompanying documentation covered by
this license (the \"Software\") to use, reproduce, display, distribute,
execute, and transmit the Software, and to prepare derivative works of the
Software, and to permit third-parties to whom the Software is furnished to
do so, all subject to the following:

The copyright notices in the Software and this entire statement, including
the above license grant, this restriction and the following disclaimer,
must be included in all copies of the Software, in whole or in part, and
all derivative works of the Software, unless such copies or derivative
works are solely in the form of machine-executable object code generated by
a source language processor.

THE SOFTWARE IS PROVIDED \"AS IS\", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE, TITLE AND NON-INFRINGEMENT. IN NO EVENT
SHALL THE COPYRIGHT HOLDERS OR ANYONE DISTRIBUTING THE SOFTWARE BE LIABLE
FOR ANY DAMAGES OR OTHER LIABILITY, WHETHER IN CONTRACT, TORT OR OTHERWISE,
ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER
DEALINGS IN THE SOFTWARE.";

const MIT_TERMS: &str = "\
Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the \"Software\"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED \"AS IS\", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.";

/// The notices file: the packages in order, then each distinct text once.
fn render(roots: &[&str], target: &str, packages: &[Package]) -> String {
    let mut numbers: BTreeMap<&str, usize> = BTreeMap::new();
    let mut texts: Vec<&str> = Vec::new();
    let mut out = String::new();
    let heading = format!("Packages ({})", packages.len());
    let _ = write!(
        out,
        "Third-party notices\n\
         ===================\n\
         \n\
         Programs: built from {}, for {target}.\n\
         \n\
         They contain the open-source packages below, each under its own\n\
         licence. Each package's source code is at\n\
         https://crates.io/crates/<name>/<version> and in its repository.\n\
         Written by `cargo xtask notices` from Cargo.lock.\n\
         \n\
         {heading}\n\
         {}\n",
        roots.join(", "),
        "-".repeat(heading.len())
    );
    for package in packages {
        let mut refs = Vec::new();
        for text in &package.texts {
            let number = *numbers.entry(text.as_str()).or_insert_with(|| {
                texts.push(text.as_str());
                texts.len()
            });
            refs.push(number.to_string());
        }
        let _ = write!(
            out,
            "\n{} {}\n    licence: {}\n",
            package.name, package.version, package.license
        );
        if let Some(repository) = &package.repository {
            let _ = writeln!(out, "    source:  {repository}");
        }
        let _ = writeln!(
            out,
            "    texts:   {}{}",
            refs.join(", "),
            if package.standard {
                " (the package ships none: a standard text of its licence)"
            } else {
                ""
            }
        );
    }
    let heading = format!("Licence texts ({})", texts.len());
    let _ = write!(out, "\n\n{heading}\n{}\n", "-".repeat(heading.len()));
    for (index, text) in texts.iter().enumerate() {
        let _ = write!(out, "\n=== {} ===\n\n{text}\n", index + 1);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn package(name: &str, texts: &[&str]) -> Package {
        Package {
            name: name.to_owned(),
            version: "1.0.0".to_owned(),
            license: "MIT OR Apache-2.0".to_owned(),
            repository: Some(format!("https://example.org/{name}")),
            texts: texts.iter().map(|t| (*t).to_owned()).collect(),
            standard: false,
        }
    }

    #[test]
    fn each_text_is_written_once_and_numbered_in_order() {
        let packages = [
            package("alpha", &["apache text", "mit alpha"]),
            package("beta", &["apache text"]),
            package("gamma", &["mit gamma"]),
        ];
        let out = render(&["app"], "x86_64-pc-windows-msvc", &packages);
        assert!(out.contains("Programs: built from app, for x86_64-pc-windows-msvc.\n"));
        assert!(out.contains("Packages (3)\n------------\n"));
        assert!(out.contains("Packages (3)"));
        assert!(out.contains("\nalpha 1.0.0\n    licence: MIT OR Apache-2.0\n    source:  https://example.org/alpha\n    texts:   1, 2\n"));
        assert!(out.contains("\nbeta 1.0.0\n"));
        assert!(out.contains("    texts:   1\n"));
        assert!(out.contains("    texts:   3\n"));
        assert!(out.contains("Licence texts (3)\n-----------------\n"));
        assert_eq!(out.matches("apache text").count(), 1);
        let order: Vec<usize> = [
            "=== 1 ===\n\napache text",
            "=== 2 ===\n\nmit alpha",
            "=== 3 ===\n\nmit gamma",
        ]
        .iter()
        .map(|needle| out.find(needle).expect("in the notices"))
        .collect();
        assert!(order.windows(2).all(|pair| pair[0] < pair[1]));
    }

    #[test]
    fn a_package_without_texts_gets_a_standard_one() {
        let mit = standard_text("kira", "MIT OR Apache-2.0", &["Andrew Minnich".to_owned()])
            .expect("MIT is a choice");
        assert!(mit.starts_with("MIT License\n\nCopyright (c) Andrew Minnich\n\nPermission"));
        let old_style = standard_text("old", "Apache-2.0/MIT", &[]).expect("MIT is a choice");
        assert!(old_style.contains("Copyright (c) the old authors"));
        let apache = standard_text("spirv", "Apache-2.0", &[]).expect("Apache-2.0 is a choice");
        assert!(apache.contains("Apache License"));
        let boost = standard_text("clipboard-win", "BSL-1.0", &[]).expect("Boost is a choice");
        assert!(boost.starts_with("Boost Software License - Version 1.0"));
        assert!(standard_text("both", "Apache-2.0 AND MIT", &[]).is_err());
        assert!(standard_text("other", "BSD-3-Clause", &[]).is_err());
    }

    #[test]
    fn only_normal_dependencies_outside_the_workspace_count() {
        let metadata = serde_json::json!({
            "packages": [
                {"id": "app", "name": "app", "source": null},
                {"id": "lib", "name": "lib", "source": null},
                {"id": "a", "name": "a", "source": "registry"},
                {"id": "b", "name": "b", "source": "registry"},
                {"id": "built", "name": "built", "source": "registry"},
                {"id": "tested", "name": "tested", "source": "registry"},
                {"id": "unused", "name": "unused", "source": "registry"},
            ],
            "resolve": {"nodes": [
                {"id": "app", "deps": [
                    {"pkg": "lib", "dep_kinds": [{"kind": null}]},
                    {"pkg": "built", "dep_kinds": [{"kind": "build"}]},
                    {"pkg": "tested", "dep_kinds": [{"kind": "dev"}]},
                ]},
                {"id": "lib", "deps": [{"pkg": "a", "dep_kinds": [{"kind": "dev"}, {"kind": null}]}]},
                {"id": "a", "deps": [{"pkg": "b", "dep_kinds": [{"kind": null}]}]},
                {"id": "b", "deps": []},
                {"id": "built", "deps": []},
                {"id": "tested", "deps": []},
                {"id": "unused", "deps": []},
            ]},
        });
        let found: Vec<&str> = third_party(&metadata, &["app"])
            .expect("a graph")
            .iter()
            .filter_map(|p| p["name"].as_str())
            .collect();
        assert_eq!(found, ["a", "b"]);
        assert!(third_party(&metadata, &["missing"]).is_err());
    }
}

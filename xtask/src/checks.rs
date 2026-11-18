//! Repo guards: crate dependency direction and no proprietary assets in the tree.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Read as _;

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use crate::util::{output, repo_root};

/// Which workspace crates each crate may depend on. Dependencies point one way:
/// protocol → core → net → cli, formats → assets → cli, formats → render →
/// cli, protocol → server-core → server, content → server-core, server and
/// cli (the import), update → launcher and cli (`release`), and ui and render
/// → launcher (its window). A new crate must be added here and to
/// docs/architecture.md.
const ALLOWED_DEPS: &[(&str, &[&str])] = &[
    ("m2-protocol", &[]),
    ("m2-core", &["m2-protocol"]),
    ("m2-net", &["m2-protocol", "m2-core"]),
    ("m2-formats", &[]),
    ("m2-assets", &["m2-formats"]),
    ("m2-ui", &[]),
    ("m2-render", &["m2-formats", "m2-ui"]),
    ("m2-audio", &["m2-formats"]),
    ("m2-content", &[]),
    ("m2-server-core", &["m2-protocol", "m2-content"]),
    (
        "m2-server",
        &["m2-protocol", "m2-server-core", "m2-content"],
    ),
    (
        "m2-cli",
        &[
            "m2-protocol",
            "m2-core",
            "m2-net",
            "m2-formats",
            "m2-assets",
            "m2-render",
            "m2-ui",
            "m2-audio",
            "m2-content",
            "m2-update",
        ],
    ),
    ("m2-update", &[]),
    ("m2-launcher", &["m2-update", "m2-ui", "m2-render"]),
    ("xtask", &[]),
];

/// Workspace crates a crate's tests may use on top of `ALLOWED_DEPS`: the
/// server's tests run the client's session as real clients, and the CLI's
/// tests start the server and import a client as the launcher does. The
/// client never depends on the server outside tests, nor the server on the
/// client.
const ALLOWED_DEV_DEPS: &[(&str, &[&str])] = &[
    ("m2-server", &["m2-core", "m2-net"]),
    ("m2-cli", &["m2-server", "m2-server-core", "m2-launcher"]),
];

#[derive(Deserialize)]
struct Metadata {
    packages: Vec<MetaPackage>,
}

#[derive(Deserialize)]
struct MetaPackage {
    name: String,
    publish: Option<Vec<String>>,
    dependencies: Vec<MetaDependency>,
}

#[derive(Deserialize)]
struct MetaDependency {
    name: String,
    path: Option<String>,
    source: Option<String>,
    /// `dev` for dev-dependencies, `build` for build-dependencies, none for
    /// normal ones.
    kind: Option<String>,
}

/// `just check-deps`
pub fn check_deps() -> Result<()> {
    let json = output(
        "cargo",
        &["metadata", "--format-version", "1", "--no-deps", "--locked"],
    )?;
    let metadata: Metadata = serde_json::from_str(&json).context("parsing cargo metadata")?;
    let allowed: BTreeMap<&str, &[&str]> = ALLOWED_DEPS.iter().copied().collect();
    let allowed_dev: BTreeMap<&str, &[&str]> = ALLOWED_DEV_DEPS.iter().copied().collect();
    let mut problems = Vec::new();

    for package in &metadata.packages {
        let name = package.name.as_str();
        let Some(allowed_deps) = allowed.get(name) else {
            problems.push(format!(
                "`{name}` is not in ALLOWED_DEPS (xtask/src/checks.rs); add it there and to docs/architecture.md"
            ));
            continue;
        };
        if package
            .publish
            .as_ref()
            .is_none_or(|registries| !registries.is_empty())
        {
            problems.push(format!("`{name}` must set `publish = false`"));
        }
        let dev_deps = allowed_dev.get(name).copied().unwrap_or_default();
        for dep in &package.dependencies {
            let is_dev = dep.kind.as_deref() == Some("dev");
            let allowed_here = allowed_deps.contains(&dep.name.as_str())
                || (is_dev && dev_deps.contains(&dep.name.as_str()));
            if dep.path.is_some() && !allowed_here {
                problems.push(format!(
                    "`{name}` must not depend on `{}` (allowed: {})",
                    dep.name,
                    if allowed_deps.is_empty() {
                        "no workspace crates".to_owned()
                    } else {
                        allowed_deps.join(", ")
                    }
                ));
            }
            if dep
                .source
                .as_deref()
                .is_some_and(|source| source.starts_with("git+"))
            {
                problems.push(format!(
                    "`{name}` uses a git dependency `{}`; use crates.io releases",
                    dep.name
                ));
            }
        }
    }

    if !problems.is_empty() {
        bail!("dependency rules broken:\n  - {}", problems.join("\n  - "));
    }
    println!("crate dependencies follow the allowed direction");
    Ok(())
}

/// File extensions of game client data, and of pictures: a screenshot of a
/// model shows the user's assets. None of these may be committed.
const ASSET_EXTENSIONS: &[&str] = &[
    "eix", "epk", "ecpk", "gr2", "dds", "tga", "mde", "msm", "msa", "mse", "mss", "msenv", "prt",
    "spt", "atr", "wtr", "raw", "pyc", "dll", "exe", "mp3", "wav", "ogg", "png", "jpg", "jpeg",
    "bmp", "glb", "gltf",
];

/// Leading bytes of client archive and data containers.
const ASSET_MAGICS: &[&[u8; 4]] = &[b"EPKD", b"MCOZ", b"MCSP", b"MIPX", b"MMPT"];

/// How every file `openm2 content import` writes starts (`m2_content::IMPORT_MARKER`):
/// content taken from the operator's own client, never committed.
const IMPORT_MARKER: &[u8] = b"# openm2 content import";

/// How every TOML file `openm2 assets import` writes starts
/// (`m2_assets::folder::IMPORT_MARKER`): a player's client converted, never
/// committed.
const ASSETS_MARKER: &[u8] = b"# openm2 assets import";

/// The project's own logo (`branding/README.md`): the one folder whose
/// pictures, `.png` files directly in it, may be committed.
const BRANDING: &str = "branding/";

const MAX_FILE_BYTES: u64 = 1024 * 1024;
const MAX_FIXTURE_BYTES: u64 = 64 * 1024;

/// `just no-assets`
pub fn no_assets() -> Result<()> {
    let root = repo_root();
    let listing = output(
        "git",
        &[
            "ls-files",
            "--cached",
            "--others",
            "--exclude-standard",
            "-z",
        ],
    )?;
    let files: BTreeSet<&str> = listing.split('\0').filter(|s| !s.is_empty()).collect();
    let mut problems = Vec::new();

    for file in files {
        let path = root.join(file);
        let Ok(meta) = std::fs::metadata(&path) else {
            continue; // deleted in the working tree
        };
        // Any `fixtures/` folder: top level or a crate's `tests/fixtures/`.
        let in_fixtures = file.starts_with("fixtures/") || file.contains("/fixtures/");
        let lower = file.to_ascii_lowercase();
        let extension = lower.rsplit_once('.').map(|(_, ext)| ext).unwrap_or("");
        let file_name = lower.rsplit('/').next().unwrap_or(&lower);

        let own_logo = extension == "png"
            && file
                .strip_prefix(BRANDING)
                .is_some_and(|name| !name.contains('/'));
        if ASSET_EXTENSIONS.contains(&extension) && !own_logo {
            problems.push(format!(
                "{file}: game client file or picture type `.{extension}`"
            ));
        }
        if file_name == "keys.toml" && !in_fixtures {
            problems.push(format!(
                "{file}: archive keys belong to the user, not the repo"
            ));
        }
        let limit = if in_fixtures {
            MAX_FIXTURE_BYTES
        } else {
            MAX_FILE_BYTES
        };
        if meta.len() > limit {
            problems.push(format!(
                "{file}: {} bytes is over the {limit}-byte limit",
                meta.len()
            ));
        }
        let mut head = [0u8; 32];
        let read = std::fs::File::open(&path)
            .and_then(|mut f| f.read(&mut head))
            .unwrap_or(0);
        let head = head.get(..read).unwrap_or_default();
        if let Some(magic) = head.first_chunk::<4>()
            && !in_fixtures
            && ASSET_MAGICS.contains(&magic)
        {
            problems.push(format!(
                "{file}: starts with `{}`, a client archive/data magic",
                magic.escape_ascii()
            ));
        }
        if head.starts_with(IMPORT_MARKER) {
            problems.push(format!(
                "{file}: content imported from a client; keep it in local/"
            ));
        }
        if head.starts_with(ASSETS_MARKER) {
            problems.push(format!(
                "{file}: assets imported from a client; keep them in local/"
            ));
        }
    }

    if !problems.is_empty() {
        bail!(
            "files that must not be committed (see docs/source-policy.md):\n  - {}",
            problems.join("\n  - ")
        );
    }
    println!("no game assets, keys or oversized files in the tree");
    Ok(())
}

/// Leftover insta files mean a snapshot change was not reviewed.
pub fn snapshot_leftovers() -> Result<()> {
    let listing = output("git", &["ls-files", "--others", "-z"])?;
    let leftovers: Vec<&str> = listing
        .split('\0')
        .filter(|f| f.ends_with(".snap.new") || f.ends_with(".pending-snap"))
        .collect();
    if !leftovers.is_empty() {
        bail!(
            "unreviewed snapshot changes: {}\nreview them (`cargo insta review`) and explain the diff in the handoff",
            leftovers.join(", ")
        );
    }
    println!("no unreviewed snapshots");
    Ok(())
}

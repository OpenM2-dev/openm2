//! The launcher's import of a player's client
//! (`docs/specs/update.md`, "What the launcher does"): `m2-launcher`'s own
//! functions with this `openm2` as the game, on a synthetic client. The
//! launcher's choices around them (a version with its own files, a game
//! that doesn't import) are tested in `m2-launcher`. And the device's key a
//! local save's server makes its game master ("Local saves").

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use m2_formats::{ArchiveWriter, EntryKind, Keys};
use m2_launcher::process::Stop;
use m2_launcher::{CONTENT, IMPORT_LOG, IMPORTED, IMPORTING, Imported, Install, Progress};

/// A folder for one test, deleted after it.
struct Dir(PathBuf);

impl Dir {
    fn new(name: &str) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!(
            "openm2-launcher-{name}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        Self(root)
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// A client whose one archive holds a texture nothing names: the import
/// writes only its `assets.toml`.
fn client(dir: &Path) -> PathBuf {
    let keys = Keys::none();
    let mut pack = ArchiveWriter::new();
    pack.add(
        b"d:/ymir work/unused.txt",
        b"nothing names this",
        EntryKind::Lzo,
        &keys,
    )
    .unwrap();
    let client = dir.join("client");
    fs::create_dir_all(client.join("pack")).unwrap();
    let (eix, epk) = pack.finish(None).unwrap();
    fs::write(client.join("pack/test.eix"), eix).unwrap();
    fs::write(client.join("pack/test.epk"), epk).unwrap();
    client
}

fn game() -> &'static Path {
    Path::new(env!("CARGO_BIN_EXE_openm2"))
}

/// The number `assets.toml` of the install folder's import gives.
fn import_number(root: &Path) -> String {
    let text = fs::read_to_string(root.join(IMPORTED).join("assets.toml")).unwrap();
    let line = text.lines().find(|line| line.starts_with("import = "));
    line.unwrap_or_else(|| panic!("no import number in:\n{text}"))
        .trim_start_matches("import = ")
        .to_owned()
}

/// The first start imports the client; the next finds it current; an
/// import of another number is imported again; a failed import leaves the
/// last one; an import that stopped part-way goes at the next start.
#[test]
fn the_launcher_imports_the_client_and_again_when_the_import_changes() {
    let dir = Dir::new("import");
    let root = dir.0.join("install");
    fs::create_dir_all(&root).unwrap();
    let client = client(&dir.0);
    let keys = dir.0.join("keys.toml");
    fs::write(&keys, "").unwrap();
    let state = || m2_launcher::imported(game(), &root).unwrap();
    assert_eq!(state(), Imported::Stale("no import yet".to_owned()));
    // The saves' world, from the client before: imported again after.
    fs::create_dir_all(root.join(CONTENT)).unwrap();

    let mut said = Vec::new();
    m2_launcher::import(
        game(),
        &root,
        &client,
        &keys,
        &Stop::default(),
        &mut |progress| {
            said.push(progress);
        },
    )
    .unwrap();
    // What the game wrote as it went, then its report's first line, then
    // the check.
    let report = said
        .iter()
        .position(|progress| matches!(progress, Progress::Report(line) if line.starts_with("imported 0 actors")));
    let check = said
        .iter()
        .position(|progress| *progress == Progress::CheckingImport);
    assert!(report.is_some() && report < check, "{said:?}");
    assert_eq!(state(), Imported::Current);
    assert!(!root.join(IMPORTING).exists());
    assert!(
        !root.join(CONTENT).exists(),
        "the saves' world goes with it"
    );
    let log = fs::read_to_string(root.join(IMPORT_LOG)).unwrap();
    assert!(log.starts_with("imported 0 actors"), "{log}");
    assert!(
        log.contains("every file reads, and every file they name is there"),
        "{log}"
    );

    // An update whose import writes something else has another number.
    let number = import_number(&root);
    let folder_file = root.join(IMPORTED).join("assets.toml");
    let text = fs::read_to_string(&folder_file).unwrap();
    fs::write(
        &folder_file,
        text.replace(&format!("import = {number}\n"), "import = 0\n"),
    )
    .unwrap();
    assert_eq!(
        state(),
        Imported::Stale(format!(
            "this version reads clients differently (import 0, now {number})"
        ))
    );
    m2_launcher::import(game(), &root, &client, &keys, &Stop::default(), &mut |_| {}).unwrap();
    assert_eq!(state(), Imported::Current);
    assert_eq!(import_number(&root), number);

    // A failed import leaves the last one as it was.
    let missing = dir.0.join("no client here");
    let error = m2_launcher::import(
        game(),
        &root,
        &missing,
        &keys,
        &Stop::default(),
        &mut |_| {},
    )
    .unwrap_err();
    assert!(
        format!("{error:#}").contains("importing your client failed"),
        "{error:#}"
    );
    assert_eq!(state(), Imported::Current);
    assert!(!root.join(IMPORTING).exists());

    // What an import that stopped part-way left goes at the next start.
    fs::create_dir_all(root.join(IMPORTING).join("ymir work")).unwrap();
    fs::write(root.join(IMPORTING).join("assets.toml"), "format = 1\n").unwrap();
    drop(Install::open(&root).unwrap());
    assert!(!root.join(IMPORTING).exists());
    assert_eq!(state(), Imported::Current);
}

/// The device's key is made beside the install folder's `settings.toml`
/// the first time, and the same one comes back after.
#[test]
fn a_local_saves_owner_is_this_devices_key() {
    let dir = Dir::new("owner");
    let root = dir.0.join("install");
    fs::create_dir_all(&root).unwrap();
    let key = m2_launcher::host::owner_key(game(), &root).unwrap();
    assert!(key.starts_with("ed25519:") && key.len() == 8 + 64, "{key}");
    assert!(root.join("device-key.pem").is_file());
    assert_eq!(m2_launcher::host::owner_key(game(), &root).unwrap(), key);
}

//! `openm2 release` (`docs/specs/update.md`): a key made, a folder's manifest
//! written and signed, and the signed file checked, as a game's CI does.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use m2_update::{Bundle, Manifest, Place, PublicKey, Sha256};

fn openm2(args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_openm2"));
    command.args(args).env_remove("OPENM2_RELEASE_KEY");
    command
}

fn text(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("openm2-release-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn path(path: &Path) -> &str {
    path.to_str().unwrap()
}

#[test]
fn a_key_a_manifest_a_signature_and_a_check() {
    let dir = temp("all");
    let key_file = dir.join("release.pem");

    // The key: written once, its public half printed.
    let output = openm2(&["release", "keygen", "--out", path(&key_file)])
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", text(&output));
    let printed = String::from_utf8_lossy(&output.stdout);
    let public = printed
        .lines()
        .find_map(|line| line.strip_prefix("key = \"")?.strip_suffix('"'))
        .unwrap()
        .to_owned();
    let public_key: PublicKey = public.parse().unwrap();
    let pem = fs::read_to_string(&key_file).unwrap();
    assert!(pem.starts_with("-----BEGIN PRIVATE KEY-----"));
    assert!(!printed.contains(pem.lines().nth(1).unwrap()), "{printed}");
    let again = openm2(&["release", "keygen", "--out", path(&key_file)])
        .output()
        .unwrap();
    assert!(!again.status.success());
    assert_eq!(fs::read_to_string(&key_file).unwrap(), pem);

    // The manifest of a version's folder, and its files named by SHA-256.
    let version = dir.join("version");
    fs::create_dir_all(version.join("packs")).unwrap();
    fs::write(version.join("openm2.exe"), b"the game").unwrap();
    fs::write(version.join("packs").join("trees.bin"), b"trees").unwrap();
    let manifest_file = dir.join("manifest.json");
    let objects = dir.join("objects");
    let output = openm2(&[
        "release",
        "manifest",
        "--dir",
        path(&version),
        "--channel",
        "test",
        "--version",
        "1.2.0",
        "--sequence",
        "42",
        "--days",
        "30",
        "--out",
        path(&manifest_file),
        "--objects",
        path(&objects),
    ])
    .output()
    .unwrap();
    assert!(output.status.success(), "{}", text(&output));
    assert!(
        text(&output).contains("sequence 42, version 1.2.0"),
        "{}",
        text(&output)
    );
    let manifest = Manifest::from_json(&fs::read(&manifest_file).unwrap()).unwrap();
    let paths: Vec<&str> = manifest.files().iter().map(|f| f.path.as_str()).collect();
    assert_eq!(paths, ["openm2.exe", "packs/trees.bin"]);
    for bytes in [b"the game".as_slice(), b"trees"] {
        let object = objects.join(Sha256::of(bytes).to_string());
        assert_eq!(fs::read(object).unwrap(), bytes);
    }

    // Signing needs the key in the environment, never on the command line.
    let signed_file = dir.join("manifest.signed");
    let sign = || {
        let mut command = openm2(&[
            "release",
            "sign",
            path(&manifest_file),
            "--out",
            path(&signed_file),
        ]);
        command.env("OPENM2_RELEASE_KEY", &pem);
        command
    };
    let output = openm2(&[
        "release",
        "sign",
        path(&manifest_file),
        "--out",
        path(&signed_file),
    ])
    .output()
    .unwrap();
    assert!(!output.status.success());
    assert!(
        text(&output).contains("OPENM2_RELEASE_KEY"),
        "{}",
        text(&output)
    );
    let output = sign().output().unwrap();
    assert!(output.status.success(), "{}", text(&output));
    assert!(text(&output).contains(&public), "{}", text(&output));
    assert!(!text(&output).contains(pem.lines().nth(1).unwrap()));
    let signed = fs::read(&signed_file).unwrap();
    assert_eq!(m2_update::open(&signed, &public_key).unwrap(), manifest);

    // The check: the right key and channel pass, anything else doesn't.
    let verify = |key: &str, channel: Option<&str>| {
        let mut args = vec!["release", "verify", path(&signed_file), "--key", key];
        if let Some(channel) = channel {
            args.extend(["--channel", channel]);
        }
        openm2(&args).output().unwrap()
    };
    let output = verify(&public, None);
    assert!(output.status.success(), "{}", text(&output));
    assert!(
        text(&output).contains("channel test, sequence 42"),
        "{}",
        text(&output)
    );
    assert!(verify(&public, Some("test")).status.success());
    let output = verify(&public, Some("live"));
    assert_eq!(output.status.code(), Some(1));
    assert!(text(&output).contains("refused"), "{}", text(&output));
    let other = "ed25519:d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a";
    let output = verify(other, None);
    assert!(!output.status.success());
    assert!(text(&output).contains("signature"), "{}", text(&output));
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_folder_with_a_bad_name_has_no_manifest() {
    let dir = temp("bad-name");
    let version = dir.join("version");
    fs::create_dir_all(version.join(".git")).unwrap();
    fs::write(version.join(".git").join("HEAD"), b"x").unwrap();
    let out = dir.join("manifest.json");
    let output = openm2(&[
        "release",
        "manifest",
        "--dir",
        path(&version),
        "--channel",
        "test",
        "--version",
        "1",
        "--out",
        path(&out),
    ])
    .output()
    .unwrap();
    assert!(!output.status.success());
    assert!(text(&output).contains(".git/HEAD"), "{}", text(&output));
    assert!(!out.exists());
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn imported_files_stay_off_channels_unless_allowed() {
    let dir = temp("imported");
    let version = dir.join("version");
    fs::create_dir_all(version.join("assets")).unwrap();
    fs::write(version.join("openm2.exe"), b"the game").unwrap();
    fs::write(
        version.join("assets").join("warrior.actor.toml"),
        format!("{}\nformat = 1\n", m2_assets::folder::IMPORT_MARKER),
    )
    .unwrap();
    let manifest = |extra: &[&str]| {
        let out = dir.join("manifest.json");
        let mut args = vec![
            "release",
            "manifest",
            "--dir",
            path(&version),
            "--channel",
            "local",
            "--version",
            "1.0.0",
            "--out",
            path(&out),
        ];
        args.extend_from_slice(extra);
        openm2(&args).output().unwrap()
    };
    let refused = manifest(&[]);
    assert!(!refused.status.success());
    let message = text(&refused);
    assert!(
        message.contains("assets/warrior.actor.toml (openm2 assets import)")
            && message.contains("--allow-imported"),
        "{message}"
    );
    let allowed = manifest(&["--allow-imported"]);
    assert!(allowed.status.success(), "{}", text(&allowed));
}

/// The files of a folder, by name.
fn objects_in(dir: &Path) -> Vec<(String, Vec<u8>)> {
    let mut found: Vec<(String, Vec<u8>)> = fs::read_dir(dir)
        .unwrap()
        .map(|entry| {
            let entry = entry.unwrap();
            (
                entry.file_name().into_string().unwrap(),
                fs::read(entry.path()).unwrap(),
            )
        })
        .collect();
    found.sort();
    found
}

#[test]
fn bundles_and_a_manifest_made_elsewhere() {
    let dir = temp("bundles");
    // A folder of small files, bundled, as where an asset folder is made.
    let world = dir.join("world");
    fs::create_dir_all(world.join("assets")).unwrap();
    for (name, bytes) in [
        ("b.toml", b"b = 1\n".as_slice()),
        ("a.toml", b"a = 1\n"),
        ("copy.toml", b"a = 1\n"),
        ("empty.txt", b""),
    ] {
        fs::write(world.join("assets").join(name), bytes).unwrap();
    }
    let manifest = |folder: &Path, out: &Path, objects: &Path, extra: &[&str]| {
        let mut args = vec![
            "release",
            "manifest",
            "--dir",
            path(folder),
            "--channel",
            "test",
            "--version",
            "1.0",
            "--sequence",
            "7",
            "--out",
            path(out),
            "--objects",
            path(objects),
        ];
        args.extend_from_slice(extra);
        openm2(&args).output().unwrap()
    };
    let world_json = dir.join("world.json");
    let world_objects = dir.join("world-objects");
    let output = manifest(&world, &world_json, &world_objects, &["--bundles"]);
    assert!(output.status.success(), "{}", text(&output));
    assert!(
        text(&output).contains("4 files, 18 bytes, 1 bundle"),
        "{}",
        text(&output)
    );
    let bundled = Manifest::from_json(&fs::read(&world_json).unwrap()).unwrap();
    // Each content once, in the order of the paths; the empty file alone.
    let bundle = b"a = 1\nb = 1\n";
    assert_eq!(
        bundled.bundles(),
        [Bundle {
            size: 12,
            sha256: Sha256::of(bundle)
        }]
    );
    let places: Vec<(&str, Option<Place>)> = bundled
        .files()
        .iter()
        .map(|file| (file.path.as_str(), file.place))
        .collect();
    let at = |offset| Some(Place { bundle: 0, offset });
    assert_eq!(
        places,
        [
            ("assets/a.toml", at(0)),
            ("assets/b.toml", at(6)),
            ("assets/copy.toml", at(0)),
            ("assets/empty.txt", None),
        ]
    );
    let mut wanted = vec![
        (Sha256::of(bundle).to_string(), bundle.to_vec()),
        (Sha256::of(b"").to_string(), Vec::new()),
    ];
    wanted.sort();
    assert_eq!(objects_in(&world_objects), wanted);
    // The same folder gives the same bundles again (the expiry, from the
    // time now, may differ).
    let again = manifest(&world, &world_json, &world_objects, &["--bundles"]);
    assert!(again.status.success(), "{}", text(&again));
    let again = Manifest::from_json(&fs::read(&world_json).unwrap()).unwrap();
    assert_eq!(
        (again.files(), again.bundles()),
        (bundled.files(), bundled.bundles())
    );
    assert_eq!(objects_in(&world_objects), wanted);

    // The program's folder, with the bundled manifest added.
    let app = dir.join("app");
    fs::create_dir_all(&app).unwrap();
    fs::write(app.join("openm2.exe"), b"the game").unwrap();
    let all_json = dir.join("manifest.json");
    let app_objects = dir.join("app-objects");
    let output = manifest(&app, &all_json, &app_objects, &["--add", path(&world_json)]);
    assert!(output.status.success(), "{}", text(&output));
    let all = Manifest::from_json(&fs::read(&all_json).unwrap()).unwrap();
    assert_eq!(all.files().len(), 5);
    assert_eq!(all.bundles(), bundled.bundles());
    assert_eq!(all.file("assets/b.toml").unwrap().place, at(6));
    assert_eq!(all.file("openm2.exe").unwrap().place, None);
    assert_eq!(
        objects_in(&app_objects),
        [(Sha256::of(b"the game").to_string(), b"the game".to_vec())]
    );
    // A path in both is refused, and nothing is written.
    fs::create_dir_all(app.join("assets")).unwrap();
    fs::write(app.join("assets").join("A.toml"), b"clash").unwrap();
    fs::remove_file(&all_json).unwrap();
    let clash_objects = dir.join("clash-objects");
    let output = manifest(
        &app,
        &all_json,
        &clash_objects,
        &["--add", path(&world_json)],
    );
    assert!(!output.status.success());
    assert!(text(&output).contains("twice"), "{}", text(&output));
    assert!(!all_json.exists());
    assert_eq!(objects_in(&clash_objects), []);
    fs::remove_dir_all(&dir).unwrap();
}

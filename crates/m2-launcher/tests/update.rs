//! Updates end to end (`docs/specs/update.md`, "What the launcher does"):
//! a channel served over HTTP on 127.0.0.1, an install folder in the
//! system's temporary folder, and `openm2-launcher` itself.

use std::collections::BTreeMap;
use std::fs;
use std::io::{Read as _, Write as _};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use m2_launcher::{
    AT_ONCE, Http, Install, LAUNCHER_FILE, LauncherSettings, Outcome, Progress, RECORD,
    replace_launcher,
};
use m2_update::{
    Bundle, FileEntry, InstallPath, Manifest, Sha256, SigningKey, Timestamp, UpdateError,
    cut_bundles, place_bundled,
};

/// The channel's files, by place, and what was asked for.
#[derive(Default)]
struct Served {
    files: BTreeMap<String, Vec<u8>>,
    /// Places whose answer stops after this many bytes of the body.
    cut: BTreeMap<String, usize>,
    asked: Vec<String>,
    /// Ranges answered with the whole file, as a server that doesn't take
    /// them answers.
    whole: bool,
    /// How long an answer for a file waits before it's sent.
    slow: Duration,
    /// The bytes of files sent, since `sent`.
    sent: usize,
    /// The answers being sent now, and the most at once.
    busy: usize,
    most: usize,
    /// Every request answered with a redirect (`302`) to this address
    /// and its place, as GitHub's `releases/latest/download/` answers.
    forward: Option<String>,
}

/// A channel's server: plain HTTP/1.1, one request per connection, each
/// connection on a thread of its own. It answers ranges (`206`).
struct Server {
    url: String,
    served: Arc<Mutex<Served>>,
}

impl Server {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/channels/", listener.local_addr().unwrap());
        let served = Arc::new(Mutex::new(Served::default()));
        let shared = served.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { continue };
                let shared = shared.clone();
                std::thread::spawn(move || {
                    let _ = answer(stream, &shared);
                });
            }
        });
        Self { url, served }
    }

    fn put(&self, place: &str, bytes: &[u8]) {
        let mut served = self.served.lock().unwrap();
        served.files.insert(place.to_owned(), bytes.to_vec());
        served.cut.remove(place);
    }

    fn cut(&self, place: &str, after: usize) {
        self.served
            .lock()
            .unwrap()
            .cut
            .insert(place.to_owned(), after);
    }

    /// From now on, ranges are answered with the whole file.
    fn ignore_ranges(&self) {
        self.served.lock().unwrap().whole = true;
    }

    /// From now on, every request is sent on to `to` and its place.
    fn forward(&self, to: &str) {
        self.served.lock().unwrap().forward = Some(to.to_owned());
    }

    /// Publishes a version as a flat channel names it
    /// (`<channel>.manifest.signed` and `<sha256>`): its files, then its
    /// signed manifest.
    fn publish_flat(&self, key: &SigningKey, manifest: &Manifest, contents: &[(&str, &[u8])]) {
        for (_, bytes) in contents {
            self.put(&Sha256::of(bytes).to_string(), bytes);
        }
        let signed = m2_update::sign(&manifest.to_json(), key).unwrap();
        self.put(&format!("{}.manifest.signed", manifest.channel()), &signed);
    }

    /// From now on, every answer for a file waits `wait` first.
    fn slow(&self, wait: Duration) {
        self.served.lock().unwrap().slow = wait;
    }

    /// How many times `place` was asked for, since `files_asked`.
    fn asked(&self, place: &str) -> usize {
        let served = self.served.lock().unwrap();
        served.asked.iter().filter(|asked| *asked == place).count()
    }

    /// How many files (not manifests) were asked for, and forget them all.
    fn files_asked(&self) -> usize {
        let mut served = self.served.lock().unwrap();
        let count = served
            .asked
            .iter()
            .filter(|asked| asked.starts_with("files/"))
            .count();
        served.asked.clear();
        count
    }

    /// How many bytes of files were sent since the last time.
    fn sent(&self) -> usize {
        std::mem::take(&mut self.served.lock().unwrap().sent)
    }

    /// The most answers sent at once.
    fn most(&self) -> usize {
        self.served.lock().unwrap().most
    }

    /// Publishes a version: its files, then its signed manifest.
    fn publish(&self, key: &SigningKey, manifest: &Manifest, contents: &[(&str, &[u8])]) {
        let objects: Vec<Vec<u8>> = contents.iter().map(|(_, bytes)| bytes.to_vec()).collect();
        self.publish_objects(key, manifest, &objects);
    }

    /// Publishes a version: its objects (files and bundles), then its
    /// signed manifest.
    fn publish_objects(&self, key: &SigningKey, manifest: &Manifest, objects: &[Vec<u8>]) {
        for bytes in objects {
            self.put(&format!("files/{}", Sha256::of(bytes)), bytes);
        }
        let signed = m2_update::sign(&manifest.to_json(), key).unwrap();
        self.put(&format!("{}/manifest.signed", manifest.channel()), &signed);
    }
}

fn answer(mut stream: TcpStream, served: &Mutex<Served>) -> std::io::Result<()> {
    let mut request = Vec::new();
    let mut byte = [0u8; 1];
    while !request.ends_with(b"\r\n\r\n") && request.len() < 8192 {
        if stream.read(&mut byte)? == 0 {
            return Ok(());
        }
        request.push(byte[0]);
    }
    let text = String::from_utf8_lossy(&request);
    let path = text.split(' ').nth(1).unwrap_or("/");
    let place = path.strip_prefix("/channels/").unwrap_or(path).to_owned();
    let range = text.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        let (first, last) = name
            .eq_ignore_ascii_case("range")
            .then_some(value.trim())?
            .strip_prefix("bytes=")?
            .split_once('-')?;
        Some((first.parse::<usize>().ok()?, last.parse::<usize>().ok()?))
    });
    let file = place.starts_with("files/");
    let (body, cut, whole, slow, forward) = {
        let mut served = served.lock().unwrap();
        served.asked.push(place.clone());
        served.busy += 1;
        served.most = served.most.max(served.busy);
        (
            served.files.get(&place).cloned(),
            served.cut.get(&place).copied(),
            served.whole,
            served.slow,
            served.forward.clone(),
        )
    };
    if let Some(to) = forward {
        served.lock().unwrap().busy -= 1;
        write!(
            stream,
            "HTTP/1.1 302 Found
Location: {to}{place}
Content-Length: 0
Connection: close

"
        )?;
        return stream.flush();
    }
    let answered = (|| {
        if file {
            std::thread::sleep(slow);
        }
        let Some(body) = body else {
            return stream.write_all(
                b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            );
        };
        let (status, headers, body) = match range {
            Some((first, last)) if !whole && first < body.len() => {
                let last = last.min(body.len() - 1);
                (
                    "206 Partial Content",
                    format!("Content-Range: bytes {first}-{last}/{}\r\n", body.len()),
                    body[first..=last].to_vec(),
                )
            }
            _ => ("200 OK", String::new(), body),
        };
        write!(
            stream,
            "HTTP/1.1 {status}\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )?;
        let sent = &body[..cut.unwrap_or(body.len()).min(body.len())];
        stream.write_all(sent)?;
        if file {
            served.lock().unwrap().sent += sent.len();
        }
        stream.flush()
    })();
    served.lock().unwrap().busy -= 1;
    answered
}

fn key() -> SigningKey {
    SigningKey::from_seed(&[7; 32]).unwrap()
}

fn entry(path: &str, bytes: &[u8]) -> FileEntry {
    FileEntry {
        path: InstallPath::new(path).unwrap(),
        size: bytes.len() as u64,
        sha256: Sha256::of(bytes),
        place: None,
    }
}

/// A manifest of the test channel that expires in 2100.
fn manifest(sequence: u64, version: &str, contents: &[(&str, &[u8])]) -> Manifest {
    manifest_of("test", sequence, version, "2100-01-01T00:00:00Z", contents)
}

fn manifest_of(
    channel: &str,
    sequence: u64,
    version: &str,
    expires: &str,
    contents: &[(&str, &[u8])],
) -> Manifest {
    let files = contents
        .iter()
        .map(|(path, bytes)| entry(path, bytes))
        .collect();
    Manifest::new(
        channel,
        sequence,
        version,
        expires.parse().unwrap(),
        files,
        Vec::new(),
    )
    .unwrap()
}

/// A manifest of the test channel whose files under `assets/` are bundled
/// as `openm2 release manifest --bundles` cuts them, and the objects to
/// publish with it: the bundles, then the files on their own.
fn bundled(sequence: u64, version: &str, contents: &[(&str, &[u8])]) -> (Manifest, Vec<Vec<u8>>) {
    let mut sorted = contents.to_vec();
    sorted.sort_by_key(|(path, _)| *path);
    let mut files: Vec<FileEntry> = sorted
        .iter()
        .map(|(path, bytes)| entry(path, bytes))
        .collect();
    let assets: Vec<usize> = (0..sorted.len())
        .filter(|&i| sorted[i].0.starts_with("assets/"))
        .collect();
    let picked: Vec<FileEntry> = assets.iter().map(|&i| files[i].clone()).collect();
    let cut: Vec<Vec<usize>> = cut_bundles(&picked)
        .iter()
        .map(|bundle| bundle.iter().map(|&i| assets[i]).collect())
        .collect();
    place_bundled(&mut files, &cut);
    let mut objects: Vec<Vec<u8>> = cut
        .iter()
        .map(|bundle| bundle.iter().flat_map(|&i| sorted[i].1.to_vec()).collect())
        .collect();
    let bundles = objects
        .iter()
        .map(|bytes| Bundle {
            size: bytes.len() as u64,
            sha256: Sha256::of(bytes),
        })
        .collect();
    let manifest = Manifest::new(
        "test",
        sequence,
        version,
        "2100-01-01T00:00:00Z".parse().unwrap(),
        files,
        bundles,
    )
    .unwrap();
    objects.extend(
        sorted
            .iter()
            .filter(|(path, _)| !path.starts_with("assets/"))
            .map(|(_, bytes)| bytes.to_vec()),
    );
    (manifest, objects)
}

/// An empty install folder with its `launcher.toml`.
fn folder(name: &str, server: &Server, key: &SigningKey) -> PathBuf {
    let root = std::env::temp_dir().join(format!("openm2-launcher-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    fs::write(
        root.join(LAUNCHER_FILE),
        format!(
            "format = 1\nurl = \"{}\"\nchannel = \"test\"\nkey = \"{}\"\n",
            server.url,
            key.public_key()
        ),
    )
    .unwrap();
    root
}

fn now() -> Timestamp {
    "2026-10-22T00:00:00Z".parse().unwrap()
}

/// Takes `root`'s lock, as the launcher does at its start. On Unix, a
/// child process another test starts holds a copy of every open file for
/// the moment between its fork and its exec, the lock's file among them: a
/// folder whose lock was just let go may still look locked then. So this
/// tries again for up to a second; one launcher per folder never meets it.
fn open(root: &Path) -> anyhow::Result<Install> {
    for _ in 0..100 {
        match Install::open(root) {
            Err(error) if error.to_string().starts_with("another launcher is running") => {
                std::thread::sleep(Duration::from_millis(10));
            }
            opened => return opened,
        }
    }
    Install::open(root)
}

fn update(root: &Path) -> anyhow::Result<Outcome> {
    let settings = LauncherSettings::read(&root.join(LAUNCHER_FILE)).unwrap();
    let install = open(root)?;
    install.update(
        &settings,
        &Http::new(&settings.url, settings.layout)?,
        now(),
        &mut |_| {},
    )
}

/// Every file under `folder` and its contents, by path with `/`.
fn contents(folder: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(folder: &Path, prefix: &str, out: &mut BTreeMap<String, Vec<u8>>) {
        for entry in fs::read_dir(folder).unwrap() {
            let entry = entry.unwrap();
            let name = format!("{prefix}{}", entry.file_name().to_str().unwrap());
            if entry.file_type().unwrap().is_dir() {
                walk(&entry.path(), &format!("{name}/"), out);
            } else {
                out.insert(name, fs::read(entry.path()).unwrap());
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(folder, "", &mut out);
    out
}

/// The files of a version as they should be on disk, without its record.
fn expected(files: &[(&str, &[u8])]) -> BTreeMap<String, Vec<u8>> {
    files
        .iter()
        .map(|(path, bytes)| ((*path).to_owned(), bytes.to_vec()))
        .collect()
}

fn without_record(mut files: BTreeMap<String, Vec<u8>>) -> BTreeMap<String, Vec<u8>> {
    files.remove(RECORD);
    files
}

const V1: [(&str, &[u8]); 3] = [
    ("openm2.exe", b"game one"),
    ("openm2.toml", b"format = 1\n"),
    ("packs/a.txt", b"aaa"),
];
const V2: [(&str, &[u8]); 3] = [
    ("openm2.exe", b"game two"),
    ("openm2.toml", b"format = 1\n"),
    ("packs/b/c.txt", b"ccc"),
];

#[test]
fn a_first_install_then_an_update_then_nothing_to_do() {
    let (server, key) = (Server::start(), key());
    let root = folder("update", &server, &key);
    server.publish(&key, &manifest(1, "1.0", &V1), &V1);
    let Outcome::Updated {
        manifest: installed,
        downloaded,
        bytes,
    } = update(&root).unwrap()
    else {
        panic!("not an update");
    };
    assert_eq!((installed.version(), downloaded, bytes), ("1.0", 3, 22));
    assert_eq!(server.files_asked(), 3);
    assert_eq!(without_record(contents(&root.join("app"))), expected(&V1));
    assert!(!root.join("staging").exists());

    // Only what changed is downloaded; what's gone is gone.
    server.publish(&key, &manifest(2, "2.0", &V2), &V2);
    let Outcome::Updated { downloaded, .. } = update(&root).unwrap() else {
        panic!("not an update");
    };
    assert_eq!(downloaded, 2);
    assert_eq!(server.files_asked(), 2);
    assert_eq!(without_record(contents(&root.join("app"))), expected(&V2));
    assert_eq!(
        without_record(contents(&root.join("previous"))),
        expected(&V1)
    );

    // The same manifest again: only it is asked for.
    assert!(matches!(update(&root).unwrap(), Outcome::Current(_)));
    assert_eq!(server.asked("test/manifest.signed"), 1);
    assert_eq!(server.files_asked(), 0);
    fs::remove_dir_all(&root).unwrap();
}

/// A flat channel (`layout = "flat"`) behind a redirect, as a GitHub
/// release's downloads are: `releases/latest/download/<name>` answers
/// `302` to the file's own address.
#[test]
fn a_flat_channel_behind_a_redirect() {
    let (front, files, key) = (Server::start(), Server::start(), key());
    front.forward(&files.url);
    let root = std::env::temp_dir().join(format!("openm2-launcher-flat-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let settings = |url: &str| {
        format!(
            "format = 1
url = \"{url}\"
channel = \"test\"
layout = \"flat\"
key = \"{}\"
",
            key.public_key()
        )
    };
    fs::write(root.join(LAUNCHER_FILE), settings(&front.url)).unwrap();

    files.publish_flat(&key, &manifest(1, "1.0", &V1), &V1);
    let Outcome::Updated { downloaded, .. } = update(&root).unwrap() else {
        panic!("not an update");
    };
    assert_eq!(downloaded, 3);
    assert_eq!(without_record(contents(&root.join("app"))), expected(&V1));
    assert_eq!(front.asked("test.manifest.signed"), 1);
    assert_eq!(files.asked("test.manifest.signed"), 1);
    let game_two = Sha256::of(b"game two").to_string();
    assert_eq!(front.asked(&game_two), 0);

    // Only what changed comes, by its SHA-256 alone.
    files.publish_flat(&key, &manifest(2, "2.0", &V2), &V2);
    let Outcome::Updated { downloaded, .. } = update(&root).unwrap() else {
        panic!("not an update");
    };
    assert_eq!(downloaded, 2);
    assert_eq!(front.asked(&game_two), 1);
    assert_eq!(files.asked(&game_two), 1);
    assert_eq!(without_record(contents(&root.join("app"))), expected(&V2));

    // A manifest whose files aren't there yet (a release published between
    // the two requests) fails the update and leaves the installed version.
    let v3: [(&str, &[u8]); 1] = [("openm2.exe", b"game three")];
    let signed = m2_update::sign(&manifest(3, "3.0", &v3).to_json(), &key).unwrap();
    files.put("test.manifest.signed", &signed);
    let error = format!("{:#}", update(&root).unwrap_err());
    assert!(error.contains("404"), "{error}");
    assert_eq!(without_record(contents(&root.join("app"))), expected(&V2));

    // A redirect away from HTTPS is refused before anything is asked there.
    front.forward("http://example.net/");
    let error = format!("{:#}", update(&root).unwrap_err());
    assert!(error.contains("isn't https://"), "{error}");
    assert_eq!(without_record(contents(&root.join("app"))), expected(&V2));
    fs::remove_dir_all(&root).unwrap();
}

/// What an update says as it goes: the window's gauge and the console's
/// lines come from it.
#[test]
fn an_update_says_how_far_it_has_got() {
    let (server, key) = (Server::start(), key());
    let root = folder("progress", &server, &key);
    let big = large();
    let files: [(&str, &[u8]); 3] = [
        ("openm2.exe", b"game one"),
        (
            "openm2.toml",
            b"format = 1
",
        ),
        ("packs/big.bin", &big),
    ];
    server.publish(&key, &manifest(1, "1.0", &files), &files);
    let said = |root: &Path| {
        let settings = LauncherSettings::read(&root.join(LAUNCHER_FILE)).unwrap();
        let install = open(root).unwrap();
        let mut said = Vec::new();
        install
            .update(
                &settings,
                &Http::new(&settings.url, settings.layout).unwrap(),
                now(),
                &mut |progress| {
                    said.push(progress);
                },
            )
            .unwrap();
        said
    };
    let first = said(&root);
    assert!(
        matches!(first.first(), Some(Progress::Looking { url }) if url.as_str() == server.url),
        "{first:?}"
    );
    let downloads: Vec<(u64, u64)> = first
        .iter()
        .filter_map(|progress| match progress {
            Progress::Downloading { done, total, .. } => Some((*done, *total)),
            _ => None,
        })
        .collect();
    let total = (8 + 11 + big.len()) as u64;
    assert_eq!(downloads.first(), Some(&(0, total)), "{downloads:?}");
    assert_eq!(downloads.last(), Some(&(total, total)), "{downloads:?}");
    assert!(
        downloads.windows(2).all(|pair| pair[0].0 <= pair[1].0),
        "never back: {downloads:?}"
    );
    assert!(first.contains(&Progress::Swapping), "{first:?}");

    // Nothing to fetch: nothing downloads.
    let again = said(&root);
    assert!(
        !again
            .iter()
            .any(|progress| matches!(progress, Progress::Downloading { .. })),
        "{again:?}"
    );
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn an_interrupted_download_goes_on_where_it_stopped() {
    let (server, key) = (Server::start(), key());
    let root = folder("interrupted", &server, &key);
    let one = vec![1u8; 200_000];
    let two = vec![2u8; 200_000];
    let files: [(&str, &[u8]); 2] = [("one.bin", &one), ("two.bin", &two)];
    server.publish(&key, &manifest(1, "1.0", &files), &files);
    let two_place = format!("files/{}", Sha256::of(&two));
    server.cut(&two_place, 50_000);
    let error = format!("{:#}", update(&root).unwrap_err());
    assert!(error.contains("two.bin"), "{error}");
    assert!(!root.join("app").exists());
    // The finished file waits in staging/, the cut one doesn't.
    assert_eq!(
        contents(&root.join("staging")),
        expected(&[("one.bin", &one)])
    );

    server.put(&two_place, &two);
    server.files_asked();
    let Outcome::Updated { downloaded, .. } = update(&root).unwrap() else {
        panic!("not an update");
    };
    assert_eq!(downloaded, 1);
    assert_eq!(server.asked(&two_place), 1);
    assert_eq!(
        without_record(contents(&root.join("app"))),
        expected(&files)
    );
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_rollback_takes_the_version_before_from_the_disk() {
    let (server, key) = (Server::start(), key());
    let root = folder("rollback", &server, &key);
    server.publish(&key, &manifest(1, "1.0", &V1), &V1);
    update(&root).unwrap();
    server.publish(&key, &manifest(2, "2.0", &V2), &V2);
    update(&root).unwrap();
    server.files_asked();
    // The channel goes back to 1.0's files, with a newer sequence.
    server.publish(&key, &manifest(3, "1.0 again", &V1), &V1);
    let Outcome::Updated { downloaded, .. } = update(&root).unwrap() else {
        panic!("not an update");
    };
    assert_eq!(downloaded, 0);
    assert_eq!(server.files_asked(), 0);
    assert_eq!(without_record(contents(&root.join("app"))), expected(&V1));
    assert_eq!(
        without_record(contents(&root.join("previous"))),
        expected(&V2)
    );
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn refused_manifests_change_nothing() {
    let (server, key) = (Server::start(), key());
    let root = folder("refused", &server, &key);
    server.publish(&key, &manifest(5, "5.0", &V2), &V2);
    update(&root).unwrap();
    server.files_asked();
    let before = contents(&root.join("app"));

    let signed = m2_update::sign(&manifest(6, "6.0", &V1).to_json(), &key).unwrap();
    let mut tampered = signed.clone();
    let last = tampered.len() - 3;
    tampered[last] ^= 0x01;
    let other = SigningKey::from_seed(&[8; 32]).unwrap();
    let cases: [(Vec<u8>, &str); 5] = [
        (tampered, "signature"),
        (
            m2_update::sign(&manifest(6, "6.0", &V1).to_json(), &other).unwrap(),
            "signature",
        ),
        (
            m2_update::sign(&manifest(4, "4.0", &V1).to_json(), &key).unwrap(),
            "older",
        ),
        (
            m2_update::sign(
                &manifest_of("test", 6, "6.0", "2026-10-21T00:00:00Z", &V1).to_json(),
                &key,
            )
            .unwrap(),
            "expired",
        ),
        (
            m2_update::sign(
                &manifest_of("live", 6, "6.0", "2100-01-01T00:00:00Z", &V1).to_json(),
                &key,
            )
            .unwrap(),
            "channel",
        ),
    ];
    for (bytes, says) in cases {
        server.put("test/manifest.signed", &bytes);
        let error = update(&root).unwrap_err();
        assert!(error.downcast_ref::<UpdateError>().is_some(), "{error:#}");
        assert!(format!("{error:#}").contains(says), "{says}: {error:#}");
        assert_eq!(contents(&root.join("app")), before);
    }
    assert_eq!(server.files_asked(), 0);
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn files_other_than_the_manifest_says_are_refused() {
    let (server, key) = (Server::start(), key());
    let root = folder("other-bytes", &server, &key);
    let v1 = manifest(1, "1.0", &V1);
    server.publish(&key, &v1, &V1);
    let place = format!("files/{}", Sha256::of(b"game one"));
    for (served, says) in [
        (b"game uno".as_slice(), "SHA-256"),
        (b"game one, and more".as_slice(), "more than"),
        (b"game".as_slice(), "4 bytes"),
    ] {
        server.put(&place, served);
        let error = format!("{:#}", update(&root).unwrap_err());
        assert!(error.contains(says), "{says}: {error}");
        assert!(!root.join("app").exists());
        assert!(!root.join("staging").join("openm2.exe").exists());
        assert!(!root.join("staging").join("openm2.exe.part").exists());
    }
    server.put(&place, b"game one");
    assert!(matches!(update(&root).unwrap(), Outcome::Updated { .. }));
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_manifest_signed_again_replaces_only_the_record() {
    let (server, key) = (Server::start(), key());
    let root = folder("renewed", &server, &key);
    server.publish(&key, &manifest(1, "1.0", &V1), &V1);
    update(&root).unwrap();
    server.files_asked();
    server.publish(&key, &manifest(2, "1.0", &V1), &V1);
    assert!(matches!(update(&root).unwrap(), Outcome::Renewed(_)));
    assert_eq!(server.files_asked(), 0);
    assert!(!root.join("previous").exists());
    let install = open(&root).unwrap();
    assert_eq!(install.installed(&key.public_key()).unwrap().sequence(), 2);
    drop(install);
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn leftovers_in_staging_are_cleaned() {
    let (server, key) = (Server::start(), key());
    let root = folder("leftovers", &server, &key);
    let staging = root.join("staging");
    fs::create_dir_all(staging.join("packs").join("old")).unwrap();
    fs::write(staging.join("packs").join("old").join("x"), b"x").unwrap();
    fs::write(staging.join("packs").join("a.txt.part"), b"aa").unwrap();
    // A file where the new version has a folder.
    fs::write(staging.join("packs").join("b"), b"b").unwrap();
    // The right name with the wrong bytes.
    fs::write(staging.join("openm2.exe"), b"game won").unwrap();
    let files: [(&str, &[u8]); 2] = [("openm2.exe", b"game one"), ("packs/b/c.txt", b"ccc")];
    server.publish(&key, &manifest(1, "1.0", &files), &files);
    update(&root).unwrap();
    assert_eq!(
        without_record(contents(&root.join("app"))),
        expected(&files)
    );
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn an_interrupted_swap_is_put_back() {
    let (server, key) = (Server::start(), key());
    let root = folder("swap", &server, &key);
    server.publish(&key, &manifest(1, "1.0", &V1), &V1);
    update(&root).unwrap();
    // As if the launcher stopped between its two renames.
    fs::rename(root.join("app"), root.join("previous")).unwrap();
    let install = open(&root).unwrap();
    assert!(!install.previous().exists());
    assert_eq!(without_record(contents(&install.app())), expected(&V1));
    drop(install);
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn one_launcher_at_a_time() {
    let (server, key) = (Server::start(), key());
    let root = folder("lock", &server, &key);
    let first = open(&root).unwrap();
    // Refused at once, while the first holds it.
    let error = Install::open(&root).err().unwrap().to_string();
    assert!(error.contains("another launcher"), "{error}");
    drop(first);
    assert!(open(&root).is_ok());
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn the_launcher_replaces_itself() {
    let (server, key) = (Server::start(), key());
    let root = folder("itself", &server, &key);
    let running = root.join("launcher.bin");
    let new = root.join("new.bin");
    fs::write(&running, b"old launcher").unwrap();
    fs::write(&new, b"new launcher").unwrap();
    assert!(replace_launcher(&running, &new).unwrap());
    assert_eq!(fs::read(&running).unwrap(), b"new launcher");
    assert_eq!(
        fs::read(root.join("openm2-launcher.old")).unwrap(),
        b"old launcher"
    );
    assert!(!replace_launcher(&running, &new).unwrap());
    assert!(!replace_launcher(&running, &root.join("missing")).unwrap());
    // The next start deletes the old one.
    drop(open(&root).unwrap());
    assert!(!root.join("openm2-launcher.old").exists());
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn the_launcher_updates_and_says_why_when_it_cant() {
    let (server, key) = (Server::start(), key());
    let root = folder("binary", &server, &key);
    server.publish(&key, &manifest(1, "1.0", &V1), &V1);
    let launcher = || {
        let mut command = Command::new(env!("CARGO_BIN_EXE_openm2-launcher"));
        command.arg("--dir").arg(&root).arg("--update-only");
        command
    };
    let output = launcher().output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "{stdout}{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        stdout.contains("updated to 1.0: 3 files downloaded (22 bytes)"),
        "{stdout}"
    );
    // The channel's address shows without its path, which may be a
    // player's secret link, and so does launcher.log.
    assert!(
        stdout.contains("looking for updates at http://127.0.0.1:"),
        "{stdout}"
    );
    assert!(stdout.contains("/…/"), "{stdout}");
    let log = fs::read_to_string(root.join(m2_launcher::LAUNCHER_LOG)).unwrap();
    for text in [&stdout.to_string(), &log] {
        assert!(!text.contains("/channels"), "{text}");
    }
    assert_eq!(without_record(contents(&root.join("app"))), expected(&V1));

    let other = SigningKey::from_seed(&[8; 32]).unwrap();
    server.publish(&other, &manifest(2, "2.0", &V2), &V2);
    let output = launcher().output().unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "{stderr}");
    assert!(stderr.contains("signature doesn't match"), "{stderr}");
    assert!(!stderr.contains("/channels"), "{stderr}");

    // Without a game to start, playing says so.
    let output = Command::new(env!("CARGO_BIN_EXE_openm2-launcher"))
        .arg("--dir")
        .arg(&root)
        .arg("--previous")
        .arg("--no-window")
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("no version before the last update"),
        "{stderr}"
    );
    fs::remove_dir_all(&root).unwrap();
}

/// A version whose `openm2.toml` names `assets` starts the
/// game without a client folder or keys file; one that doesn't, with a
/// game that doesn't import, asks for them, and without a console says how
/// to give them. (The launcher stands in for such a game: it refuses
/// `assets current` as a command it doesn't know, as openm2 0.2.1
/// and earlier do. The import itself is tested with openm2: `m2-cli`'s
/// `tests/launcher.rs`.)
#[test]
fn a_version_with_its_own_assets_needs_no_client() {
    let (server, key) = (Server::start(), key());
    let root = folder("assets", &server, &key);
    let game = m2_launcher::game_name();
    let launcher = |extra: &[&str]| {
        let output = Command::new(env!("CARGO_BIN_EXE_openm2-launcher"))
            .arg("--dir")
            .arg(&root)
            .arg("--no-window")
            .args(extra)
            .stdin(std::process::Stdio::null())
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        (output.status.success(), format!("{stdout}{stderr}"))
    };
    // Not a program: starting it fails, which shows the launcher tried.
    let with_assets: [(&str, &[u8]); 3] = [
        (&game, b"not a program"),
        ("openm2.toml", b"format = 1\nassets = \"assets\"\n"),
        ("assets/assets.toml", b"format = 1\nname = \"sample\"\n"),
    ];
    server.publish(&key, &manifest(1, "1.0", &with_assets), &with_assets);
    let (updated, text) = launcher(&["--update-only"]);
    assert!(updated, "{text}");
    let (_, text) = launcher(&["--no-update"]);
    assert!(text.contains("starting"), "{text}");
    assert!(!text.contains("client folder"), "{text}");
    let (_, text) = launcher(&["--no-update", "--client", "c", "--keys", "k.toml"]);
    assert!(
        text.contains("--client, --keys and --import aren't used"),
        "{text}"
    );

    // A game that doesn't start can't say whether the client is imported.
    let broken: [(&str, &[u8]); 2] = [(&game, b"not a program"), ("openm2.toml", b"format = 1\n")];
    server.publish(&key, &manifest(2, "2.0", &broken), &broken);
    let (updated, text) = launcher(&["--update-only"]);
    assert!(updated, "{text}");
    let (played, text) = launcher(&["--no-update"]);
    assert!(!played);
    assert!(text.contains("starting"), "{text}");
    assert!(!root.join("imported").exists());

    // Last: Windows keeps a program's folder a moment after it ran, and
    // an update would have to wait for it.
    let old_game = fs::read(env!("CARGO_BIN_EXE_openm2-launcher")).unwrap();
    let without: [(&str, &[u8]); 2] = [(&game, &old_game), ("openm2.toml", b"format = 1\n")];
    server.publish(&key, &manifest(3, "3.0", &without), &without);
    let (updated, text) = launcher(&["--update-only"]);
    assert!(updated, "{text}");
    let (played, text) = launcher(&["--no-update"]);
    assert!(!played);
    assert!(
        text.contains("doesn't name your client folder and keys file"),
        "{text}"
    );
    assert!(!root.join("imported").exists());
    let _ = fs::remove_dir_all(&root);
}

/// More than `RANGE_GAP`, so the files on either side of it come apart.
fn large() -> Vec<u8> {
    (0..300_000u32).map(|i| (i % 251) as u8).collect()
}

#[test]
fn bundles_come_whole_then_by_range() {
    let (server, key) = (Server::start(), key());
    let root = folder("bundles", &server, &key);
    let large = large();
    let v1: [(&str, &[u8]); 6] = [
        ("openm2.exe", b"game one"),
        ("assets/a.toml", b"a = 1\n"),
        ("assets/b.png", &large),
        ("assets/c.toml", b"c = 1\n"),
        ("assets/copy.toml", b"a = 1\n"),
        ("assets/d.toml", b"d = 1\n"),
    ];
    let (manifest, objects) = bundled(1, "1.0", &v1);
    assert_eq!(manifest.bundles().len(), 1);
    server.publish_objects(&key, &manifest, &objects);
    // A first install: the whole bundle, and the file on its own.
    let Outcome::Updated {
        downloaded, bytes, ..
    } = update(&root).unwrap()
    else {
        panic!("not an update");
    };
    assert_eq!((downloaded, bytes), (5, 300_026));
    assert_eq!((server.files_asked(), server.sent()), (2, 300_026));
    assert_eq!(without_record(contents(&root.join("app"))), expected(&v1));

    // One file changes: only its bytes come, by range.
    let mut v2 = v1;
    v2[3] = ("assets/c.toml", b"c = 2\n");
    let (manifest, objects) = bundled(2, "2.0", &v2);
    server.publish_objects(&key, &manifest, &objects);
    let Outcome::Updated {
        downloaded, bytes, ..
    } = update(&root).unwrap()
    else {
        panic!("not an update");
    };
    assert_eq!((downloaded, bytes), (1, 6));
    assert_eq!((server.files_asked(), server.sent()), (1, 6));
    assert_eq!(without_record(contents(&root.join("app"))), expected(&v2));
    assert_eq!(
        without_record(contents(&root.join("previous"))),
        expected(&v1)
    );

    // The same files bundled another way: nothing to fetch.
    let unbundled = manifest_of("test", 3, "2.0", "2100-01-01T00:00:00Z", &v2);
    server.publish(&key, &unbundled, &v2);
    assert!(matches!(update(&root).unwrap(), Outcome::Renewed(_)));
    assert_eq!(server.files_asked(), 0);
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_server_that_ignores_ranges_is_refused() {
    let (server, key) = (Server::start(), key());
    let root = folder("no-ranges", &server, &key);
    let large = large();
    let v1: [(&str, &[u8]); 3] = [
        ("assets/a.toml", b"a = 1\n"),
        ("assets/b.png", &large),
        ("assets/c.toml", b"c = 1\n"),
    ];
    let (manifest, objects) = bundled(1, "1.0", &v1);
    server.publish_objects(&key, &manifest, &objects);
    update(&root).unwrap();
    // A first install asks for whole bundles, which such a server sends;
    // an update asks for a range.
    server.ignore_ranges();
    let mut v2 = v1;
    v2[2] = ("assets/c.toml", b"c = 2\n");
    let (manifest, objects) = bundled(2, "2.0", &v2);
    server.publish_objects(&key, &manifest, &objects);
    let error = format!("{:#}", update(&root).unwrap_err());
    assert!(error.contains("doesn't answer ranges"), "{error}");
    assert_eq!(without_record(contents(&root.join("app"))), expected(&v1));
    assert!(!root.join("staging").join("assets").join("c.toml").exists());
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_bundle_stopped_part_way_keeps_its_whole_files() {
    let (server, key) = (Server::start(), key());
    let root = folder("bundle-cut", &server, &key);
    let [one, two, three] = [1u8, 2, 3].map(|byte| vec![byte; 100_000]);
    let files: [(&str, &[u8]); 3] = [
        ("assets/1.bin", &one),
        ("assets/2.bin", &two),
        ("assets/3.bin", &three),
    ];
    let (manifest, objects) = bundled(1, "1.0", &files);
    server.publish_objects(&key, &manifest, &objects);
    let place = format!("files/{}", manifest.bundles()[0].sha256);
    server.cut(&place, 150_000);
    let error = format!("{:#}", update(&root).unwrap_err());
    assert!(error.contains("assets/2.bin"), "{error}");
    assert_eq!(
        contents(&root.join("staging")),
        expected(&[("assets/1.bin", &one)])
    );

    // The next start asks only for the rest of the bundle.
    server.put(&place, &objects[0]);
    server.files_asked();
    server.sent();
    let Outcome::Updated { downloaded, .. } = update(&root).unwrap() else {
        panic!("not an update");
    };
    assert_eq!(downloaded, 2);
    assert_eq!((server.files_asked(), server.sent()), (1, 200_000));
    assert_eq!(
        without_record(contents(&root.join("app"))),
        expected(&files)
    );
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_content_at_several_paths_is_downloaded_once() {
    let (server, key) = (Server::start(), key());
    let root = folder("duplicates", &server, &key);
    let files: [(&str, &[u8]); 3] = [
        ("a.txt", b"the same"),
        ("b/c.txt", b"the same"),
        ("d.txt", b"another"),
    ];
    server.publish(&key, &manifest(1, "1.0", &files), &files);
    let Outcome::Updated { downloaded, .. } = update(&root).unwrap() else {
        panic!("not an update");
    };
    assert_eq!(downloaded, 2);
    assert_eq!(server.files_asked(), 2);
    assert_eq!(
        without_record(contents(&root.join("app"))),
        expected(&files)
    );
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn several_requests_run_at_once() {
    let (server, key) = (Server::start(), key());
    let root = folder("at-once", &server, &key);
    let names: Vec<String> = (0..24).map(|i| format!("file{i:02}.txt")).collect();
    let files: Vec<(&str, &[u8])> = names
        .iter()
        .map(|name| (name.as_str(), name.as_bytes()))
        .collect();
    server.slow(Duration::from_millis(50));
    server.publish(&key, &manifest(1, "1.0", &files), &files);
    update(&root).unwrap();
    assert_eq!(server.files_asked(), 24);
    assert!(
        (2..=AT_ONCE).contains(&server.most()),
        "{} at once",
        server.most()
    );
    assert_eq!(
        without_record(contents(&root.join("app"))),
        expected(&files)
    );
    fs::remove_dir_all(&root).unwrap();
}

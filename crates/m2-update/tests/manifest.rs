//! Signed manifests (`docs/specs/update.md`): the bytes written, what is
//! refused, the checks against the installed version, the plan, bundles and
//! the requests that fetch them, and random input.

use m2_update::{
    BUNDLE_SIZE, Bundle, FileEntry, InstallPath, MAX_FILE_SIZE, MAX_MANIFEST, Manifest, Place,
    PublicKey, RANGE_GAP, Sha256, SigningKey, Source, Timestamp, UpdateError, Verdict, check,
    cut_bundles, open, place_bundled, plan, requests, sign,
};
use proptest::prelude::*;

/// RFC 8032's first test key.
const SEED: &str = "9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60";
const PUBLIC: &str = "ed25519:d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a";

fn key() -> SigningKey {
    let mut seed = [0u8; 32];
    for (i, byte) in seed.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&SEED[i * 2..i * 2 + 2], 16).unwrap();
    }
    SigningKey::from_seed(&seed).unwrap()
}

fn public() -> PublicKey {
    PUBLIC.parse().unwrap()
}

fn time(text: &str) -> Timestamp {
    text.parse().unwrap()
}

fn entry(path: &str, contents: &[u8]) -> FileEntry {
    FileEntry {
        path: InstallPath::new(path).unwrap(),
        size: contents.len() as u64,
        sha256: Sha256::of(contents),
        place: None,
    }
}

fn manifest(sequence: u64, files: Vec<FileEntry>) -> Manifest {
    Manifest::new(
        "test",
        sequence,
        "0.1.0",
        time("2027-01-20T00:00:00Z"),
        files,
        Vec::new(),
    )
    .unwrap()
}

fn two_files() -> Manifest {
    manifest(
        1_792_627_200,
        vec![entry("openm2.exe", b"abc"), entry("LICENSE-MIT", b"")],
    )
}

/// The signature line over `json`, signed with the test key whatever it says.
fn signed_anyway(json: &[u8]) -> Vec<u8> {
    let signature = key().sign(json).unwrap();
    let mut signed = b"openm2-manifest ed25519 ".to_vec();
    for byte in signature {
        signed.extend_from_slice(format!("{byte:02x}").as_bytes());
    }
    signed.push(b'\n');
    signed.extend_from_slice(json);
    signed
}

/// A manifest's JSON with `from` replaced by `to`, signed again.
fn edited(from: &str, to: &str) -> Vec<u8> {
    let json = String::from_utf8(two_files().to_json()).unwrap();
    assert!(json.contains(from), "{from}");
    signed_anyway(json.replacen(from, to, 1).as_bytes())
}

const JSON: &str = r#"{
  "format": 1,
  "channel": "test",
  "sequence": 1792627200,
  "version": "0.1.0",
  "expires": "2027-01-20T00:00:00Z",
  "files": [
    {
      "path": "LICENSE-MIT",
      "size": 0,
      "sha256": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    },
    {
      "path": "openm2.exe",
      "size": 3,
      "sha256": "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    }
  ]
}
"#;

/// The test key's signature over `JSON` (the same from RFC 8032's own
/// Python code, section 6).
const SIGNATURE: &str = concat!(
    "bcab1fdd58dbeac35ef9b3fdcce51686ae89290959ee5f2d12094719f2b5732b",
    "234c7b17ea76ccce43487b9f19786a4489d3cbfdb4b01c52b98e4b2e512d6d01",
);

#[test]
fn the_manifest_is_written_byte_for_byte() {
    let json = two_files().to_json();
    assert_eq!(String::from_utf8(json.clone()).unwrap(), JSON);
    assert_eq!(Manifest::from_json(JSON.as_bytes()).unwrap(), two_files());
    // Files are put in order; readers take any order.
    let reordered = JSON
        .replace("LICENSE-MIT", "zz")
        .replace("openm2.exe", "LICENSE-MIT");
    let read = Manifest::from_json(reordered.as_bytes()).unwrap();
    let paths: Vec<&str> = read.files().iter().map(|f| f.path.as_str()).collect();
    assert_eq!(paths, ["LICENSE-MIT", "zz"]);
}

#[test]
fn the_signed_file_is_written_byte_for_byte() {
    let signed = sign(JSON.as_bytes(), &key()).unwrap();
    // The signature line: its start, 128 hex digits, a line feed.
    assert_eq!(&signed[..24], b"openm2-manifest ed25519 ");
    assert_eq!(std::str::from_utf8(&signed[24..152]).unwrap(), SIGNATURE);
    assert_eq!(signed[152], b'\n');
    assert_eq!(&signed[153..], JSON.as_bytes());
    assert_eq!(signed.len(), 153 + JSON.len());
    assert_eq!(key().public_key().to_string(), PUBLIC);
    assert_eq!(open(&signed, &public()).unwrap(), two_files());
}

#[test]
fn changed_bytes_or_another_key_are_refused() {
    let signed = sign(JSON.as_bytes(), &key()).unwrap();
    // Every byte of the manifest is covered: one changed anywhere fails.
    for at in [153, 200, signed.len() - 2] {
        let mut tampered = signed.clone();
        tampered[at] ^= 0x01;
        assert!(
            matches!(open(&tampered, &public()), Err(UpdateError::BadSignature)),
            "byte {at}"
        );
    }
    // So does one digit of the signature.
    let mut tampered = signed.clone();
    tampered[30] = if tampered[30] == b'0' { b'1' } else { b'0' };
    assert!(matches!(
        open(&tampered, &public()),
        Err(UpdateError::BadSignature)
    ));
    // And a key other than the signer's.
    let other = SigningKey::from_seed(&[7; 32]).unwrap().public_key();
    assert!(matches!(
        open(&signed, &other),
        Err(UpdateError::BadSignature)
    ));
    // Anything after the manifest is part of it.
    let mut longer = signed.clone();
    longer.extend_from_slice(b" ");
    assert!(matches!(
        open(&longer, &public()),
        Err(UpdateError::BadSignature)
    ));
}

#[test]
fn a_bad_signature_line_is_refused() {
    let signed = sign(JSON.as_bytes(), &key()).unwrap();
    let upper = {
        let mut upper = signed.clone();
        upper[24..152].make_ascii_uppercase();
        upper
    };
    let no_line_feed = {
        let mut bytes = signed.clone();
        bytes[152] = b' ';
        bytes
    };
    for bytes in [
        Vec::new(),
        signed[..152].to_vec(),
        JSON.as_bytes().to_vec(),
        upper,
        no_line_feed,
        [b"openm2-manifest rsa     ".as_slice(), &signed[24..]].concat(),
        [b"\n".as_slice(), &signed].concat(),
    ] {
        assert!(
            matches!(open(&bytes, &public()), Err(UpdateError::SignatureLine)),
            "{:?}",
            String::from_utf8_lossy(&bytes[..bytes.len().min(40)])
        );
    }
}

#[test]
fn signed_manifests_that_break_a_rule_are_refused() {
    let cases: [(Vec<u8>, &str); 14] = [
        (edited("\"format\": 1", "\"format\": 3"), "format 3"),
        (
            edited("\"version\"", "\"extra\": 1,\n  \"version\""),
            "unknown field",
        ),
        (
            edited("\"channel\": \"test\"", "\"channel\": \"Test\""),
            "channel",
        ),
        (
            edited("\"sequence\": 1792627200", "\"sequence\": 0"),
            "sequence",
        ),
        (
            edited("\"version\": \"0.1.0\"", "\"version\": \"\""),
            "version",
        ),
        (edited("2027-01-20T00:00:00Z", "2027-01-20"), "YYYY"),
        (
            edited("\"path\": \"LICENSE-MIT\"", "\"path\": \"../LICENSE-MIT\""),
            "`.`",
        ),
        (
            edited("\"path\": \"LICENSE-MIT\"", "\"path\": \"OPENM2.EXE\""),
            "twice",
        ),
        (
            edited("\"path\": \"LICENSE-MIT\"", "\"path\": \"openm2.exe/x\""),
            "inside",
        ),
        (edited("\"size\": 3", "\"size\": 4294967297"), "more than"),
        (edited("\"size\": 3", "\"size\": -3"), "invalid value"),
        (edited("ba7816bf", "BA7816BF"), "SHA-256"),
        (edited("\"sha256\": \"e3b0", "\"sha256\": \"e3b"), "SHA-256"),
        (signed_anyway(b"{\"format\": 1}"), "missing field"),
    ];
    for (bytes, says) in cases {
        let error = open(&bytes, &public()).unwrap_err().to_string();
        assert!(error.contains(says), "{says}: {error}");
    }
    // `sign` refuses what `open` would.
    let bad = String::from_utf8(edited("\"format\": 1", "\"format\": 3")[153..].to_vec()).unwrap();
    assert!(matches!(
        sign(bad.as_bytes(), &key()),
        Err(UpdateError::Format(3))
    ));
}

#[test]
fn limits_are_kept() {
    let empty = Manifest::new(
        "test",
        1,
        "v",
        time("2027-01-01T00:00:00Z"),
        Vec::new(),
        Vec::new(),
    );
    assert!(matches!(empty, Err(UpdateError::FileCount { count: 0 })));
    let huge = |path: &str| FileEntry {
        path: InstallPath::new(path).unwrap(),
        size: MAX_FILE_SIZE,
        sha256: Sha256::of(path.as_bytes()),
        place: None,
    };
    let sixteen: Vec<FileEntry> = (0..16).map(|i| huge(&format!("f{i}"))).collect();
    assert!(
        Manifest::new(
            "test",
            1,
            "v",
            time("2027-01-01T00:00:00Z"),
            sixteen.clone(),
            Vec::new()
        )
        .is_ok()
    );
    let mut seventeen = sixteen;
    seventeen.push(entry("one-more", b"x"));
    assert!(matches!(
        Manifest::new(
            "test",
            1,
            "v",
            time("2027-01-01T00:00:00Z"),
            seventeen,
            Vec::new()
        ),
        Err(UpdateError::TotalSize)
    ));
    let mut too_large = JSON.as_bytes().to_vec();
    too_large.resize(MAX_MANIFEST + 1, b' ');
    assert!(matches!(
        Manifest::from_json(&too_large),
        Err(UpdateError::TooLarge { .. })
    ));
    assert!(matches!(
        open(&signed_anyway(&too_large), &public()),
        Err(UpdateError::TooLarge { .. })
    ));
}

#[test]
fn checks_against_the_channel_the_time_and_the_installed_version() {
    let now = time("2026-10-22T00:00:00Z");
    let v1 = manifest(10, vec![entry("openm2.exe", b"one")]);
    let v1_again = manifest(11, vec![entry("openm2.exe", b"one")]);
    let v2 = manifest(12, vec![entry("openm2.exe", b"two")]);
    assert_eq!(check(&v1, "test", now, None).unwrap(), Verdict::Update);
    assert_eq!(
        check(&v1, "test", now, Some(&v1)).unwrap(),
        Verdict::Current
    );
    assert_eq!(
        check(&v1_again, "test", now, Some(&v1)).unwrap(),
        Verdict::Renewed
    );
    assert_eq!(check(&v2, "test", now, Some(&v1)).unwrap(), Verdict::Update);
    assert!(matches!(
        check(&v1, "test", now, Some(&v2)),
        Err(UpdateError::Older {
            found: 10,
            installed: 12
        })
    ));
    assert!(matches!(
        check(&v2, "live", now, Some(&v1)),
        Err(UpdateError::OtherChannel { .. })
    ));
    let expiry = time("2027-01-20T00:00:00Z");
    assert!(check(&v2, "test", Timestamp::from_unix(expiry.unix() - 1), None).is_ok());
    let error = check(&v2, "test", expiry, None).unwrap_err().to_string();
    assert_eq!(error, "the manifest expired at 2027-01-20T00:00:00Z");
    // An installed version of another channel counts as nothing installed.
    let live = Manifest::new("live", 99, "9", expiry, vec![entry("a", b"a")], Vec::new()).unwrap();
    assert_eq!(
        check(&v1, "test", now, Some(&live)).unwrap(),
        Verdict::Update
    );
}

#[test]
fn the_plan_reuses_files_by_their_contents() {
    let previous = manifest(1, vec![entry("old.txt", b"old"), entry("same", b"same")]);
    let installed = manifest(
        2,
        vec![entry("same", b"same"), entry("moved/from.txt", b"moving")],
    );
    let new = manifest(
        3,
        vec![
            entry("same", b"same"),
            entry("to.txt", b"moving"),
            entry("back.txt", b"old"),
            entry("fresh", b"fresh"),
            // Same contents, another size: never mixed up.
            FileEntry {
                path: InstallPath::new("sized").unwrap(),
                size: 99,
                sha256: Sha256::of(b"same"),
                place: None,
            },
        ],
    );
    let steps = plan(&new, Some(&installed), Some(&previous));
    let sources: Vec<(&str, Source)> = steps
        .iter()
        .map(|step| (step.file.path.as_str(), step.source.clone()))
        .collect();
    let path = |p: &str| InstallPath::new(p).unwrap();
    assert_eq!(
        sources,
        [
            ("back.txt", Source::Previous(path("old.txt"))),
            ("fresh", Source::Download),
            ("same", Source::Installed(path("same"))),
            ("sized", Source::Download),
            ("to.txt", Source::Installed(path("moved/from.txt"))),
        ]
    );
    assert!(
        plan(&new, None, None)
            .iter()
            .all(|step| step.source == Source::Download)
    );
}

/// A file at `offset` of bundle 0.
fn bundled(path: &str, contents: &[u8], offset: u64) -> FileEntry {
    FileEntry {
        place: Some(Place { bundle: 0, offset }),
        ..entry(path, contents)
    }
}

/// Two files in a bundle, one of them at two paths, and a file on its own.
fn bundled_files() -> Manifest {
    Manifest::new(
        "test",
        1_792_627_200,
        "0.2.0",
        time("2027-01-20T00:00:00Z"),
        vec![
            bundled("b.txt", b"defg", 3),
            bundled("a.txt", b"abc", 0),
            bundled("copy.txt", b"abc", 0),
            entry("loose.bin", b"loose"),
        ],
        vec![Bundle {
            size: 7,
            sha256: Sha256::of(b"abcdefg"),
        }],
    )
    .unwrap()
}

const BUNDLED_JSON: &str = r#"{
  "format": 2,
  "channel": "test",
  "sequence": 1792627200,
  "version": "0.2.0",
  "expires": "2027-01-20T00:00:00Z",
  "bundles": [
    {
      "size": 7,
      "sha256": "7d1a54127b222502f5b79b5fb0803061152a44f92b37e23c6527baf665d4da9a"
    }
  ],
  "files": [
    {
      "path": "a.txt",
      "size": 3,
      "sha256": "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
      "bundle": 0,
      "offset": 0
    },
    {
      "path": "b.txt",
      "size": 4,
      "sha256": "4c8a43980498636e9c1d1595fa5d115af7937c2422dfe68a2520a52b7a5fb4de",
      "bundle": 0,
      "offset": 3
    },
    {
      "path": "copy.txt",
      "size": 3,
      "sha256": "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
      "bundle": 0,
      "offset": 0
    },
    {
      "path": "loose.bin",
      "size": 5,
      "sha256": "b5e0eee6e28efca6d6ad05d7b8a94631576037ec9e5ff6d305fe89faa0e1032e"
    }
  ]
}
"#;

#[test]
fn a_bundled_manifest_is_written_byte_for_byte() {
    let json = bundled_files().to_json();
    assert_eq!(String::from_utf8(json).unwrap(), BUNDLED_JSON);
    let read = Manifest::from_json(BUNDLED_JSON.as_bytes()).unwrap();
    assert_eq!(read, bundled_files());
    assert_eq!(
        read.file("b.txt").unwrap().place,
        Some(Place {
            bundle: 0,
            offset: 3
        })
    );
    assert_eq!(read.file("loose.bin").unwrap().place, None);
    let signed = sign(BUNDLED_JSON.as_bytes(), &key()).unwrap();
    assert_eq!(open(&signed, &public()).unwrap(), read);
    // The same files without bundles are the same files: a renewal.
    let unbundled = manifest(
        1_792_627_201,
        ["a.txt", "b.txt", "copy.txt", "loose.bin"]
            .map(|path| FileEntry {
                place: None,
                ..read.file(path).unwrap().clone()
            })
            .to_vec(),
    );
    assert!(unbundled.same_files(&read) && read.same_files(&unbundled));
    assert!(!unbundled.same_files(&two_files()));
    assert_eq!(
        check(
            &unbundled,
            "test",
            time("2026-10-22T00:00:00Z"),
            Some(&read)
        )
        .unwrap(),
        Verdict::Renewed
    );
}

#[test]
fn bundles_that_break_a_rule_are_refused() {
    let edited = |from: &str, to: &str| {
        assert!(BUNDLED_JSON.contains(from), "{from}");
        BUNDLED_JSON.replacen(from, to, 1).into_bytes()
    };
    let b_at = "\"bundle\": 0,\n      \"offset\": 3";
    let cases: [(Vec<u8>, &str); 12] = [
        (
            edited("\"format\": 2", "\"format\": 1"),
            "format 1 has no bundles",
        ),
        (
            JSON.replacen("\"format\": 1", "\"format\": 2", 1)
                .into_bytes(),
            "format 2 lists at least one",
        ),
        (
            JSON.replacen(
                "20015ad\"\n",
                "20015ad\",\n      \"bundle\": 0,\n      \"offset\": 0\n",
                1,
            )
            .into_bytes(),
            "format 1 has no bundles",
        ),
        (
            edited(b_at, "\"bundle\": 1,\n      \"offset\": 3"),
            "`b.txt` is in bundle 1, past",
        ),
        (
            edited(b_at, "\"bundle\": 0,\n      \"offset\": 4"),
            "`b.txt` runs past the end",
        ),
        (
            edited(b_at, "\"bundle\": 0,\n      \"offset\": 2"),
            "`b.txt` overlaps",
        ),
        (
            edited(b_at, "\"bundle\": 0,\n      \"offset\": 0"),
            "`b.txt` starts where another content",
        ),
        (
            edited(b_at, "\"bundle\": 0"),
            "`b.txt` names a bundle without",
        ),
        (
            edited("\"size\": 7", "\"size\": 8"),
            "bundle 0 has bytes no file covers, from 7",
        ),
        (edited("\"size\": 7", "\"size\": 0"), "bundle 0 is not 1 to"),
        (
            edited("\"sha256\": \"7d1a", "\"sha256\": \"7D1A"),
            "SHA-256",
        ),
        (
            edited("\"size\": 7,", "\"size\": 7,\n      \"extra\": 1,"),
            "unknown field",
        ),
    ];
    for (bytes, says) in cases {
        let error = Manifest::from_json(&bytes).unwrap_err().to_string();
        assert!(error.contains(says), "{says}: {error}");
        let error = open(&signed_anyway(&bytes), &public())
            .unwrap_err()
            .to_string();
        assert!(error.contains(says), "{says}: {error}");
    }
    let made = |files: Vec<FileEntry>, bundles: Vec<Bundle>| {
        Manifest::new("test", 1, "v", time("2027-01-01T00:00:00Z"), files, bundles)
            .unwrap_err()
            .to_string()
    };
    let abc = Bundle {
        size: 3,
        sha256: Sha256::of(b"abc"),
    };
    let unused = made(vec![bundled("a.txt", b"abc", 0)], vec![abc, abc]);
    assert!(unused.contains("no file is in bundle 1"), "{unused}");
    let empty = made(
        vec![bundled("a.txt", b"abc", 0), bundled("empty", b"", 3)],
        vec![abc],
    );
    assert!(empty.contains("`empty` is empty"), "{empty}");
    let after = made(vec![bundled("a.txt", b"abc", 1)], vec![abc]);
    assert!(after.contains("`a.txt` runs past"), "{after}");
    let gap = made(
        vec![bundled("a.txt", b"a", 1)],
        vec![Bundle {
            size: 2,
            sha256: Sha256::of(b"xa"),
        }],
    );
    assert!(gap.contains("no file covers, from 0"), "{gap}");
}

/// A file of `size` bytes, without its bytes: its SHA-256 is its path's.
fn sized(path: &str, size: u64) -> FileEntry {
    FileEntry {
        path: InstallPath::new(path).unwrap(),
        size,
        sha256: Sha256::of(path.as_bytes()),
        place: None,
    }
}

#[test]
fn bundles_are_cut_by_size_each_content_once() {
    let mib = 1 << 20;
    let mut files = vec![
        sized("a", 10 * mib),
        sized("b", 6 * mib),
        sized("c", 1),
        // Too large, and empty: on their own.
        sized("d", BUNDLE_SIZE),
        sized("e", 0),
        // a's content again.
        FileEntry {
            path: InstallPath::new("f").unwrap(),
            ..sized("a", 10 * mib)
        },
        sized("g", 5 * mib),
    ];
    let cut = cut_bundles(&files);
    assert_eq!(cut, [vec![0, 1], vec![2, 6]]);
    place_bundled(&mut files, &cut);
    let at = |bundle, offset| Some(Place { bundle, offset });
    let places: Vec<Option<Place>> = files.iter().map(|file| file.place).collect();
    assert_eq!(
        places,
        [
            at(0, 0),
            at(0, 10 * mib),
            at(1, 0),
            None,
            None,
            at(0, 0),
            at(1, 1)
        ]
    );
    let bundles = vec![
        Bundle {
            size: BUNDLE_SIZE,
            sha256: Sha256::of(b"0"),
        },
        Bundle {
            size: 5 * mib + 1,
            sha256: Sha256::of(b"1"),
        },
    ];
    assert!(Manifest::new("test", 1, "v", time("2027-01-01T00:00:00Z"), files, bundles).is_ok());
}

#[test]
fn requests_fetch_each_content_once_by_bundle() {
    let gap = RANGE_GAP;
    let at = |path: &str, size: u64, bundle: usize, offset: u64| FileEntry {
        place: Some(Place { bundle, offset }),
        ..sized(path, size)
    };
    let files = vec![
        // Bundle 0: a, a large b, then c and d; a's content again.
        at("0/a", 100, 0, 0),
        at("0/b", gap + 100, 0, 100),
        at("0/c", 10, 0, gap + 200),
        at("0/d", 10, 0, gap + 210),
        FileEntry {
            path: InstallPath::new("0/copy").unwrap(),
            ..at("0/a", 100, 0, 0)
        },
        // Bundle 1: e and f.
        at("1/e", 50, 1, 0),
        at("1/f", 10, 1, 50),
        // Bundle 2: g, h and i.
        at("2/g", 10, 2, 0),
        at("2/h", 1000, 2, 10),
        at("2/i", 10, 2, 1010),
        sized("loose", 5),
    ];
    let bundles: Vec<Bundle> = [gap + 220, 60, 1020]
        .iter()
        .map(|&size| Bundle {
            size,
            sha256: Sha256::of(&size.to_le_bytes()),
        })
        .collect();
    let manifest = Manifest::new(
        "test",
        1,
        "v",
        time("2027-01-01T00:00:00Z"),
        files,
        bundles.clone(),
    )
    .unwrap();
    let needed = [
        "0/a", "0/c", "0/copy", "0/d", "1/e", "1/f", "2/g", "2/i", "loose",
    ]
    .map(|path| manifest.file(path).unwrap());
    type Shown<'a> = (Sha256, Option<(u64, u64)>, u64, Vec<(u64, Vec<&'a str>)>);
    let shown: Vec<Shown> = requests(&manifest, &needed)
        .iter()
        .map(|request| {
            (
                request.object,
                request.range,
                request.size,
                request
                    .parts
                    .iter()
                    .map(|part| {
                        (
                            part.at,
                            part.files.iter().map(|file| file.path.as_str()).collect(),
                        )
                    })
                    .collect(),
            )
        })
        .collect();
    assert_eq!(
        shown,
        [
            // a and c are too far apart for one request.
            (
                bundles[0].sha256,
                Some((0, 100)),
                100,
                vec![(0, vec!["0/a", "0/copy"])]
            ),
            (
                bundles[0].sha256,
                Some((gap + 200, gap + 220)),
                20,
                vec![(0, vec!["0/c"]), (10, vec!["0/d"])]
            ),
            // All of bundle 1; and all of bundle 2, h's bytes along.
            (
                bundles[1].sha256,
                None,
                60,
                vec![(0, vec!["1/e"]), (50, vec!["1/f"])]
            ),
            (
                bundles[2].sha256,
                None,
                1020,
                vec![(0, vec!["2/g"]), (1010, vec!["2/i"])]
            ),
            (Sha256::of(b"loose"), None, 5, vec![(0, vec!["loose"])]),
        ]
    );
    assert!(requests(&manifest, &[]).is_empty());
}

/// Paths of a few short parts.
fn paths() -> impl Strategy<Value = Vec<String>> {
    prop::collection::btree_set("[a-z]{1,6}(/[a-z]([a-z0-9 ._-]{0,4}[a-z])?){0,2}", 1..12)
        .prop_map(|set| set.into_iter().collect())
}

proptest! {
    #[test]
    fn random_bytes_never_panic(bytes in prop::collection::vec(any::<u8>(), 0..400)) {
        let _ = open(&bytes, &public());
        let _ = Manifest::from_json(&bytes);
    }

    #[test]
    fn damaged_signed_files_never_panic(
        flips in prop::collection::vec((0usize..2000, any::<u8>()), 1..6),
    ) {
        let mut signed = sign(JSON.as_bytes(), &key()).unwrap();
        for (at, byte) in flips {
            let at = at % signed.len();
            signed[at] = byte;
        }
        let _ = open(&signed, &public());
        let json = signed[153..].to_vec();
        let _ = Manifest::from_json(&json);
    }

    #[test]
    fn random_manifests_read_back(
        paths in paths(),
        sequence in 1u64..=u64::MAX,
        days in 0u64..100_000,
    ) {
        // Device names (`con`, `aux`, …) are left out.
        let files: Vec<FileEntry> = paths
            .iter()
            .filter(|p| InstallPath::new(p).is_ok())
            .map(|p| entry(p, p.as_bytes()))
            .collect();
        // Paths where one is the folder of another are refused, the rest kept.
        let Ok(manifest) = Manifest::new(
            "test",
            sequence,
            "1.0",
            Timestamp::from_unix(0).plus_days(days),
            files,
            Vec::new(),
        ) else {
            return Ok(());
        };
        let signed = sign(&manifest.to_json(), &key()).unwrap();
        prop_assert_eq!(open(&signed, &public()).unwrap(), manifest);
    }
    #[test]
    fn bundled_manifests_read_back_and_their_requests_bring_their_files(
        // Few different bytes, so that contents repeat.
        contents in prop::collection::vec(prop::collection::vec(0u8..3, 0..6), 1..16),
        wanted in prop::collection::vec(any::<bool>(), 16),
    ) {
        let mut files: Vec<FileEntry> = contents
            .iter()
            .enumerate()
            .map(|(i, bytes)| entry(&format!("f{i:02}"), bytes))
            .collect();
        let cut = cut_bundles(&files);
        place_bundled(&mut files, &cut);
        let blobs: Vec<Vec<u8>> = cut
            .iter()
            .map(|indices| indices.iter().flat_map(|&i| contents[i].clone()).collect())
            .collect();
        let bundles = blobs
            .iter()
            .map(|blob| Bundle { size: blob.len() as u64, sha256: Sha256::of(blob) })
            .collect();
        let manifest =
            Manifest::new("test", 1, "1.0", time("2027-01-20T00:00:00Z"), files, bundles).unwrap();
        let signed = sign(&manifest.to_json(), &key()).unwrap();
        prop_assert_eq!(&open(&signed, &public()).unwrap(), &manifest);
        // The objects a channel serves: the bundles, and every content.
        let mut objects: std::collections::BTreeMap<Sha256, Vec<u8>> =
            contents.iter().map(|bytes| (Sha256::of(bytes), bytes.clone())).collect();
        objects.extend(blobs.iter().map(|blob| (Sha256::of(blob), blob.clone())));
        let needed: Vec<&FileEntry> = manifest
            .files()
            .iter()
            .zip(&wanted)
            .filter(|(_, wanted)| **wanted)
            .map(|(file, _)| file)
            .collect();
        let mut brought = Vec::new();
        let mut parts = 0;
        for request in requests(&manifest, &needed) {
            let object = &objects[&request.object];
            let whole = (0, object.len() as u64);
            prop_assert!(request.range != Some(whole));
            let (start, end) = request.range.unwrap_or(whole);
            prop_assert_eq!(end - start, request.size);
            let body = &object[start as usize..end as usize];
            for part in &request.parts {
                parts += 1;
                let at = part.at as usize;
                let bytes = &body[at..at + part.files[0].size as usize];
                for file in &part.files {
                    prop_assert_eq!(
                        (file.size, file.sha256),
                        (bytes.len() as u64, Sha256::of(bytes))
                    );
                    brought.push(file.path.as_str());
                }
            }
        }
        let mut asked: Vec<&str> = needed.iter().map(|file| file.path.as_str()).collect();
        asked.sort_unstable();
        brought.sort_unstable();
        prop_assert_eq!(brought, asked);
        let contents: std::collections::BTreeSet<_> =
            needed.iter().map(|file| (file.size, file.sha256)).collect();
        prop_assert_eq!(parts, contents.len());
    }
}

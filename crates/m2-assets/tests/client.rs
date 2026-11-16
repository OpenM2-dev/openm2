//! Archives, the VFS and extraction on synthetic clients in temp folders.

mod support;

use m2_assets::{Archive, AssetError, Source, Vfs, extract, load_keys};
use m2_formats::{ArchiveWriter, Entry, EntryKind, Keys, RawStr};
use support::{TempDir, test_keys, write_archive};

#[test]
fn archive_reads_every_supported_type() {
    let tmp = TempDir::new("archive");
    let index = write_archive(
        tmp.path(),
        "root",
        &[
            ("d:/ymir work/a.txt", b"stored", EntryKind::Stored),
            ("locale/xx/b.txt", b"compressed", EntryKind::Lzo),
            (
                "locale/xx/c.txt",
                b"compressed and encrypted",
                EntryKind::LzoXtea,
            ),
        ],
        true,
    );
    let archive = Archive::open(&index, &test_keys()).unwrap();
    assert_eq!(archive.entries().len(), 3);
    assert_eq!(
        archive.read_name(b"D:\\Ymir Work\\A.TXT").unwrap(),
        b"stored"
    );
    assert_eq!(
        archive.read_name(b"locale/xx/b.txt").unwrap(),
        b"compressed"
    );
    assert_eq!(
        archive.read_name(b"locale/xx/c.txt").unwrap(),
        b"compressed and encrypted"
    );
    assert!(matches!(
        archive.read_name(b"missing"),
        Err(AssetError::NotFound(_))
    ));
}

#[test]
fn missing_keys_are_named() {
    let tmp = TempDir::new("keys");
    let index = write_archive(tmp.path(), "root", &[("a", b"x", EntryKind::LzoXtea)], true);
    let error = Archive::open(&index, &Keys::none()).err().unwrap();
    assert!(
        error.to_string().contains("add the `eterpack.index` key"),
        "{error}"
    );

    let mut only_index = Keys::none();
    only_index.eterpack_index = test_keys().eterpack_index;
    let archive = Archive::open(&index, &only_index).unwrap();
    let error = archive.read_name(b"a").unwrap_err();
    assert!(
        error.to_string().contains("add the `eterpack.entries` key"),
        "{error}"
    );
}

#[test]
fn entries_past_the_end_of_the_data_file_are_refused() {
    let tmp = TempDir::new("bounds");
    let index = write_archive(
        tmp.path(),
        "root",
        &[("a", b"0123456789", EntryKind::Stored)],
        false,
    );
    std::fs::write(tmp.path().join("root.epk"), b"01234").unwrap();
    let archive = Archive::open(&index, &Keys::none()).unwrap();
    let error = archive.read_name(b"a").unwrap_err();
    assert!(
        matches!(
            error,
            AssetError::OutOfBounds {
                end: 10,
                len: 5,
                ..
            }
        ),
        "{error}"
    );
}

#[test]
fn a_missing_data_file_is_reported() {
    let tmp = TempDir::new("no-epk");
    let index = write_archive(tmp.path(), "root", &[("a", b"x", EntryKind::Stored)], false);
    std::fs::remove_file(tmp.path().join("root.epk")).unwrap();
    let error = Archive::open(&index, &Keys::none()).err().unwrap();
    assert!(error.to_string().contains("root.epk"), "{error}");
}

#[test]
fn vfs_mounts_a_client_and_loose_files_win() {
    let tmp = TempDir::new("vfs");
    let pack = tmp.path().join("pack");
    write_archive(
        &pack,
        "b_patch",
        &[
            ("shared.txt", b"from b", EntryKind::Lzo),
            ("only_b.txt", b"b", EntryKind::Stored),
        ],
        true,
    );
    write_archive(
        &pack,
        "a_root",
        &[
            ("shared.txt", b"from a", EntryKind::LzoXtea),
            ("d:/ymir work/x.txt", b"x", EntryKind::Stored),
        ],
        true,
    );
    let mut vfs = Vfs::open_client(tmp.path(), &test_keys()).unwrap();
    // Sorted by file name: a_root first, so its copy wins.
    assert_eq!(vfs.archives().len(), 2);
    assert_eq!(vfs.read(b"shared.txt").unwrap(), b"from a");
    assert_eq!(vfs.read(b"only_b.txt").unwrap(), b"b");
    assert_eq!(vfs.shadowed().len(), 1);
    assert_eq!(vfs.shadowed()[0].name, RawStr::from("shared.txt"));
    assert!(vfs.shadowed()[0].ignored.ends_with("b_patch.eix"));
    let names: Vec<&[u8]> = vfs.files().map(|(name, _, _)| name).collect();
    assert_eq!(
        names,
        [&b"d:/ymir work/x.txt"[..], b"only_b.txt", b"shared.txt"]
    );

    // A loose folder overrides the archives, with the drive prefix dropped.
    let loose = tmp.path().join("loose");
    std::fs::create_dir_all(loose.join("ymir work")).unwrap();
    std::fs::write(loose.join("ymir work").join("x.txt"), b"loose x").unwrap();
    vfs.add_loose_dir(&loose);
    assert_eq!(vfs.read(b"D:\\Ymir Work\\X.txt").unwrap(), b"loose x");
    assert!(matches!(
        vfs.source(b"shared.txt"),
        Some(Source::Archive(..))
    ));
    assert!(matches!(vfs.read(b"nothing"), Err(AssetError::NotFound(_))));
}

#[test]
fn loose_lookups_never_leave_their_folder() {
    let tmp = TempDir::new("escape");
    let loose = tmp.path().join("loose");
    std::fs::create_dir_all(&loose).unwrap();
    std::fs::write(tmp.path().join("secret.txt"), b"outside").unwrap();
    let mut vfs = Vfs::new();
    vfs.add_loose_dir(&loose);
    for path in ["../secret.txt", "..\\secret.txt", "loose/../../secret.txt"] {
        assert!(
            matches!(vfs.read(path.as_bytes()), Err(AssetError::NotFound(_))),
            "{path}"
        );
    }
}

#[test]
fn extract_writes_files_inside_the_folder_only() {
    let tmp = TempDir::new("extract");
    let keys = test_keys();
    let mut writer = ArchiveWriter::new();
    writer
        .add(b"d:/ymir work/pc/a.txt", b"a", EntryKind::Lzo, &keys)
        .unwrap();
    writer
        .add(b"locale/xx/b.txt", b"bb", EntryKind::LzoXtea, &keys)
        .unwrap();
    // Hostile names straight into the index, bypassing the writer.
    for name in [
        "../escaped.txt",
        "c:/../../escaped2.txt",
        "/abs.txt",
        "con.txt",
    ] {
        writer.add_raw(
            Entry {
                id: 9,
                name: RawStr::from(name),
                name_crc: 0,
                sizes: [4, 4],
                data_crc: 0,
                offset: 0,
                kind: EntryKind::Stored,
            },
            b"evil",
        );
    }
    let (eix, epk) = writer.finish(None).unwrap();
    std::fs::write(tmp.path().join("t.eix"), eix).unwrap();
    std::fs::write(tmp.path().join("t.epk"), epk).unwrap();
    let archive = Archive::open(&tmp.path().join("t.eix"), &keys).unwrap();

    let out = tmp.path().join("out");
    let report = extract(&archive, &out, |_| true).unwrap();
    assert_eq!(report.written, 2);
    assert_eq!(report.bytes, 3);
    assert_eq!(report.skipped.len(), 4, "{:?}", report.skipped);
    assert_eq!(std::fs::read(out.join("ymir work/pc/a.txt")).unwrap(), b"a");
    assert_eq!(std::fs::read(out.join("locale/xx/b.txt")).unwrap(), b"bb");
    assert!(!tmp.path().join("escaped.txt").exists());
    assert!(!tmp.path().join("escaped2.txt").exists());

    // Running it again would overwrite: refused.
    let error = extract(&archive, &out, |_| true).unwrap_err();
    assert!(matches!(error, AssetError::Io { .. }), "{error}");
}

#[test]
fn extract_does_not_write_through_a_file_where_a_folder_should_be() {
    let tmp = TempDir::new("extract-file");
    let index = write_archive(
        tmp.path(),
        "t",
        &[("a/b.txt", b"x", EntryKind::Stored)],
        false,
    );
    let archive = Archive::open(&index, &Keys::none()).unwrap();
    let out = tmp.path().join("out");
    std::fs::create_dir_all(&out).unwrap();
    std::fs::write(out.join("a"), b"in the way").unwrap();
    let error = extract(&archive, &out, |_| true).unwrap_err();
    assert!(matches!(error, AssetError::UnsafePath { .. }), "{error}");
}

#[cfg(unix)]
#[test]
fn extract_does_not_follow_links() {
    let tmp = TempDir::new("extract-link");
    let index = write_archive(
        tmp.path(),
        "t",
        &[("a/b.txt", b"x", EntryKind::Stored)],
        false,
    );
    let archive = Archive::open(&index, &Keys::none()).unwrap();
    let out = tmp.path().join("out");
    let elsewhere = tmp.path().join("elsewhere");
    std::fs::create_dir_all(&out).unwrap();
    std::fs::create_dir_all(&elsewhere).unwrap();
    std::os::unix::fs::symlink(&elsewhere, out.join("a")).unwrap();
    let error = extract(&archive, &out, |_| true).unwrap_err();
    assert!(matches!(error, AssetError::UnsafePath { .. }), "{error}");
    assert!(!elsewhere.join("b.txt").exists());
}

#[test]
fn keys_file_round_trip() {
    let tmp = TempDir::new("keysfile");
    let path = tmp.path().join("keys.toml");
    std::fs::write(
        &path,
        "[eterpack]\nindex = [0x01234567, 0x89ABCDEF, 0xFEDCBA98, 0x76543210]\n\
         entries = [0x11111111, 0x22222222, 0x33333333, 0x44444444]\n\
         [proto]\nitem = [0xA5A5A5A5, 0x5A5A5A5A, 0x0F0F0F0F, 0xF0F0F0F0]\nmob = [1, 2, 3, 4]\n",
    )
    .unwrap();
    let keys = load_keys(&path).unwrap();
    assert_eq!(keys, test_keys());
    assert_eq!(
        format!("{keys:?}"),
        r#"{"eterpack.index", "eterpack.entries", "proto.item", "proto.mob"}"#
    );

    std::fs::write(&path, "[eterpack]\nindex = 5\n").unwrap();
    let error = load_keys(&path).unwrap_err();
    assert!(error.to_string().contains("line 2"), "{error}");
}

// Rules from a real TMP4 client: size fields in either order, container
// entries listed too short, an archive list (`pack/Index`) that orders them.

/// An index entry with the given sizes, for `ArchiveWriter::add_raw`.
fn raw(name: &str, sizes: [u32; 2], kind: EntryKind) -> Entry {
    Entry {
        id: 0,
        name: RawStr::from(name),
        name_crc: 0,
        sizes,
        data_crc: 0,
        offset: 0,
        kind,
    }
}

#[test]
fn odd_size_fields_read_like_the_client() {
    let tmp = TempDir::new("sizes");
    let keys = test_keys();
    let short = m2_formats::container::encode(b"listed too short", None);
    let encrypted =
        m2_formats::container::encode(b"encrypted, at the end", keys.eterpack_entries.as_ref());
    let mut writer = ArchiveWriter::new();
    // Stored: data length first, reserved space second, padding in between.
    writer.add_raw(raw("a.txt", [5, 8], EntryKind::Stored), b"aaaaa...");
    // Stored: the other order.
    writer.add_raw(raw("b.txt", [8, 5], EntryKind::Stored), b"bbbbb...");
    // Container listed 16 bytes short (header left out).
    let n = short.len() as u32 - 16;
    writer.add_raw(raw("c.txt", [n, n + 7], EntryKind::Lzo), &short);
    // Last entry: one size field runs past the end of the .epk.
    let n = encrypted.len() as u32;
    writer.add_raw(
        raw("d.txt", [n + 4, n + 512], EntryKind::LzoXtea),
        &encrypted,
    );
    let (eix, epk) = writer.finish(None).unwrap();
    std::fs::write(tmp.path().join("t.eix"), eix).unwrap();
    std::fs::write(tmp.path().join("t.epk"), epk).unwrap();

    let archive = Archive::open(&tmp.path().join("t.eix"), &keys).unwrap();
    assert_eq!(archive.read_name(b"a.txt").unwrap(), b"aaaaa");
    assert_eq!(archive.read_name(b"b.txt").unwrap(), b"bbbbb");
    assert_eq!(archive.read_name(b"c.txt").unwrap(), b"listed too short");
    assert_eq!(
        archive.read_name(b"d.txt").unwrap(),
        b"encrypted, at the end"
    );
}

#[test]
fn a_container_running_past_the_data_file_is_refused() {
    let tmp = TempDir::new("container-bounds");
    let container = m2_formats::container::encode(b"cut off", None);
    let mut writer = ArchiveWriter::new();
    writer.add_raw(
        raw("a", [3, 3], EntryKind::Lzo),
        &container[..container.len() - 3],
    );
    let (eix, epk) = writer.finish(None).unwrap();
    std::fs::write(tmp.path().join("t.eix"), eix).unwrap();
    std::fs::write(tmp.path().join("t.epk"), epk).unwrap();
    let archive = Archive::open(&tmp.path().join("t.eix"), &Keys::none()).unwrap();
    let error = archive.read_name(b"a").unwrap_err();
    assert!(matches!(error, AssetError::OutOfBounds { .. }), "{error}");
}

#[test]
fn the_archive_list_decides_which_archive_wins() {
    let tmp = TempDir::new("pack-list");
    let pack = tmp.path().join("pack");
    write_archive(
        &pack,
        "base",
        &[("shared.txt", b"from base", EntryKind::Lzo)],
        true,
    );
    write_archive(
        &pack,
        "patch",
        &[("shared.txt", b"from patch", EntryKind::Lzo)],
        true,
    );
    write_archive(
        &pack,
        "extra",
        &[("extra.txt", b"x", EntryKind::Stored)],
        true,
    );
    // Without a list: file name order, so `base` wins.
    let vfs = Vfs::open_client(tmp.path(), &test_keys()).unwrap();
    assert_eq!(vfs.read(b"shared.txt").unwrap(), b"from base");

    // The list names the patch first (case differs from the file name);
    // `extra` is not listed and comes after the listed ones.
    let list = m2_formats::pack_list::write(&[
        m2_formats::pack_list::PackListEntry {
            prefix: RawStr::from("*"),
            name: RawStr::from("PATCH"),
        },
        m2_formats::pack_list::PackListEntry {
            prefix: RawStr::from("d:/ymir work/"),
            name: RawStr::from("base"),
        },
    ]);
    std::fs::write(pack.join("Index"), list).unwrap();
    let vfs = Vfs::open_client(tmp.path(), &test_keys()).unwrap();
    assert_eq!(vfs.read(b"shared.txt").unwrap(), b"from patch");
    let order: Vec<String> = vfs
        .archives()
        .iter()
        .map(|a| a.path().file_stem().unwrap().to_string_lossy().into_owned())
        .collect();
    assert_eq!(order, ["patch", "base", "extra"]);
    assert!(vfs.shadowed()[0].ignored.ends_with("base.eix"));
}

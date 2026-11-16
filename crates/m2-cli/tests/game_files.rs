//! `openm2 pack`, `vfs` and `proto` on a synthetic client built with our own
//! writers and test keys, in a temp folder.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

use m2_formats::proto::TypeValue;
use m2_formats::{
    ArchiveWriter, Entry, EntryKind, ItemProto, ItemRecord, Keys, MobProto, MobRecord, RawStr,
    XteaKey,
};

const INDEX_KEY: XteaKey = XteaKey([0x0123_4567, 0x89AB_CDEF, 0xFEDC_BA98, 0x7654_3210]);
const ENTRIES_KEY: XteaKey = XteaKey([0x1111_1111, 0x2222_2222, 0x3333_3333, 0x4444_4444]);
const ITEM_KEY: XteaKey = XteaKey([0xA5A5_A5A5, 0x5A5A_5A5A, 0x0F0F_0F0F, 0xF0F0_F0F0]);
const MOB_KEY: XteaKey = XteaKey([1, 2, 3, 4]);

const KEYS_TOML: &str = "\
[eterpack]
index = [0x01234567, 0x89ABCDEF, 0xFEDCBA98, 0x76543210]
entries = [0x11111111, 0x22222222, 0x33333333, 0x44444444]
[proto]
item = [0xA5A5A5A5, 0x5A5A5A5A, 0x0F0F0F0F, 0xF0F0F0F0]
mob = [1, 2, 3, 4]
";

fn keys() -> Keys {
    Keys {
        eterpack_index: Some(INDEX_KEY),
        eterpack_entries: Some(ENTRIES_KEY),
        item_proto: Some(ITEM_KEY),
        mob_proto: Some(MOB_KEY),
    }
}

/// A client folder with two archives, and a keys.toml beside it.
struct Client {
    root: PathBuf,
}

impl Client {
    fn new(name: &str) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!(
            "openm2-files-{name}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("client/pack")).unwrap();
        std::fs::write(root.join("keys.toml"), KEYS_TOML).unwrap();

        let items = ItemProto {
            version: 1,
            records: vec![ItemRecord {
                vnum: 10,
                name: RawStr::from_field(&[0xB0, 0xCB]),
                locale_name: RawStr::from("Sword+0"),
                limits: [TypeValue { kind: 1, value: 25 }, TypeValue::default()],
                ..ItemRecord::default()
            }],
        };
        let mobs = MobProto {
            records: vec![
                MobRecord {
                    vnum: 101,
                    locale_name: RawStr::from("Wild Dog"),
                    damage_multiplier: 1.5,
                    ..MobRecord::default()
                },
                MobRecord::default(),
            ],
        };
        let keys = keys();
        let mut root_pack = ArchiveWriter::new();
        root_pack
            .add(
                b"d:/ymir work/pc/a.txt",
                b"model list",
                EntryKind::Lzo,
                &keys,
            )
            .unwrap();
        root_pack
            .add(
                b"locale/xx/item_proto",
                &items.to_bytes(Some(&ITEM_KEY)).unwrap(),
                EntryKind::LzoXtea,
                &keys,
            )
            .unwrap();
        root_pack
            .add(
                b"locale/xx/mob_proto",
                &mobs.to_bytes(Some(&MOB_KEY)).unwrap(),
                EntryKind::Stored,
                &keys,
            )
            .unwrap();
        let client = Self { root };
        client.write_archive("root", root_pack, true);

        let mut patch = ArchiveWriter::new();
        patch
            .add(b"locale/xx/readme.txt", b"patched", EntryKind::Lzo, &keys)
            .unwrap();
        client.write_archive("patch", patch, false);
        client
    }

    fn write_archive(&self, stem: &str, writer: ArchiveWriter, encrypt_index: bool) {
        let (eix, epk) = writer.finish(encrypt_index.then_some(&INDEX_KEY)).unwrap();
        std::fs::write(self.pack(stem), eix).unwrap();
        std::fs::write(self.pack(stem).with_extension("epk"), epk).unwrap();
    }

    fn dir(&self) -> PathBuf {
        self.root.join("client")
    }

    fn pack(&self, stem: &str) -> PathBuf {
        self.root.join("client/pack").join(format!("{stem}.eix"))
    }

    fn keys_file(&self) -> PathBuf {
        self.root.join("keys.toml")
    }

    /// Runs openm2 with OPENM2_KEYS pointing at the test keys.
    fn openm2(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_openm2"))
            .args(args)
            .env("OPENM2_KEYS", self.keys_file())
            .env_remove("RUST_LOG")
            .output()
            .unwrap()
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn path(p: &Path) -> &str {
    p.to_str().unwrap()
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n")
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).replace("\r\n", "\n")
}

#[test]
fn pack_ls_lists_every_entry() {
    let client = Client::new("ls");
    let output = client.openm2(&["pack", "ls", path(&client.pack("root"))]);
    assert!(output.status.success(), "{}", stderr(&output));
    let text = stdout(&output);
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 4, "{text}");
    assert!(lines[0].starts_with("lzo "), "{text}");
    assert!(lines[0].ends_with("  d:/ymir work/pc/a.txt"), "{text}");
    assert!(lines[1].starts_with("lzo+xtea "), "{text}");
    assert!(lines[2].starts_with("stored "), "{text}");
    assert_eq!(lines[3], "3 entries (stored 1, lzo 1, lzo+xtea 1)");
}

#[test]
fn pack_cat_writes_the_decoded_entry() {
    let client = Client::new("cat");
    let index = client.pack("root");
    let output = client.openm2(&["pack", "cat", path(&index), "D:\\Ymir Work\\PC\\a.txt"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(output.stdout, b"model list");

    let out = client.root.join("item_proto");
    let output = client.openm2(&[
        "pack",
        "cat",
        path(&index),
        "locale/xx/item_proto",
        "--out",
        path(&out),
    ]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(&std::fs::read(&out).unwrap()[..4], b"MIPX");
}

#[test]
fn missing_keys_say_which_key_to_add() {
    let client = Client::new("nokeys");
    let output = Command::new(env!("CARGO_BIN_EXE_openm2"))
        .args(["pack", "ls", path(&client.pack("root"))])
        .env_remove("OPENM2_KEYS")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).contains("add the `eterpack.index` key to keys.toml"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn pack_extract_skips_unsafe_names_and_stays_inside() {
    let client = Client::new("extract");
    let mut writer = ArchiveWriter::new();
    writer
        .add(b"d:/ymir work/pc/a.txt", b"a", EntryKind::Lzo, &keys())
        .unwrap();
    writer.add_raw(
        Entry {
            id: 1,
            name: RawStr::from("../evil.txt"),
            name_crc: 0,
            sizes: [4, 4],
            data_crc: 0,
            offset: 0,
            kind: EntryKind::Stored,
        },
        b"evil",
    );
    client.write_archive("hostile", writer, false);
    let out = client.root.join("out");
    let output = client.openm2(&[
        "pack",
        "extract",
        path(&client.pack("hostile")),
        "--out",
        path(&out),
    ]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(
        stdout(&output).starts_with("wrote 1 files (1 bytes) to "),
        "{}",
        stdout(&output)
    );
    assert!(
        stdout(&output).ends_with("; skipped 1\n"),
        "{}",
        stdout(&output)
    );
    assert!(
        stderr(&output).contains("skipped ../evil.txt: not a safe path"),
        "{}",
        stderr(&output)
    );
    assert_eq!(std::fs::read(out.join("ymir work/pc/a.txt")).unwrap(), b"a");
    assert!(!client.root.join("evil.txt").exists());
}

#[test]
fn vfs_ls_and_cat_by_game_path() {
    let client = Client::new("vfs");
    let dir = client.dir();
    let output = client.openm2(&["vfs", "ls", "--client", path(&dir), "locale/"]);
    assert!(output.status.success(), "{}", stderr(&output));
    let text = stdout(&output);
    // Columns: type, real size, name, archive.
    let rows: Vec<Vec<&str>> = text
        .lines()
        .map(|line| line.split_whitespace().collect())
        .collect();
    assert_eq!(
        rows.iter().map(|row| (row[2], row[3])).collect::<Vec<_>>(),
        [
            ("locale/xx/item_proto", "root.eix"),
            ("locale/xx/mob_proto", "root.eix"),
            ("locale/xx/readme.txt", "patch.eix")
        ],
        "{text}"
    );

    let output = client.openm2(&["vfs", "cat", "--client", path(&dir), "locale/xx/readme.txt"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(output.stdout, b"patched");
}

#[test]
fn vfs_check_decodes_everything() {
    let client = Client::new("check");
    let output = client.openm2(&["vfs", "check", "--client", path(&client.dir())]);
    assert!(
        output.status.success(),
        "{}{}",
        stdout(&output),
        stderr(&output)
    );
    assert_eq!(
        stdout(&output),
        "\
patch.eix: 1 entries (lzo 1)
  decoded 1/1; name CRC matches 1/1; data CRC matches 1/1
root.eix: 3 entries (stored 1, lzo 1, lzo+xtea 1)
  decoded 3/3; name CRC matches 3/3; data CRC matches 3/3
locale/xx/item_proto: 1 items
locale/xx/mob_proto: 2 monsters
ok
"
    );
}

#[test]
fn vfs_check_reports_damage() {
    let client = Client::new("damage");
    // Break the LZO stream of patch.epk's only entry, after the container
    // header and inner magic.
    let data = client.pack("patch").with_extension("epk");
    let mut bytes = std::fs::read(&data).unwrap();
    bytes[20] = 0x7F;
    std::fs::write(&data, bytes).unwrap();
    let output = client.openm2(&["vfs", "check", "--client", path(&client.dir())]);
    assert_eq!(output.status.code(), Some(1));
    let text = stdout(&output);
    assert!(
        text.contains("  FAILED `locale/xx/readme.txt`: entry: LZO1X:"),
        "{text}"
    );
    assert!(text.ends_with("1 problems\n"), "{text}");
}

#[test]
fn proto_dump_prints_json_lines() {
    let client = Client::new("proto");
    let output = client.openm2(&[
        "proto",
        "dump",
        "--client",
        path(&client.dir()),
        "locale/xx/item_proto",
    ]);
    assert!(output.status.success(), "{}", stderr(&output));
    let text = stdout(&output);
    assert_eq!(text.lines().count(), 1);
    assert!(
        text.starts_with(
            r#"{"vnum":10,"unknown_4":0,"name":"\\xb0\\xcb","locale_name":"Sword+0","kind":0,"#
        ),
        "{text}"
    );
    assert!(
        text.contains(r#""limits":[{"kind":1,"value":25},{"kind":0,"value":0}]"#),
        "{text}"
    );
    assert_eq!(stderr(&output), "1 items\n");

    // From a file on disk.
    let file = client.root.join("mob_proto");
    let output = client.openm2(&[
        "vfs",
        "cat",
        "--client",
        path(&client.dir()),
        "locale/xx/mob_proto",
        "--out",
        path(&file),
    ]);
    assert!(output.status.success(), "{}", stderr(&output));
    let output = client.openm2(&["proto", "dump", path(&file)]);
    assert!(output.status.success(), "{}", stderr(&output));
    let text = stdout(&output);
    assert_eq!(text.lines().count(), 2);
    assert!(text.contains(r#""locale_name":"Wild Dog""#), "{text}");
    assert!(text.contains(r#""damage_multiplier":1.5"#), "{text}");
}

#[test]
fn a_wrong_data_crc_is_counted_not_failed() {
    // Real clients have whole archives whose data CRCs match nothing.
    let client = Client::new("crc");
    let mut writer = ArchiveWriter::new();
    writer.add_raw(
        Entry {
            id: 0,
            name: RawStr::from("odd.txt"),
            name_crc: m2_formats::crc32::crc32(b"odd.txt"),
            sizes: [3, 3],
            data_crc: 0xDEAD_BEEF,
            offset: 0,
            kind: EntryKind::Stored,
        },
        b"odd",
    );
    client.write_archive("odd", writer, false);
    let output = client.openm2(&["vfs", "check", "--client", path(&client.dir())]);
    assert!(output.status.success(), "{}", stdout(&output));
    let text = stdout(&output);
    assert!(
        text.contains("odd.eix: 1 entries (stored 1)\n  decoded 1/1; name CRC matches 1/1; data CRC matches 0/1\n"),
        "{text}"
    );
    assert!(text.ends_with("ok\n"), "{text}");
}

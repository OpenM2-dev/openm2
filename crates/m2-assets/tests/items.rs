//! The items of a locale, in a synthetic client: names and sizes from the
//! item table, icons and models from the item list; its monsters' names; and
//! the locale setting.

mod support;

use m2_assets::Vfs;
use m2_assets::items::{LocaleSetting, read_items, read_locale, read_monsters};
use m2_formats::script::item_list::{self, ItemLine};
use m2_formats::{EntryKind, ItemProto, ItemRecord, MobProto, MobRecord, RawStr};
use support::{TempDir, test_keys, write_archive};

fn record(vnum: u32, name: &str, size: u8) -> ItemRecord {
    ItemRecord {
        vnum,
        locale_name: RawStr::from(name),
        size,
        ..ItemRecord::default()
    }
}

fn listed(vnum: u32, icon: &str) -> ItemLine {
    ItemLine {
        vnum,
        kind: RawStr::from("ETC"),
        icon: RawStr::from(icon),
        model: None,
    }
}

#[test]
fn items_have_their_names_sizes_and_icons() {
    let tmp = TempDir::new("items");
    let keys = test_keys();
    let proto = ItemProto {
        version: 1,
        records: vec![
            record(10, "Sword+0", 2),
            record(27001, "Red Potion (S)", 1),
            record(10, "Second record", 3),
        ],
    }
    .to_bytes(keys.item_proto.as_ref())
    .unwrap();
    let list = item_list::write(&[
        ItemLine {
            kind: RawStr::from("WEAPON"),
            model: Some(RawStr::from("D:/Ymir Work/item/weapon/00010.gr2")),
            ..listed(10, "icon/item/00010.tga")
        },
        ItemLine {
            model: Some(RawStr::from("d:/ymir work/item/weapon/other.gr2")),
            ..listed(10, "icon/item/other.tga")
        },
        listed(27001, "Icon/Item/27001.TGA"),
        listed(50, "icon/item/00050.tga"),
    ])
    .unwrap();
    let files: Vec<(&str, &[u8], EntryKind)> = vec![
        ("locale/en/item_proto", &proto, EntryKind::Lzo),
        ("locale/en/item_list.txt", &list, EntryKind::Lzo),
    ];
    write_archive(&tmp.path().join("pack"), "locale_en", &files, true);
    std::fs::write(tmp.path().join("locale.cfg"), "10002 1252 en\r\n").unwrap();

    let locale = read_locale(tmp.path()).unwrap();
    assert_eq!(locale.name, "en");
    assert_eq!(locale.codepage, 1252);
    let vfs = Vfs::open_client(tmp.path(), &keys).unwrap();
    let items = read_items(&vfs, &keys, &locale.name).unwrap();
    assert_eq!(items.len(), 2);
    // The first record and the first line of a number win; icon and model
    // paths are normalised like every game path.
    let sword = items.get(10).unwrap();
    assert_eq!(sword.record.locale_name, RawStr::from("Sword+0"));
    assert_eq!(sword.record.size, 2);
    assert_eq!(
        sword.icon.as_deref(),
        Some(b"icon/item/00010.tga".as_slice())
    );
    assert_eq!(
        sword.model.as_deref(),
        Some(b"d:/ymir work/item/weapon/00010.gr2".as_slice())
    );
    let potion = items.get(27001).unwrap();
    assert_eq!(
        potion.icon.as_deref(),
        Some(b"icon/item/27001.tga".as_slice())
    );
    assert_eq!(potion.model, None);
    // An icon for an item the table does not have is counted.
    assert_eq!(items.unlisted_icons, 1);
    assert!(items.get(50).is_none());
    // Another locale is not there.
    assert!(read_items(&vfs, &keys, "de").is_err());
}

#[test]
fn monsters_have_their_names_and_levels() {
    let tmp = TempDir::new("monsters");
    let keys = test_keys();
    let monster = |vnum: u32, name: &str, level: u8| MobRecord {
        vnum,
        name: RawStr::from("korean"),
        locale_name: RawStr::from(name),
        level,
        ..MobRecord::default()
    };
    let proto = MobProto {
        records: vec![
            monster(101, "Wild Dog", 1),
            monster(601, "Orc", 23),
            monster(101, "Second record", 2),
        ],
    }
    .to_bytes(keys.mob_proto.as_ref())
    .unwrap();
    let files: Vec<(&str, &[u8], EntryKind)> =
        vec![("locale/en/mob_proto", &proto, EntryKind::Lzo)];
    write_archive(&tmp.path().join("pack"), "locale_en", &files, true);
    let vfs = Vfs::open_client(tmp.path(), &keys).unwrap();
    let monsters = read_monsters(&vfs, &keys, "en").unwrap();
    assert_eq!(monsters.len(), 2);
    let dog = monsters.get(101).unwrap();
    assert_eq!(
        (dog.name.as_bytes(), dog.level),
        (b"Wild Dog".as_slice(), 1)
    );
    assert_eq!(monsters.get(601).unwrap().name, RawStr::from("Orc"));
    assert!(monsters.get(102).is_none());
    assert!(read_monsters(&vfs, &keys, "de").is_err());
}

#[test]
fn the_locale_setting_names_a_plain_folder() {
    let parse = |text: &str| LocaleSetting::parse(text.as_bytes());
    assert_eq!(
        parse("10002 1252 en").unwrap(),
        LocaleSetting {
            code: 10002,
            codepage: 1252,
            name: "en".to_owned()
        }
    );
    assert!(parse("10002 1252").is_err());
    assert!(parse("10002 x en").is_err());
    assert!(parse("10002 1252 ../en").is_err());
    assert!(parse("10002 1252 en extra").is_err());
    let tmp = TempDir::new("no-locale");
    assert!(read_locale(tmp.path()).is_err());
}

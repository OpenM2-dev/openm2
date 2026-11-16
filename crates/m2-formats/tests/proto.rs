//! item_proto and mob_proto: one record written out field by field, round
//! trips, layouts we do not know, damaged input.

mod common;

use common::{ITEM_KEY, MOB_KEY, test_keys};
use m2_formats::proto::{ITEM_RECORD_LEN, MOB_RECORD_LEN, MobSkill, TypeValue};
use m2_formats::{
    FormatError, ItemProto, ItemRecord, Keys, MobProto, MobRecord, Proto, RawStr, container,
};
use proptest::prelude::*;

fn name_field(name: &str, len: usize) -> Vec<u8> {
    let mut field = vec![0u8; len];
    field[..name.len()].copy_from_slice(name.as_bytes());
    field
}

fn item() -> ItemRecord {
    ItemRecord {
        vnum: 10,
        unknown_4: 0x0102_0304,
        name: RawStr::from_field(&[0xB0, 0xCB, b'+', b'0']),
        locale_name: RawStr::from("Sword+0"),
        kind: 1,
        subtype: 2,
        unknown_60: 3,
        size: 2,
        anti_flags: 0x10,
        flags: 0x20,
        wear_flags: 0x30,
        immune_flags: 0x40,
        buy_price: 1000,
        sell_price: 500,
        limits: [
            TypeValue { kind: 1, value: 25 },
            TypeValue { kind: 0, value: 0 },
        ],
        applies: [
            TypeValue { kind: 7, value: 22 },
            TypeValue { kind: 8, value: -5 },
            TypeValue { kind: 0, value: 0 },
        ],
        values: [0, 15, 19, 13, 15, -1],
        sockets: [1, 2, 3],
        upgrade_vnum: 11,
        upgrade_set: 0x0203,
        magic_item_percent: 15,
        specular: 4,
        socket_percent: 1,
    }
}

/// [`item`] as bytes, field by field, with the offsets from the spec.
fn item_bytes() -> Vec<u8> {
    let mut b = Vec::new();
    b.extend_from_slice(&10u32.to_le_bytes()); // 0 vnum
    b.extend_from_slice(&0x0102_0304u32.to_le_bytes()); // 4 unknown
    b.extend_from_slice(&[0xB0, 0xCB, b'+', b'0']); // 8 name (25)
    b.extend_from_slice(&[0; 21]);
    b.extend_from_slice(&name_field("Sword+0", 25)); // 33 locale name (25)
    assert_eq!(b.len(), 58);
    b.extend_from_slice(&[1, 2, 3, 2]); // 58 type, subtype, unknown, size
    for flags in [0x10u32, 0x20, 0x30, 0x40] {
        b.extend_from_slice(&flags.to_le_bytes()); // 62 anti, flags, wear, immune
    }
    b.extend_from_slice(&1000u32.to_le_bytes()); // 78 buy price
    b.extend_from_slice(&500u32.to_le_bytes()); // 82 sell price
    assert_eq!(b.len(), 86);
    for (kind, value) in [(1u8, 25i32), (0, 0), (7, 22), (8, -5), (0, 0)] {
        b.push(kind); // 86 limits (2 x 5), 96 applies (3 x 5)
        b.extend_from_slice(&value.to_le_bytes());
    }
    assert_eq!(b.len(), 111);
    for value in [0i32, 15, 19, 13, 15, -1, 1, 2, 3] {
        b.extend_from_slice(&value.to_le_bytes()); // 111 values (6), 135 sockets (3)
    }
    assert_eq!(b.len(), 147);
    b.extend_from_slice(&11u32.to_le_bytes()); // 147 upgrade vnum
    b.extend_from_slice(&0x0203u16.to_le_bytes()); // 151 upgrade set
    b.extend_from_slice(&[15, 4, 1]); // 153 magic %, specular, socket %
    assert_eq!(b.len(), ITEM_RECORD_LEN);
    b
}

fn mob() -> MobRecord {
    MobRecord {
        vnum: 101,
        name: RawStr::from_field(&[0xB5, 0xE9]),
        locale_name: RawStr::from("Wild Dog"),
        kind: 0,
        rank: 1,
        battle_type: 2,
        level: 1,
        size: 3,
        min_gold: 5,
        max_gold: 10,
        experience: 20,
        hp: 100,
        regen_delay: 6,
        regen_percent: 7,
        defence: 0x0809,
        ai_flags: 0x0A,
        race_flags: 0x0B,
        immune_flags: 0x0C,
        st: 13,
        dx: 14,
        ht: 15,
        iq: 16,
        damage: [17, 18],
        attack_speed: 100,
        move_speed: -1,
        aggressive_percent: 19,
        aggressive_sight: 2000,
        attack_range: 150,
        enchantments: [1, 2, 3, 4, 5, 6],
        resists: [7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17],
        resurrection_vnum: 102,
        drop_item_vnum: 103,
        mount_capacity: 2,
        on_click: 1,
        empire: 3,
        folder: RawStr::from("dog"),
        damage_multiplier: 1.5,
        summon_vnum: 104,
        drain_sp: 105,
        color: 0x00FF_00FF,
        polymorph_item_vnum: 106,
        skills: [
            MobSkill { vnum: 1, level: 2 },
            MobSkill { vnum: 3, level: 4 },
            MobSkill::default(),
            MobSkill::default(),
            MobSkill { vnum: 5, level: 6 },
        ],
        berserk_point: 20,
        stone_skin_point: 21,
        god_speed_point: 22,
        death_blow_point: 23,
        revive_point: 24,
    }
}

/// [`mob`] as bytes, field by field, with the offsets from the spec.
fn mob_bytes() -> Vec<u8> {
    let mut b = Vec::new();
    b.extend_from_slice(&101u32.to_le_bytes()); // 0 vnum
    b.extend_from_slice(&[0xB5, 0xE9]); // 4 name (25)
    b.extend_from_slice(&[0; 23]);
    b.extend_from_slice(&name_field("Wild Dog", 25)); // 29 locale name (25)
    b.extend_from_slice(&[0, 1, 2, 1, 3]); // 54 type, rank, battle type, level, size
    for value in [5u32, 10, 20, 100] {
        b.extend_from_slice(&value.to_le_bytes()); // 59 min/max gold, exp, hp
    }
    b.extend_from_slice(&[6, 7]); // 75 regen delay, regen %
    b.extend_from_slice(&0x0809u16.to_le_bytes()); // 77 defence
    for flags in [0x0Au32, 0x0B, 0x0C] {
        b.extend_from_slice(&flags.to_le_bytes()); // 79 ai, race, immune flags
    }
    assert_eq!(b.len(), 91);
    b.extend_from_slice(&[13, 14, 15, 16]); // 91 st, dx, ht, iq
    b.extend_from_slice(&17u32.to_le_bytes()); // 95 damage min
    b.extend_from_slice(&18u32.to_le_bytes()); // 99 damage max
    b.extend_from_slice(&100i16.to_le_bytes()); // 103 attack speed
    b.extend_from_slice(&(-1i16).to_le_bytes()); // 105 move speed
    b.push(19); // 107 aggressive %
    b.extend_from_slice(&2000u16.to_le_bytes()); // 108 aggressive sight
    b.extend_from_slice(&150u16.to_le_bytes()); // 110 attack range
    b.extend_from_slice(&[1, 2, 3, 4, 5, 6]); // 112 enchantments
    b.extend_from_slice(&[7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17]); // 118 resists
    assert_eq!(b.len(), 129);
    b.extend_from_slice(&102u32.to_le_bytes()); // 129 resurrection vnum
    b.extend_from_slice(&103u32.to_le_bytes()); // 133 drop item vnum
    b.extend_from_slice(&[2, 1, 3]); // 137 mount capacity, on click, empire
    b.extend_from_slice(&name_field("dog", 65)); // 140 folder (65)
    assert_eq!(b.len(), 205);
    b.extend_from_slice(&1.5f32.to_le_bytes()); // 205 damage multiplier
    for value in [104u32, 105, 0x00FF_00FF, 106] {
        b.extend_from_slice(&value.to_le_bytes()); // 209 summon, drain sp, color, polymorph
    }
    assert_eq!(b.len(), 225);
    for (vnum, level) in [(1u32, 2u8), (3, 4), (0, 0), (0, 0), (5, 6)] {
        b.extend_from_slice(&vnum.to_le_bytes()); // 225 skills (5 x 5)
        b.push(level);
    }
    b.extend_from_slice(&[20, 21, 22, 23, 24]); // 250 berserk, stone skin, god speed, death blow, revive
    assert_eq!(b.len(), MOB_RECORD_LEN);
    b
}

#[test]
fn item_record_bytes() {
    let proto = ItemProto {
        version: 1,
        records: vec![item()],
    };
    let file = proto.to_bytes(None).unwrap();
    // Header: magic, version, record size, count, container size.
    assert_eq!(&file[..4], b"MIPX");
    assert_eq!(&file[4..16], [1, 0, 0, 0, 156, 0, 0, 0, 1, 0, 0, 0]);
    assert_eq!(
        u32::from_le_bytes(file[16..20].try_into().unwrap()) as usize,
        file.len() - 20
    );
    // The container holds exactly the record bytes.
    let records = container::decode(
        "test",
        &file[20..],
        &Keys::none(),
        m2_formats::KeyName::ItemProto,
    )
    .unwrap();
    assert_eq!(records, item_bytes());
    assert_eq!(ItemProto::parse(&file, &Keys::none()).unwrap(), proto);
}

#[test]
fn mob_record_bytes() {
    let proto = MobProto {
        records: vec![mob()],
    };
    let file = proto.to_bytes(None).unwrap();
    // Header: magic, count, container size.
    assert_eq!(&file[..8], b"MMPT\x01\0\0\0");
    assert_eq!(
        u32::from_le_bytes(file[8..12].try_into().unwrap()) as usize,
        file.len() - 12
    );
    let records = container::decode(
        "test",
        &file[12..],
        &Keys::none(),
        m2_formats::KeyName::MobProto,
    )
    .unwrap();
    assert_eq!(records, mob_bytes());
    assert_eq!(MobProto::parse(&file, &Keys::none()).unwrap(), proto);
}

#[test]
fn encrypted_tables_round_trip() {
    let keys = test_keys();
    let items = ItemProto {
        version: 1,
        records: vec![item(), ItemRecord::default()],
    };
    let mobs = MobProto {
        records: vec![mob(), MobRecord::default(), mob()],
    };
    let item_file = items.to_bytes(Some(&ITEM_KEY)).unwrap();
    let mob_file = mobs.to_bytes(Some(&MOB_KEY)).unwrap();
    assert_eq!(Proto::parse(&item_file, &keys).unwrap(), Proto::Item(items));
    assert_eq!(Proto::parse(&mob_file, &keys).unwrap(), Proto::Mob(mobs));
    // The item key does not open the mob table.
    let mut swapped = keys.clone();
    swapped.mob_proto = Some(ITEM_KEY);
    assert!(matches!(
        MobProto::parse(&mob_file, &swapped),
        Err(FormatError::WrongKey { .. })
    ));
}

#[test]
fn unknown_item_layout_is_named() {
    let mut file = ItemProto {
        version: 1,
        records: vec![item()],
    }
    .to_bytes(None)
    .unwrap();
    file[8] = 160;
    let error = ItemProto::parse(&file, &Keys::none()).unwrap_err();
    assert_eq!(
        error.to_string(),
        "item_proto: version 1, 160-byte records is not a known layout (known: version 1, 156-byte records)"
    );
}

#[test]
fn unknown_mob_layout_is_named() {
    // Two records of 260 bytes.
    let container = container::encode(&[0u8; 520], None);
    let mut file = b"MMPT".to_vec();
    file.extend_from_slice(&2u32.to_le_bytes());
    file.extend_from_slice(&(container.len() as u32).to_le_bytes());
    file.extend_from_slice(&container);
    let error = MobProto::parse(&file, &Keys::none()).unwrap_err();
    assert!(error.to_string().contains("260-byte records"), "{error}");
}

#[test]
fn record_count_must_match_the_data() {
    let mut file = ItemProto {
        version: 1,
        records: vec![item()],
    }
    .to_bytes(None)
    .unwrap();
    file[12] = 2;
    assert!(matches!(
        ItemProto::parse(&file, &Keys::none()),
        Err(FormatError::WrongSize {
            expected: 312,
            actual: 156,
            ..
        })
    ));
}

#[test]
fn names_too_long_for_their_field_are_refused() {
    let mut record = item();
    record.locale_name = RawStr::from("a name that is far too long");
    let error = ItemProto {
        version: 1,
        records: vec![record],
    }
    .to_bytes(None)
    .unwrap_err();
    assert!(
        matches!(error, FormatError::StringTooLong { max: 24, .. }),
        "{error}"
    );
}

proptest! {
    #[test]
    fn garbage_never_panics(bytes in prop::collection::vec(any::<u8>(), 0..400)) {
        let _ = Proto::parse(&bytes, &test_keys());
    }

    #[test]
    fn damaged_tables_never_panic(
        flips in prop::collection::vec((any::<prop::sample::Index>(), any::<u8>()), 1..6),
        mob_table: bool,
    ) {
        let keys = test_keys();
        let mut file = if mob_table {
            MobProto { records: vec![mob()] }.to_bytes(Some(&MOB_KEY)).unwrap()
        } else {
            ItemProto { version: 1, records: vec![item()] }.to_bytes(Some(&ITEM_KEY)).unwrap()
        };
        for (at, byte) in flips {
            let i = at.index(file.len());
            file[i] ^= byte;
        }
        let _ = Proto::parse(&file, &keys);
    }
}

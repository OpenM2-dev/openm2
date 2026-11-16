//! Random and damaged folders never panic; random valid content reads back
//! the same after writing.

use m2_content::{
    Books, Content, Drop, Formula, GoldDrop, Good, Group, Header, Item, ItemBonus, Level,
    MAX_PRICE, Map, Monster, MonsterKind, MountCost, MountFeed, MountGrade, MountLevel, MountParts,
    MountRules, Npc, OnFail, Pack, PackMember, Parts, Race, Regen, Skill, SkillJob, SkillKind,
    Sources, SpawnFile, Start, StartItem, Stats, Teacher, UpgradeCost, UpgradeParts, UpgradeScroll,
    UpgradeSet, parse,
};
use proptest::prelude::*;
use proptest::strategy::ValueTree as _;

const FILES: [&str; 16] = [
    "content.toml",
    "maps.toml",
    "monsters.toml",
    "items.toml",
    "races.toml",
    "levels.toml",
    "drops.toml",
    "start.toml",
    "shops.toml",
    "teachers.toml",
    "training.toml",
    "mounts.toml",
    "upgrades.toml",
    "skills.toml",
    "packs.toml",
    "spawns/a.toml",
];

/// Random mounts: how they tire, each grade's `fights` and `skills`, each
/// level's speed, stamina, rider level, gold, skill points and whether it
/// costs an item, and one food's stamina.
type RandomMounts = (
    (u16, u16),
    Vec<(bool, bool)>,
    Vec<(u8, u16, u8, u32, u8, bool)>,
    u16,
);

fn mounts() -> impl Strategy<Value = Option<RandomMounts>> {
    proptest::option::of((
        (any::<u16>(), any::<u16>()),
        proptest::collection::vec((any::<bool>(), any::<bool>()), 1..4),
        proptest::collection::vec(
            (
                1u8..=255,
                1u16..=u16::MAX,
                any::<u8>(),
                0u32..=MAX_PRICE,
                0u8..=50,
                any::<bool>(),
            ),
            1..6,
        ),
        1u16..=u16::MAX,
    ))
}

/// Formulas the reader takes, some with every function.
const FORMULAS: [&str; 6] = [
    "1",
    "10+20*k",
    "-(2*atk+(0.5*atk+3*str)*k)",
    "number(1, 3)*lv + floor(3+k*5)",
    "min(maxhp, maxsp) - max(odef, chain) / ar",
    "(200+dex*0.2+con*0.5+iq-mwep)*k",
];

fn formula() -> impl Strategy<Value = Option<Formula>> {
    proptest::option::of((0..FORMULAS.len()).prop_map(|at| Formula::parse(FORMULAS[at]).unwrap()))
}

fn word() -> impl Strategy<Value = String> {
    "[a-z_][a-z0-9_]{0,31}"
}

fn skill() -> impl Strategy<Value = Skill> {
    (
        name(),
        (0..SkillJob::ALL.len(), any::<u8>(), 0..SkillKind::ALL.len()),
        proptest::collection::vec(word(), 0..4),
        (proptest::option::of(word()), formula(), formula()),
        (proptest::option::of(word()), formula(), formula()),
        (formula(), formula(), formula()),
        (any::<u8>(), metres(0, 100_000), metres(0, 100_000)),
    )
        .prop_map(
            |(
                name,
                (job, level, kind),
                flags,
                (point, amount, duration),
                (point2, amount2, duration2),
                (sp_cost, duration_sp_cost, cooldown),
                (max_hit, target_range, splash_range),
            )| Skill {
                vnum: 0,
                name,
                job: SkillJob::ALL[job].0,
                level,
                kind: SkillKind::ALL[kind].0,
                flags,
                point,
                amount,
                duration,
                point2,
                amount2,
                duration2,
                sp_cost,
                duration_sp_cost,
                cooldown,
                max_hit,
                target_range,
                splash_range,
            },
        )
}

/// Metres in hundredths, so every value is valid and exact to write.
fn metres(low: u32, high: u32) -> impl Strategy<Value = f64> {
    (low..=high).prop_map(|cm| f64::from(cm) / 100.0)
}

fn name() -> impl Strategy<Value = String> {
    "[A-Za-z][A-Za-z0-9 'é\"\\\\]{0,20}"
}

fn monster(vnum: u32, kind: MonsterKind) -> impl Strategy<Value = Monster> {
    (
        name(),
        (any::<u8>(), any::<u8>(), any::<u8>(), any::<u32>()),
        proptest::option::of((metres(1, 360_000), 0u8..=100)),
        (any::<u32>(), any::<u32>(), 0u32..1000, 0u32..1000),
        (metres(0, 100_000), any::<[u8; 4]>()),
        (any::<u8>(), any::<u8>(), metres(0, 100_000), any::<bool>()),
        (metres(0, 100_000), any::<bool>()),
        proptest::option::of(metres(1, 10_000)),
    )
        .prop_map(
            move |(
                name,
                (rank, battle, level, hp),
                regen,
                (defence, experience, low, extra),
                (damage_multiplier, [st, dx, ht, iq]),
                (attack_speed, move_speed, attack_range, aggressive),
                (aggressive_sight, moves),
                run_speed,
            )| Monster {
                vnum,
                name,
                kind,
                to: None,
                rank,
                battle,
                level,
                hp,
                hp_regen: regen.map(|(every, percent)| Regen { every, percent }),
                defence,
                experience,
                gold: [low, low + extra],
                damage: [extra, extra + low],
                damage_multiplier,
                stats: Stats { st, dx, ht, iq },
                attack_speed,
                move_speed,
                attack_range,
                aggressive,
                aggressive_sight,
                moves,
                run_speed,
                calls: Vec::new(),
            },
        )
}

fn item(vnum: u32) -> impl Strategy<Value = Item> {
    (
        name(),
        (any::<u8>(), any::<u8>(), 1u8..=9),
        any::<[u32; 6]>(),
        proptest::collection::vec((any::<u8>(), any::<i32>()), 0..=3),
        any::<[i32; 6]>(),
        any::<[i32; 3]>(),
        (any::<u16>(), any::<[u8; 3]>()),
    )
        .prop_map(
            move |(
                name,
                (kind, subtype, size),
                [
                    anti_flags,
                    flags,
                    wear_flags,
                    immune_flags,
                    buy_price,
                    sell_price,
                ],
                bonuses,
                values,
                sockets,
                (upgrade_set, [magic_percent, socket_percent, specular]),
            )| {
                let bonuses: Vec<ItemBonus> = bonuses
                    .into_iter()
                    .map(|(kind, value)| ItemBonus { kind, value })
                    .collect();
                Item {
                    vnum,
                    name,
                    kind,
                    subtype,
                    size,
                    anti_flags,
                    flags,
                    wear_flags,
                    immune_flags,
                    buy_price,
                    sell_price,
                    limits: bonuses.clone(),
                    applies: bonuses.into_iter().rev().collect(),
                    values,
                    sockets,
                    upgrade: buy_price,
                    upgrade_set,
                    magic_percent,
                    socket_percent,
                    specular,
                }
            },
        )
}

/// Valid content: one map, a monster, an NPC, an item, races, levels,
/// drops, starting items, a shop, skills, a pack and a spawn file with
/// groups (of the monster or the pack, in circles or rectangles) and NPCs
/// inside the map.
fn content() -> impl Strategy<Value = Parts> {
    (
        name(),
        (metres(0, 1_000_000), metres(0, 1_000_000)),
        (metres(100_000, 200_000), metres(100_000, 200_000)),
        (1u32..=65_535, monster(0, MonsterKind::Monster)),
        monster(0, MonsterKind::Npc),
        (2u32..=1_000_000, item(0)),
        proptest::collection::btree_map(0u8..=7, metres(1, 10_000), 0..=8),
        proptest::collection::vec(
            (
                metres(5_000, 95_000),
                metres(5_000, 95_000),
                metres(0, 5_000),
                1u16..=20,
                metres(0, 8_640_000),
                proptest::option::of(metres(0, 35_999)),
                proptest::option::of((metres(0, 10_000), metres(0, 10_000))),
                any::<bool>(),
            ),
            0..5,
        ),
        proptest::collection::vec(
            (metres(0, 100_000), metres(0, 100_000), metres(0, 35_999)),
            0..3,
        ),
        proptest::collection::vec(1u32..=u32::MAX, 0..6),
        (
            proptest::collection::vec((1u32..=10_000, any::<u8>()), 0..4),
            0u32..=2_000_000_000,
            proptest::collection::vec(
                (
                    any::<u8>(),
                    proptest::collection::vec(0u8..=7, 0..3),
                    any::<bool>(),
                ),
                0..4,
            ),
        ),
        (
            proptest::collection::vec((any::<u8>(), proptest::option::of(0u32..=MAX_PRICE)), 0..4),
            proptest::collection::btree_map(1u32..=254, skill(), 0..4),
            1u8..=254,
            mounts(),
        ),
    )
        .prop_map(
            |(
                name,
                (base_x, base_y),
                (width, height),
                (vnum, mut beast),
                mut keeper,
                (item_vnum, mut thing),
                races,
                groups,
                npcs,
                levels,
                (drops, gold, start_items),
                (goods, skills, start_level, mounts),
            )| {
                beast.vnum = vnum;
                keeper.vnum = if vnum == 65_535 { 1 } else { vnum + 1 };
                let levels_len = levels.len();
                let keeper_vnum = keeper.vnum;
                thing.vnum = item_vnum;
                let map = "test_map".to_owned();
                let spawns = SpawnFile {
                    groups: groups
                        .into_iter()
                        .map(
                            |(x, y, radius, count, respawn, rotation, size, packed)| Group {
                                map: map.clone(),
                                at: [x, y],
                                radius: if size.is_some() { 0.0 } else { radius },
                                size: size.map(|(width, height)| [width, height]),
                                monster: (!packed).then_some(beast.vnum),
                                pack: packed.then_some(1),
                                count,
                                respawn,
                                rotation,
                            },
                        )
                        .collect(),
                    npcs: npcs
                        .into_iter()
                        .map(|(x, y, rotation)| Npc {
                            map: map.clone(),
                            at: [x, y],
                            monster: keeper.vnum,
                            rotation,
                        })
                        .collect(),
                };
                // The beast's gold, by the first drop's chance.
                let gold_chance = drops
                    .first()
                    .map_or(100.0, |(chance, _)| f64::from(*chance) / 100.0);
                let mut parts = Parts {
                    name,
                    maps: vec![Map {
                        name: map,
                        base: [base_x, base_y],
                        size: [width, height],
                        walk: None,
                        water: None,
                    }],
                    packs: vec![Pack {
                        number: 1,
                        leader: beast.vnum,
                        members: vec![PackMember {
                            monster: beast.vnum,
                            count: 2,
                        }],
                    }],
                    monsters: vec![
                        beast.clone(),
                        keeper,
                        Monster {
                            vnum: if beast.vnum <= 65_533 {
                                beast.vnum + 2
                            } else {
                                2
                            },
                            kind: MonsterKind::Stone,
                            calls: vec![1, 1],
                            ..beast.clone()
                        },
                    ],
                    items: vec![thing.clone()],
                    races: races
                        .into_iter()
                        .map(|(race, run_speed)| Race { race, run_speed })
                        .collect(),
                    levels: (1u8..)
                        .zip(levels)
                        .map(|(level, experience)| Level { level, experience })
                        .collect(),
                    drops: drops
                        .into_iter()
                        .map(|(chance, count)| Drop {
                            monster: beast.vnum,
                            item: thing.vnum,
                            count: count.clamp(1, thing.stack()),
                            chance: f64::from(chance) / 100.0,
                        })
                        .collect(),
                    golds: (gold > 0)
                        .then_some(GoldDrop {
                            monster: beast.vnum,
                            amount: [gold / 2, gold],
                            chance: gold_chance,
                        })
                        .into_iter()
                        .collect(),
                    start: Start {
                        level: if levels_len == 0 {
                            start_level
                        } else {
                            start_level.min(u8::try_from(levels_len + 1).unwrap())
                        },
                        gold,
                        items: start_items
                            .into_iter()
                            .map(|(count, races, wear)| StartItem {
                                item: thing.vnum,
                                count: count.clamp(1, thing.stack()),
                                races,
                                wear: wear && thing.wear_position().is_some(),
                            })
                            .collect(),
                    },
                    goods: if thing.size > 8 {
                        // Taller than a shop.
                        Vec::new()
                    } else {
                        goods
                            .into_iter()
                            .filter_map(|(count, price)| {
                                let count = count.clamp(1, thing.stack());
                                // A price at least what the stack sells
                                // back for, and a price at all.
                                let least = u64::from(thing.sell_value()) * u64::from(count);
                                let least = u32::try_from(least)
                                    .ok()
                                    .filter(|least| *least <= MAX_PRICE)?;
                                let default = u64::from(thing.buy_price) * u64::from(count);
                                let default_fits =
                                    default <= u64::from(MAX_PRICE) && default >= u64::from(least);
                                let price = match price {
                                    Some(price) => Some(price.max(least)),
                                    None => (!default_fits).then_some(least),
                                };
                                Some(Good {
                                    npc: keeper_vnum,
                                    item: thing.vnum,
                                    count,
                                    price,
                                })
                            })
                            .collect()
                    },
                    skills: skills
                        .into_iter()
                        .map(|(vnum, skill)| Skill { vnum, ..skill })
                        .collect(),
                    spawns: [("a".to_owned(), spawns)].into_iter().collect(),
                    walks: Default::default(),
                    ..Parts::default()
                };
                // Mounts look like the beast; their call and food are
                // items of their own.
                if let Some(((drain, rest), grades, levels, food)) = mounts {
                    let call = Item {
                        vnum: 2_000_001,
                        name: "Bell Cord".to_owned(),
                        kind: 0,
                        wear_flags: 0,
                        ..thing.clone()
                    };
                    let oats = Item {
                        vnum: 2_000_002,
                        name: "Oat Cake".to_owned(),
                        ..call.clone()
                    };
                    let top = u8::try_from(grades.len()).unwrap();
                    parts.mounts = MountParts {
                        mount: Some(MountRules {
                            summon: call.vnum,
                            drain,
                            rest,
                        }),
                        grades: (1u8..)
                            .zip(grades)
                            .map(|(grade, (fights, skills))| MountGrade {
                                grade,
                                look: beast.vnum,
                                fights,
                                skills,
                            })
                            .collect(),
                        levels: (1u8..)
                            .zip(levels)
                            .map(
                                |(
                                    level,
                                    (move_speed, stamina, rider_level, gold, skill_points, pays),
                                )| MountLevel {
                                    level,
                                    grade: level.min(top),
                                    move_speed,
                                    stamina,
                                    rider_level,
                                    gold,
                                    items: if pays {
                                        vec![MountCost {
                                            item: oats.vnum,
                                            count: 1,
                                        }]
                                    } else {
                                        Vec::new()
                                    },
                                    skill_points,
                                },
                            )
                            .collect(),
                        feeds: vec![MountFeed {
                            item: oats.vnum,
                            stamina: food,
                        }],
                        stables: Vec::new(),
                    };
                    parts.items.push(call);
                    parts.items.push(oats);
                }
                // A keeper without a shop teaches.
                if parts.goods.is_empty() {
                    parts.teachers.push(Teacher {
                        npc: keeper_vnum,
                        job: [
                            SkillJob::Warrior,
                            SkillJob::Ninja,
                            SkillJob::Sura,
                            SkillJob::Shaman,
                        ][usize::try_from(vnum % 4).unwrap()],
                        group: 1 + u8::from(vnum % 2 == 1),
                    });
                }
                // Books and stones train by numbers of their own.
                let percent = u8::try_from(vnum % 100).unwrap();
                parts.training.books = Books {
                    chance: 1 + percent,
                    wait: vnum % 7_200,
                };
                parts.training.stones.chance = 100 - percent;
                // A blade that upgrades once, by a set of its own, and a
                // scroll, by numbers of their own.
                let blade = Item {
                    vnum: 2_000_011,
                    name: "Rust Blade+0".to_owned(),
                    kind: 0,
                    wear_flags: 0,
                    upgrade: 2_000_012,
                    upgrade_set: 1 + u16::from(percent),
                    ..thing.clone()
                };
                let better = Item {
                    vnum: 2_000_012,
                    name: "Rust Blade+1".to_owned(),
                    upgrade: 0,
                    upgrade_set: 0,
                    ..blade.clone()
                };
                let grit = Item {
                    vnum: 2_000_013,
                    name: "Grey Grit".to_owned(),
                    upgrade: 0,
                    upgrade_set: 0,
                    ..blade.clone()
                };
                let scroll = Item {
                    vnum: 2_000_014,
                    name: "Keep Scroll".to_owned(),
                    ..grit.clone()
                };
                let sure = vnum % 2 == 0;
                parts.upgrades = UpgradeParts {
                    smiths: Vec::new(),
                    scrolls: vec![UpgradeScroll {
                        item: scroll.vnum,
                        on_fail: [OnFail::Destroyed, OnFail::Down, OnFail::Kept]
                            [usize::try_from(vnum % 3).unwrap()],
                        chance: if sure { 0 } else { percent },
                        sure,
                        up_to: u8::try_from(vnum % 10).unwrap(),
                    }],
                    sets: vec![UpgradeSet {
                        set: blade.upgrade_set,
                        chance: 1 + percent,
                        gold: vnum % 100_000,
                        items: vec![UpgradeCost {
                            item: grit.vnum,
                            count: 1,
                        }],
                    }],
                };
                parts.items.extend([blade, better, grit, scroll]);
                parts
            },
        )
}

/// A valid folder's text, to damage.
fn valid_files() -> Vec<(String, String)> {
    let mut runner = proptest::test_runner::TestRunner::deterministic();
    let parts = content().new_tree(&mut runner).unwrap().current();
    Content::build(parts).unwrap().to_files(Header::Plain)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn random_text_never_panics(file in 0..FILES.len(), text in "\\PC{0,300}") {
        let mut sources = Sources::new();
        sources.insert("content.toml", "format = 1\nname = \"x\"\n").unwrap();
        sources.insert(FILES[file], text).unwrap();
        let _ = parse(&sources);
    }

    #[test]
    fn damaged_folders_never_panic(
        file in 0..FILES.len(),
        edits in proptest::collection::vec((any::<usize>(), 0usize..8, "\\PC{0,4}"), 1..6),
    ) {
        let mut sources = Sources::new();
        for (path, text) in valid_files() {
            let text = if path == FILES[file] {
                let mut chars: Vec<char> = text.chars().collect();
                for (at, cut, insert) in &edits {
                    let at = at % (chars.len() + 1);
                    let end = (at + cut).min(chars.len());
                    chars.splice(at..end, insert.chars());
                }
                chars.into_iter().collect()
            } else {
                text
            };
            sources.insert(path, text).unwrap();
        }
        let _ = parse(&sources);
    }

    #[test]
    fn valid_content_reads_back_the_same(parts in content()) {
        let content = Content::build(parts).unwrap();
        let mut sources = Sources::new();
        for (path, text) in content.to_files(Header::Imported) {
            sources.insert(path, text).unwrap();
        }
        prop_assert_eq!(parse(&sources).unwrap(), content);
    }
}

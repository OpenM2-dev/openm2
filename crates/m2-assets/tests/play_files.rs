//! What `play` needs of an asset folder: images and regions of them, the
//! interface's skin, the item table and the skills, through TOML and back
//! to the client's types, and what the files refuse.

use std::collections::BTreeMap;

use m2_assets::folder::{resolve, to_toml};
use m2_assets::items_file::{ItemFile, ItemsFile, LEVEL_LIMIT};
use m2_assets::skills::group_names;
use m2_assets::skills_file::{SkillFile, SkillGroupFile, SkillLook, SkillsFile, job_word};
use m2_assets::skin_file::{CornersFile, FontFile, ImageRef, LookFile, SkinFile};
use m2_formats::proto::TypeValue;
use m2_formats::{ItemRecord, RawStr};

#[test]
fn images_are_pngs_or_regions_of_them() {
    let whole = ImageRef::parse("ui/board.png").unwrap();
    assert_eq!(whole.region, None);
    assert_eq!(whole.to_text(), "ui/board.png");
    let part = ImageRef::parse("ui/public.png#xywh=61,0,61,21").unwrap();
    assert_eq!(part.file, "ui/public.png");
    assert_eq!(part.region, Some([61, 0, 61, 21]));
    assert_eq!(part.to_text(), "ui/public.png#xywh=61,0,61,21");
    let resolved = ImageRef::resolve("ui/skin.toml", "../art/a.png#xywh=0,0,4,4").unwrap();
    assert_eq!(resolved.to_text(), "art/a.png#xywh=0,0,4,4");
    for (text, says) in [
        ("", "no file"),
        ("#xywh=0,0,1,1", "no file"),
        ("a.png#t=10", "a region is written"),
        ("a.png#xywh=0,0,1", "four whole numbers"),
        ("a.png#xywh=0,0,-1,4", "four whole numbers"),
        ("a.png#xywh=0,0,0,4", "at least a pixel"),
    ] {
        let error = ImageRef::parse(text).unwrap_err();
        assert!(error.contains(says), "{text:?}: {error}");
    }
    assert!(ImageRef::resolve("skin.toml", "../out.png").is_err());
}

fn look(kind: &str) -> LookFile {
    LookFile {
        kind: kind.to_owned(),
        fallback: Some([1, 2, 3, 4]),
        ..LookFile::default()
    }
}

fn skin() -> SkinFile {
    let mut looks = BTreeMap::new();
    looks.insert(
        "board".to_owned(),
        LookFile {
            corners: Some(CornersFile {
                top_left: Some("corner.png".to_owned()),
                ..CornersFile::default()
            }),
            fill: Some("atlas.png#xywh=0,0,32,32".to_owned()),
            ..look("frame")
        },
    );
    looks.insert(
        "button".to_owned(),
        LookFile {
            normal: Some("atlas.png#xywh=32,0,61,21".to_owned()),
            text_colour: Some([255, 255, 255, 255]),
            ..look("button")
        },
    );
    looks.insert(
        "back".to_owned(),
        LookFile {
            kind: "colour".to_owned(),
            colour: Some([0, 0, 0, 96]),
            ..LookFile::default()
        },
    );
    looks.insert(
        "health".to_owned(),
        LookFile {
            slot: Some("back".to_owned()),
            fill: Some("red.png".to_owned()),
            inset: Some([1.0, 2.0, 1.0, 2.0]),
            ..look("gauge")
        },
    );
    SkinFile {
        format: 1,
        text_colour: [232, 228, 214, 255],
        tooltip: Some("back".to_owned()),
        font: FontFile {
            families: vec!["Tahoma".to_owned()],
            size: 12.0,
        },
        looks,
    }
}

#[test]
fn a_skin_goes_through_toml_and_names_its_images() {
    let file = skin();
    let text = to_toml(&file, true).unwrap();
    for expected in [
        "[look.board]\nkind = \"frame\"\nfill = \"atlas.png#xywh=0,0,32,32\"\nfallback = [1, 2, 3, 4]\n",
        "[look.board.corners]\ntop_left = \"corner.png\"\n",
        "[look.back]\nkind = \"colour\"\ncolour = [0, 0, 0, 96]\n",
    ] {
        assert!(text.contains(expected), "{expected:?} in:\n{text}");
    }
    assert_eq!(SkinFile::parse(&text).unwrap(), file);
    let mut images: Vec<&str> = file.references().into_iter().map(|(_, i)| i).collect();
    images.sort_unstable();
    assert_eq!(
        images,
        [
            "atlas.png#xywh=0,0,32,32",
            "atlas.png#xywh=32,0,61,21",
            "corner.png",
            "red.png"
        ]
    );
}

#[test]
fn a_skin_refuses_what_it_cant_draw() {
    let refused = |change: &dyn Fn(&mut SkinFile), says: &str| {
        let mut file = skin();
        change(&mut file);
        let error = SkinFile::parse(&to_toml(&file, false).unwrap()).unwrap_err();
        assert!(error.contains(says), "{says}: {error}");
    };
    refused(
        &|f| {
            f.looks.insert("x".to_owned(), look("sparkle"));
        },
        "kind `sparkle`",
    );
    refused(
        &|f| {
            f.looks.insert(
                "x".to_owned(),
                LookFile {
                    image: Some("a.png".to_owned()),
                    ..look("bar")
                },
            );
        },
        "`image` is not a key of a bar look",
    );
    refused(
        &|f| {
            f.looks.insert(
                "x".to_owned(),
                LookFile {
                    fallback: None,
                    ..look("image")
                },
            );
        },
        "needs `fallback`",
    );
    refused(
        &|f| {
            f.looks.insert("x".to_owned(), look("button"));
        },
        "needs `text_colour`",
    );
    refused(
        &|f| {
            f.looks.insert(
                "x".to_owned(),
                LookFile {
                    image: Some("a.png#xywh=0,0,0,0".to_owned()),
                    ..look("image")
                },
            );
        },
        "at least a pixel",
    );
    refused(
        &|f| {
            if let Some(health) = f.looks.get_mut("health") {
                health.slot = Some("nothing".to_owned());
            }
        },
        "slot `nothing` is not a look",
    );
    refused(
        &|f| f.tooltip = Some("button".to_owned()),
        "not a panel's look",
    );
    refused(&|f| f.font.families.clear(), "at least one family");
    refused(&|f| f.font.size = 0.0, "size 0");
    let error = SkinFile::parse("format = 1\ntext_colour = [0, 0, 0, 0]\n").unwrap_err();
    assert!(error.contains("font"), "{error}");
}

#[test]
fn items_say_what_the_clients_tables_say() {
    let armour = ItemRecord {
        vnum: 11299,
        locale_name: RawStr::from("Black Steel Armour+9"),
        kind: 2,
        size: 2,
        wear_flags: 1,
        buy_price: 4000,
        sell_price: 400,
        limits: [
            TypeValue {
                kind: LEVEL_LIMIT,
                value: 54,
            },
            TypeValue::default(),
        ],
        values: [0, 0, 0, 3, 0, 0],
        ..ItemRecord::default()
    };
    let item = ItemFile::from_record(
        &armour,
        "Black Steel Armour+9".to_owned(),
        Some("icon/11290.png".to_owned()),
        None,
    );
    assert_eq!((item.shape, item.level), (Some(3), Some(54)));
    let sword = ItemFile {
        vnum: 19,
        name: "Sword+9".to_owned(),
        icon: Some("icons.png#xywh=0,0,32,64".to_owned()),
        model: Some("weapon/sword.glb".to_owned()),
        kind: 1,
        wear_flags: 16,
        glow: 7,
        shine: 100,
        ..ItemFile::default()
    };
    let file = ItemsFile {
        format: 1,
        bag: Some("etc/bag.glb".to_owned()),
        items: vec![item, sword],
    };
    let text = to_toml(&file, false).unwrap();
    assert!(
        text.contains("[[item]]\nvnum = 19\nname = \"Sword+9\"\nicon = \"icons.png#xywh=0,0,32,64\"\nmodel = \"weapon/sword.glb\"\nkind = 1\nwear_flags = 16\nglow = 7\nshine = 100\n"),
        "{text}"
    );
    let read = ItemsFile::parse(&text).unwrap();
    assert_eq!(read, file);
    let items = read
        .to_items(&|reference| resolve("tables/items.toml", reference))
        .unwrap();
    assert_eq!(items.bag.as_deref(), Some(&b"tables/etc/bag.glb"[..]));
    assert_eq!(items.get(19).unwrap().glow, 7);
    assert_eq!(items.get(19).unwrap().record.specular, 100);
    let info = items.get(11299).unwrap();
    assert_eq!(
        info.record.locale_name,
        RawStr::from("Black Steel Armour+9")
    );
    assert_eq!(
        (
            info.record.size,
            info.record.wear_flags,
            info.record.values[3]
        ),
        (2, 1, 3)
    );
    assert_eq!(info.record.limits[0].value, 54);
    assert_eq!((info.record.buy_price, info.record.sell_price), (4000, 400));
    assert_eq!(info.icon.as_deref(), Some(&b"tables/icon/11290.png"[..]));
    let sword = items.get(19).unwrap();
    assert_eq!(
        sword.icon.as_deref(),
        Some(&b"tables/icons.png#xywh=0,0,32,64"[..])
    );
    assert_eq!(
        sword.model.as_deref(),
        Some(&b"tables/weapon/sword.glb"[..])
    );
    assert_eq!(sword.record.size, 1, "1 when left out");
}

#[test]
fn items_refuse_what_the_client_has_no_place_for() {
    for (text, says) in [
        (
            "format = 1\n[[item]]\nvnum = 1\nname = \"A\"\n[[item]]\nvnum = 1\nname = \"B\"\n",
            "written twice",
        ),
        (
            "format = 1\n[[item]]\nvnum = 1\nname = \" \"\n",
            "name is empty",
        ),
        (
            "format = 1\n[[item]]\nvnum = 1\nname = \"A\"\nsize = 4\n",
            "1 to 3 cells",
        ),
        (
            "format = 1\n[[item]]\nvnum = 1\nname = \"A\"\nshape = 2\n",
            "worn on the body",
        ),
        (
            "format = 1\n[[item]]\nvnum = 1\nname = \"A\"\nicon = \"a.png#xywh=1\"\n",
            "icon",
        ),
        (
            "format = 1\n[[item]]\nvnum = 1\nname = \"A\"\ncolour = 2\n",
            "colour",
        ),
    ] {
        let error = ItemsFile::parse(text).unwrap_err();
        assert!(error.contains(says), "{text:?}: {error}");
    }
}

#[test]
fn skills_say_what_the_clients_descriptions_say() {
    let skill = SkillFile {
        vnum: 31,
        job: "ninja".to_owned(),
        name: "Ambush".to_owned(),
        description: Some("Strike from the shadows.".to_owned()),
        notes: vec!["Attack on One Target".to_owned()],
        flags: vec!["attack_skill".to_owned(), "weapon_limitation".to_owned()],
        weapons: vec!["double_sword".to_owned()],
        icons: vec![
            "skill.png#xywh=0,0,32,32".to_owned(),
            "skill.png#xywh=32,0,32,32".to_owned(),
        ],
        motion: Some(1),
        motions: Some("amseup".to_owned()),
        cooldown: Some("15".to_owned()),
        sp_cost: Some("20+40*k".to_owned()),
        target_range: Some(8.5),
        // How it looks while it is on.
        look: Some(SkillLook {
            effect: "look.effect.toml".to_owned(),
            two_handed: Some("look_2.effect.toml".to_owned()),
            bone: Some("equip_right_hand".to_owned()),
            moving: true,
        }),
    };
    let file = SkillsFile {
        format: 1,
        // A group's name, by job and number.
        groups: vec![SkillGroupFile {
            job: "warrior".to_owned(),
            group: 1,
            name: "Body".to_owned(),
        }],
        skills: vec![skill],
    };
    let text = to_toml(&file, false).unwrap();
    let read = SkillsFile::parse(&text).unwrap();
    assert_eq!(read, file);
    assert_eq!(
        read.look_files(),
        [
            ("skill 31's look".to_owned(), "look.effect.toml"),
            ("skill 31's look".to_owned(), "look_2.effect.toml"),
        ]
    );
    assert_eq!(read.cooldowns(), [(31, "15")]);
    assert_eq!(read.costs(), [(31, "20+40*k")]);
    assert_eq!(read.ranges(), [(31, 8.5)]);
    let skills = read
        .to_skills(&|reference| resolve("skills.toml", reference))
        .unwrap();
    let line = skills.get(31).unwrap();
    assert_eq!(line.job, RawStr::from("ASSASSIN"));
    assert_eq!(line.names[0], RawStr::from("Ambush"));
    assert!(line.has_flag("WEAPON_LIMITATION"));
    assert_eq!(line.weapons, [RawStr::from("DOUBLE_SWORD")]);
    assert_eq!(
        (line.icon.clone(), line.motion),
        (RawStr::from("amseup"), Some(1))
    );
    // Its icons by grade; perfect masters show the last.
    let icon = |grade| String::from_utf8(skills.icon(line, grade).unwrap()).unwrap();
    assert_eq!(icon(0), "skill.png#xywh=0,0,32,32");
    assert_eq!(icon(1), "skill.png#xywh=32,0,32,32");
    assert_eq!(icon(3), "skill.png#xywh=32,0,32,32");
    assert_eq!(skills.of_job(1).map(|l| l.vnum).collect::<Vec<_>>(), [31]);
    assert_eq!(job_word(b"ASSASSIN"), "ninja");
    assert_eq!(job_word(b"SUPPORT"), "support");
    // The groups' names, by job and group.
    assert_eq!(skills.group_name(0, 1), Some("Body"));
    assert_eq!(skills.group_name(0, 2), None);
    // As a locale's game texts have them: a key and its text, a tab between.
    let names = group_names(
        b"SKILL_GROUP_ASSASSIN_2\tArc\t\r\nSKILL_GROUP_HORSE\tRiding\nSKILL_GROUP_SURA_3\tX\nSKILL_GROUP_WARRIOR_1\tBody\n",
    );
    assert_eq!(
        names.into_iter().collect::<Vec<_>>(),
        [((0, 1), b"Body".to_vec()), ((1, 2), b"Arc".to_vec())]
    );
}

#[test]
fn skills_refuse_what_the_client_has_no_words_for() {
    let skill = |extra: &str| {
        format!("format = 1\n[[skill]]\nvnum = 1\njob = \"warrior\"\nname = \"A\"\n{extra}")
    };
    for (text, says) in [
        (
            format!(
                "{}[[skill]]\nvnum = 1\njob = \"warrior\"\nname = \"B\"\n",
                skill("")
            ),
            "written twice".to_owned(),
        ),
        (
            skill("flags = [\"Attack Skill\"]\n"),
            "lower-case".to_owned(),
        ),
        (
            skill("notes = [\"1\", \"2\", \"3\", \"4\", \"5\"]\n"),
            "at most 4 notes".to_owned(),
        ),
        (
            skill("icons = [\"a.png\", \"a.png\", \"a.png\", \"a.png\"]\n"),
            "at most 3 icons".to_owned(),
        ),
        (
            skill("motions = \"skill/samyeon\"\n"),
            "not a path".to_owned(),
        ),
        (
            "format = 1\n[[skill]]\nvnum = 1\njob = \"WARRIOR\"\nname = \"A\"\n".to_owned(),
            "job".to_owned(),
        ),
    ] {
        let error = SkillsFile::parse(&text).unwrap_err();
        assert!(error.contains(&says), "{text:?}: {error}");
    }
}

proptest::proptest! {
    #[test]
    fn images_read_back_or_are_refused(text in "[a-z./#=,0-9xywh-]{0,40}") {
        if let Ok(image) = ImageRef::parse(&text) {
            proptest::prop_assert_eq!(ImageRef::parse(&image.to_text()), Ok(image));
        }
    }

    #[test]
    fn damaged_files_are_refused_without_a_panic(text in r"\PC{0,200}") {
        let _ = SkinFile::parse(&text);
        let _ = ItemsFile::parse(&text);
        let _ = SkillsFile::parse(&text);
    }
}

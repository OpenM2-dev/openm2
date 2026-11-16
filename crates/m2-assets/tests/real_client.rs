//! Checks against the user's own client. Skipped unless `OPENM2_CLIENT_DIR`
//! is set; the client's files are read where they are and never copied.
//!
//! ```sh
//! OPENM2_CLIENT_DIR=/path/to/client OPENM2_KEYS=local/keys.toml \
//!   cargo test -p m2-assets --test real_client -- --nocapture
//! ```
//!
//! For a full check of every entry, use `openm2 vfs check`, `gr2 check`,
//! `dds check`, `script check` and `map check` (all run by
//! `just client-check`).

use std::path::PathBuf;

use m2_assets::{Vfs, load_keys, map};
use m2_formats::dds::Dds;
use m2_formats::gr2::Gr2;
use m2_formats::gr2::model::Gr2File;
use m2_formats::mdatr::ModelAttributes;
use m2_formats::mde::MeshEffect;
use m2_formats::script;
use m2_formats::script::effect::EffectData;
use m2_formats::script::motion::{self, MotionData};
use m2_formats::script::race::RaceData;
use m2_formats::{FormatError, Keys, Proto, RawStr};

fn client() -> Option<(PathBuf, Keys)> {
    let dir = PathBuf::from(std::env::var_os("OPENM2_CLIENT_DIR")?);
    let keys = match std::env::var_os("OPENM2_KEYS") {
        Some(path) => load_keys(path.as_ref()).unwrap(),
        None => Keys::none(),
    };
    Some((dir, keys))
}

#[test]
fn every_archive_opens_and_a_sample_of_entries_decodes() {
    let Some((dir, keys)) = client() else {
        eprintln!("skipped: set OPENM2_CLIENT_DIR (and OPENM2_KEYS) to check a real client");
        return;
    };
    let vfs = Vfs::open_client(&dir, &keys).unwrap();
    assert!(
        !vfs.archives().is_empty(),
        "no archives in {}",
        dir.display()
    );
    for archive in vfs.archives() {
        let entries = archive.entries();
        // The first and last 20 entries of each archive.
        let sample = entries
            .iter()
            .take(20)
            .chain(entries.iter().rev().take(20))
            .filter(|entry| entry.kind.is_supported());
        for entry in sample {
            archive.read(entry).unwrap();
        }
        eprintln!("{}: {} entries", archive.path().display(), entries.len());
    }
}

#[test]
fn proto_tables_parse() {
    let Some((dir, keys)) = client() else {
        eprintln!("skipped: set OPENM2_CLIENT_DIR (and OPENM2_KEYS) to check a real client");
        return;
    };
    let vfs = Vfs::open_client(&dir, &keys).unwrap();
    let tables: Vec<Vec<u8>> = vfs
        .files()
        .map(|(name, _, _)| name.to_vec())
        .filter(|name| name.ends_with(b"/item_proto") || name.ends_with(b"/mob_proto"))
        .collect();
    assert!(
        !tables.is_empty(),
        "no item_proto or mob_proto in the archives"
    );
    for name in tables {
        let bytes = vfs.read(&name).unwrap();
        let count = match Proto::parse(&bytes, &keys).unwrap() {
            Proto::Item(table) => table.records.len(),
            Proto::Mob(table) => table.records.len(),
        };
        eprintln!("{}: {count} records", name.escape_ascii());
    }
}

#[test]
fn a_sample_of_gr2_files_reads_as_models() {
    let Some((dir, keys)) = client() else {
        eprintln!("skipped: set OPENM2_CLIENT_DIR (and OPENM2_KEYS) to check a real client");
        return;
    };
    let vfs = Vfs::open_client(&dir, &keys).unwrap();
    let names: Vec<Vec<u8>> = vfs
        .files()
        .map(|(name, _, _)| name.to_vec())
        .filter(|name| name.ends_with(b".gr2"))
        .collect();
    assert!(!names.is_empty(), "no .gr2 files in the archives");
    // Every 25th file: a few hundred, spread over the whole client.
    let (mut read, mut skipped) = (0, 0);
    for name in names.iter().step_by(25) {
        let bytes = vfs.read(name).unwrap();
        let gr2 = match Gr2::parse(&bytes) {
            Ok(gr2) => gr2,
            Err(FormatError::Unsupported { .. }) => {
                skipped += 1;
                continue;
            }
            Err(error) => panic!("{}: {error}", name.escape_ascii()),
        };
        assert!(gr2.crc_ok, "{}: CRC", name.escape_ascii());
        Gr2File::read(&gr2).unwrap_or_else(|e| panic!("{}: {e}", name.escape_ascii()));
        read += 1;
    }
    eprintln!("{read} GR2 files read, {skipped} skipped (format 7)");
}

#[test]
fn a_sample_of_dds_files_decodes() {
    let Some((dir, keys)) = client() else {
        eprintln!("skipped: set OPENM2_CLIENT_DIR (and OPENM2_KEYS) to check a real client");
        return;
    };
    let vfs = Vfs::open_client(&dir, &keys).unwrap();
    let names: Vec<Vec<u8>> = vfs
        .files()
        .map(|(name, _, _)| name.to_vec())
        .filter(|name| name.ends_with(b".dds"))
        .collect();
    assert!(!names.is_empty(), "no .dds files in the archives");
    let mut decoded = 0;
    for name in names.iter().step_by(25) {
        let bytes = vfs.read(name).unwrap();
        let dds = Dds::parse(&bytes).unwrap_or_else(|e| panic!("{}: {e}", name.escape_ascii()));
        for level in 0..dds.levels().len() {
            dds.decode(level)
                .unwrap_or_else(|e| panic!("{} level {level}: {e}", name.escape_ascii()));
        }
        decoded += 1;
    }
    eprintln!("{decoded} DDS files decoded");
}

#[test]
fn model_and_motion_scripts_parse() {
    let Some((dir, keys)) = client() else {
        eprintln!("skipped: set OPENM2_CLIENT_DIR (and OPENM2_KEYS) to check a real client");
        return;
    };
    let vfs = Vfs::open_client(&dir, &keys).unwrap();
    let names: Vec<Vec<u8>> = vfs
        .files()
        .map(|(name, _, _)| name.to_vec())
        .filter(|name| name.ends_with(b".msm") || name.ends_with(b".msa"))
        .collect();
    assert!(!names.is_empty(), "no .msm or .msa files in the archives");
    let (mut typed, mut unreadable) = (0, Vec::new());
    for name in &names {
        let bytes = vfs.read(name).unwrap();
        let parsed =
            script::parse(&bytes).unwrap_or_else(|e| panic!("{}: {e}", name.escape_ascii()));
        let read = match parsed.script_type().map(RawStr::as_bytes) {
            Some(b"RaceDataScript") => RaceData::read(&parsed).map(drop),
            Some(b"MotionData") => MotionData::read(&parsed).map(drop),
            _ => continue,
        };
        match read {
            Ok(()) => typed += 1,
            Err(error) => unreadable.push(format!("{}: {error}", name.escape_ascii())),
        }
    }
    eprintln!(
        "{} scripts parsed, {typed} read as race or motion data, unreadable: {unreadable:#?}",
        names.len()
    );
    // The client ships a few broken or locale-only scripts (`script.md`).
    assert!(unreadable.len() * 100 <= typed, "{unreadable:#?}");
}

#[test]
fn motion_lists_parse() {
    let Some((dir, keys)) = client() else {
        eprintln!("skipped: set OPENM2_CLIENT_DIR (and OPENM2_KEYS) to check a real client");
        return;
    };
    let vfs = Vfs::open_client(&dir, &keys).unwrap();
    let mut lists = 0;
    for (name, _, _) in vfs.files() {
        let file_name = name.rsplit(|b| *b == b'/').next().unwrap_or(name);
        if file_name.starts_with(b"motlist") && file_name.ends_with(b".txt") {
            let bytes = vfs.read(name).unwrap();
            motion::parse_list(&bytes).unwrap_or_else(|e| panic!("{}: {e}", name.escape_ascii()));
            lists += 1;
        }
    }
    assert!(lists > 0, "no motion lists in the archives");
    eprintln!("{lists} motion lists parsed");
}

#[test]
fn maps_and_their_areas_parse() {
    let Some((dir, keys)) = client() else {
        eprintln!("skipped: set OPENM2_CLIENT_DIR (and OPENM2_KEYS) to check a real client");
        return;
    };
    let vfs = Vfs::open_client(&dir, &keys).unwrap();
    let folders = map::map_folders(&vfs);
    assert!(!folders.is_empty(), "no map folders in the archives");
    let mut areas = 0;
    for folder in &folders {
        let shown = folder.escape_ascii();
        let setting = map::read_setting(&vfs, folder).unwrap_or_else(|e| panic!("{shown}: {e}"));
        // Texture sets that are there parse (`map.md`, "Texture sets").
        if map::texture_set_path(&vfs, &setting).is_some() {
            map::read_texture_set(&vfs, &setting).unwrap_or_else(|e| panic!("{shown}: {e}"));
        }
        let [w, h] = setting.size;
        for y in 0..h {
            for x in 0..w {
                if map::has_area(&vfs, folder, x, y) {
                    map::read_area(&vfs, folder, x, y)
                        .unwrap_or_else(|e| panic!("{shown} {x} {y}: {e}"));
                    areas += 1;
                }
            }
        }
    }
    let properties = map::PropertyIndex::load(&vfs);
    assert!(properties.failures.is_empty(), "{:#?}", properties.failures);
    map::read_atlas(&vfs).unwrap();
    eprintln!(
        "{} maps, {areas} areas, {} properties",
        folders.len(),
        properties.len()
    );
}

#[test]
fn effect_scripts_and_mesh_effects_read() {
    let Some((dir, keys)) = client() else {
        eprintln!("skipped: set OPENM2_CLIENT_DIR (and OPENM2_KEYS) to check a real client");
        return;
    };
    let vfs = Vfs::open_client(&dir, &keys).unwrap();
    let names: Vec<Vec<u8>> = vfs
        .files()
        .map(|(name, _, _)| name.to_vec())
        .filter(|name| name.ends_with(b".mse") || name.ends_with(b".mde"))
        .collect();
    let (mut scripts, mut meshes, mut failed) = (0, 0, Vec::new());
    for name in &names {
        let bytes = vfs.read(name).unwrap();
        let read = if name.ends_with(b".mse") {
            scripts += 1;
            EffectData::parse(&bytes).map(drop)
        } else {
            meshes += 1;
            MeshEffect::parse(&bytes).map(drop)
        };
        if let Err(error) = read {
            failed.push(format!("{}: {error}", name.escape_ascii()));
        }
    }
    eprintln!("{scripts} effect scripts and {meshes} mesh effects read; failed: {failed:#?}");
    assert!(scripts > 0 && meshes > 0, "no effects in the archives");
    assert!(failed.is_empty(), "{failed:#?}");
    // A placed effect of the first map reads with what it names.
    let effect = m2_assets::effect::read_effect(
        &vfs,
        b"d:/ymir work/effect/background/fire_general_obj_charcoal.mse",
    )
    .unwrap();
    assert!(!effect.elements.is_empty());
}

#[test]
fn model_attributes_read_and_write_back() {
    let Some((dir, keys)) = client() else {
        eprintln!("skipped: set OPENM2_CLIENT_DIR (and OPENM2_KEYS) to check a real client");
        return;
    };
    let vfs = Vfs::open_client(&dir, &keys).unwrap();
    let names: Vec<Vec<u8>> = vfs
        .files()
        .map(|(name, _, _)| name.to_vec())
        .filter(|name| name.ends_with(b".mdatr"))
        .collect();
    let (mut planes, mut failed) = (0, Vec::new());
    for name in &names {
        let bytes = vfs.read(name).unwrap();
        match ModelAttributes::parse(&bytes) {
            Ok(attributes) => {
                planes += attributes.heights.len();
                if attributes.write().ok().as_deref() != Some(bytes.as_slice()) {
                    failed.push(format!("{}: written back otherwise", name.escape_ascii()));
                }
            }
            Err(error) => failed.push(format!("{}: {error}", name.escape_ascii())),
        }
    }
    eprintln!(
        "{} model attribute files read, {planes} height planes; failed: {failed:#?}",
        names.len()
    );
    assert!(!names.is_empty(), "no model attributes in the archives");
    assert!(failed.is_empty(), "{failed:#?}");
    // The pavilion of the first map has floors.
    let pavilion =
        map::read_model_attributes(&vfs, b"d:/ymir work/zone/a/building/a1-001-house3.gr2")
            .unwrap()
            .unwrap();
    assert!(!pavilion.heights.is_empty());
}

#[test]
fn sounds_sound_scripts_and_sound_spots_read() {
    let Some((dir, keys)) = client() else {
        eprintln!("skipped: set OPENM2_CLIENT_DIR (and OPENM2_KEYS) to check a real client");
        return;
    };
    let vfs = Vfs::open_client(&dir, &keys).unwrap();
    let names: Vec<Vec<u8>> = vfs
        .files()
        .map(|(name, _, _)| name.to_vec())
        .filter(|name| {
            name.ends_with(b".wav")
                || name.ends_with(b".mss")
                || name.ends_with(b"areaambiencedata.txt")
        })
        .collect();
    let (mut waves, mut scripts, mut spots, mut failed) = (0, 0, 0, Vec::new());
    for name in &names {
        let bytes = vfs.read(name).unwrap();
        let read = if name.ends_with(b".wav") {
            waves += 1;
            m2_formats::wav::parse(&bytes).map(drop)
        } else if name.ends_with(b".mss") {
            scripts += 1;
            m2_formats::script::sound::SoundScript::parse(&bytes).map(drop)
        } else {
            m2_formats::map::ambience::AreaAmbience::parse(&bytes)
                .map(|file| spots += file.spots.len())
        };
        if let Err(error) = read {
            failed.push(format!("{}: {error}", name.escape_ascii()));
        }
    }
    eprintln!(
        "{waves} sounds, {scripts} sound scripts, {spots} sound spots read; failed: {failed:#?}"
    );
    assert!(waves > 0 && scripts > 0, "no sounds in the archives");
    assert!(failed.is_empty(), "{failed:#?}");
    let settings = m2_assets::sound::read_sound_settings(&dir).unwrap();
    eprintln!("metin2.cfg: {settings:?}");
    assert!(!m2_assets::sound::music_names(&dir, &vfs).is_empty());
}

#[test]
fn atlas_pictures_points_and_area_pictures_read() {
    let Some((dir, keys)) = client() else {
        eprintln!("skipped: set OPENM2_CLIENT_DIR (and OPENM2_KEYS) to check a real client");
        return;
    };
    let vfs = Vfs::open_client(&dir, &keys).unwrap();
    let atlas = m2_assets::map::read_atlas(&vfs).unwrap();
    let folders = m2_assets::map::map_folders(&vfs);
    let (mut pictures, mut point_files, mut points) = (0, 0, 0);
    for entry in &atlas {
        let folder = m2_formats::normalize_name(entry.map.as_bytes());
        if !folders.contains(&folder) {
            continue;
        }
        pictures += usize::from(
            m2_assets::map::read_atlas_picture(&vfs, &folder)
                .unwrap()
                .is_some(),
        );
        if let Some(list) = m2_assets::map::read_points(&vfs, "en", &folder).unwrap() {
            point_files += 1;
            points += list.len();
        }
        // The first area's picture from above, where the map has one.
        let _ = m2_assets::map::read_minimap(&vfs, &folder, 0, 0).unwrap();
    }
    eprintln!("{pictures} atlas pictures, {point_files} point files with {points} points");
    assert!(
        pictures > 0 && point_files > 0,
        "no atlas pictures or points"
    );
}

#[test]
fn skills_find_their_icons_and_motions() {
    let Some((dir, keys)) = client() else {
        eprintln!("skipped: set OPENM2_CLIENT_DIR (and OPENM2_KEYS) to check a real client");
        return;
    };
    let vfs = Vfs::open_client(&dir, &keys).unwrap();
    let skills = m2_assets::skills::read_skills(&vfs, "en").unwrap();
    let bases = [
        "d:/ymir work/pc/warrior/",
        "d:/ymir work/pc/assassin/",
        "d:/ymir work/pc/sura/",
        "d:/ymir work/pc/shaman/",
    ];
    let (mut used, mut own) = (0, 0);
    for (job, base) in (0u8..).zip(bases) {
        for line in skills.of_job(job) {
            used += 1;
            // The motion number is the skill's place in its job's numbering.
            assert_eq!(
                line.motion,
                Some(line.vnum - 30 * u32::from(job)),
                "{}",
                line.vnum
            );
            for grade in 0..4 {
                let icon = m2_assets::skills::skill_icon(line, grade).unwrap();
                assert!(vfs.source(&icon).is_some(), "{}", icon.escape_ascii());
                let model = format!("{base}x.gr2");
                let motions = m2_assets::skills::skill_motion_paths(
                    model.as_bytes(),
                    line.icon.as_bytes(),
                    grade,
                );
                let found = motions.iter().position(|path| vfs.source(path).is_some());
                assert!(found.is_some(), "{}: no motion", line.vnum);
                own += usize::from(found == Some(0));
            }
        }
    }
    eprintln!(
        "{} skills, {used} used by a job; {own} of their {} grades have a motion of their own",
        skills.len(),
        used * 4
    );
    assert_eq!(used, 44);
}

//! Map commands: `map ls|info|attr|check`.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::Subcommand;
use m2_assets::Vfs;
use m2_assets::items::read_locale;
use m2_assets::map::{
    self, PropertyIndex, atlas_picture_path, environment_path, has_area, map_folders, points_path,
    read_area, read_atlas, read_atlas_picture, read_attributes, read_minimap,
    read_model_attributes, read_parsed, read_points, read_setting, texture_set_path,
};
use m2_formats::dds::Dds;
use m2_formats::gr2::Gr2;
use m2_formats::gr2::model::Gr2File;
use m2_formats::map::area_folder;
use m2_formats::map::grid::{AttrMap, WaterMap};
use m2_formats::map::points::{self, PointKind};
use m2_formats::map::property::PropertyKind;
use m2_formats::map::setting::MapSetting;
use m2_formats::map::texture_set::TextureSet;
use m2_formats::mdatr::Shape;
use m2_formats::{RgbaImage, normalize_name};

use crate::files::{KeysArg, name_bytes, open_vfs};
use crate::scripts::{SHOWN_PROBLEMS, Tally};

/// `openm2 map …`
#[derive(Debug, Subcommand)]
pub enum MapCommand {
    /// List the client's maps: folder, size in areas, areas present, and
    /// where the atlas puts them
    Ls {
        /// The client folder (archives in its `pack` folder)
        #[arg(long, value_name = "DIR")]
        client: PathBuf,
        #[command(flatten)]
        keys: KeysArg,
    },
    /// What a map is made of: settings, texture set, areas, objects, sound
    /// spots, its atlas picture and points
    Info {
        /// The map's folder, e.g. `metin2_map_a1`
        map: String,
        /// The client folder (archives in its `pack` folder)
        #[arg(long, value_name = "DIR")]
        client: PathBuf,
        #[command(flatten)]
        keys: KeysArg,
    },
    /// Draw a map's movement attributes (`attr.atr`) over its areas'
    /// pictures into a PNG, a pixel a square, and count each flag: where it
    /// is set and how much of it is under water
    Attr {
        /// The map's folder, e.g. `metin2_map_a1`
        map: String,
        /// The client folder (archives in its `pack` folder)
        #[arg(long, value_name = "DIR")]
        client: PathBuf,
        #[command(flatten)]
        keys: KeysArg,
        /// The flags drawn: squares with any of them are tinted red
        #[arg(long, default_value = "0x01", value_parser = parse_flags)]
        bits: u8,
        /// The PNG written
        #[arg(long, value_name = "FILE")]
        out: PathBuf,
    },
    /// Parse every map of a client (settings, texture sets, every area's
    /// files, property files) and check the files they name. Exit code 1 if
    /// a file a map uses cannot be parsed.
    Check {
        /// The client folder (archives in its `pack` folder)
        #[arg(long, value_name = "DIR")]
        client: PathBuf,
        #[command(flatten)]
        keys: KeysArg,
    },
}

/// Runs a `map` command.
pub fn map(command: MapCommand) -> Result<ExitCode> {
    match command {
        MapCommand::Ls { client, keys } => ls(&open_vfs(&client, &keys.load()?)?),
        MapCommand::Info { map, client, keys } => {
            let locale = read_locale(&client).ok().map(|locale| locale.name);
            info(
                &open_vfs(&client, &keys.load()?)?,
                &name_bytes(&map)?,
                locale.as_deref(),
            )
        }
        MapCommand::Attr {
            map,
            client,
            keys,
            bits,
            out,
        } => attr(
            &open_vfs(&client, &keys.load()?)?,
            &name_bytes(&map)?,
            bits,
            &out,
        ),
        MapCommand::Check { client, keys } => check(&client, &keys),
    }
}

/// A byte of flags: `0x01` or `1`.
fn parse_flags(text: &str) -> Result<u8, String> {
    let parsed = match text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        Some(hex) => u8::from_str_radix(hex, 16),
        None => text.parse(),
    };
    parsed.map_err(|_| format!("`{text}` is not a byte (0 to 255, or 0x00 to 0xff)"))
}

/// Squares an area's attributes have on a side in the client.
const ATTR_SIDE: u32 = 256;

fn attr(vfs: &Vfs, map: &[u8], bits: u8, out: &Path) -> Result<ExitCode> {
    let folder = normalize_name(map);
    let folder = folder.strip_suffix(b"/").unwrap_or(&folder).to_vec();
    let setting = read_setting(vfs, &folder).context("reading the map's settings")?;
    let [w, h] = setting.size;
    let mut picture = RgbaImage::filled(w * ATTR_SIDE, h * ATTR_SIDE, [0, 0, 0, 255])?;
    let (mut areas, mut squares, mut wet) = (0, 0u64, 0u64);
    let (mut set, mut set_wet) = ([0u64; 8], [0u64; 8]);
    let mut values: BTreeMap<u8, u64> = BTreeMap::new();
    for y in 0..h {
        for x in 0..w {
            let Some(attributes) = read_attributes(vfs, &folder, x, y)? else {
                continue;
            };
            areas += 1;
            let minimap = read_minimap(vfs, &folder, x, y)?;
            let water = area_folder(&folder, x, y)
                .map(|f| [f.as_slice(), b"water.wtr"].concat())
                .filter(|path| vfs.source(path).is_some())
                .map(|path| read_parsed(vfs, &path, WaterMap::parse))
                .transpose()?;
            let grid = &attributes.grid;
            for sy in 0..grid.height().min(ATTR_SIDE) {
                for sx in 0..grid.width().min(ATTR_SIDE) {
                    let flags = grid.get(sx, sy).unwrap_or(0);
                    // Water cells are two squares wide.
                    let under = water.as_ref().is_some_and(|water| {
                        water
                            .layers
                            .get(sx / 2, sy / 2)
                            .is_some_and(|layer| usize::from(layer) < water.heights.len())
                    });
                    squares += 1;
                    wet += u64::from(under);
                    *values.entry(flags).or_default() += 1;
                    for (bit, (set, set_wet)) in set.iter_mut().zip(&mut set_wet).enumerate() {
                        if flags & (1 << bit) != 0 {
                            *set += 1;
                            *set_wet += u64::from(under);
                        }
                    }
                    let ground = minimap
                        .as_ref()
                        .and_then(|m| m.pixel(sx, sy))
                        .unwrap_or([128, 128, 128, 255]);
                    let shown = if flags & bits != 0 {
                        let tint = |c: u8, t: u8| ((u16::from(c) * 2 + u16::from(t) * 3) / 5) as u8;
                        [
                            tint(ground[0], 255),
                            tint(ground[1], 0),
                            tint(ground[2], 0),
                            255,
                        ]
                    } else {
                        [ground[0], ground[1], ground[2], 255]
                    };
                    picture.set_pixel(x * ATTR_SIDE + sx, y * ATTR_SIDE + sy, shown);
                }
            }
        }
    }
    std::fs::write(out, m2_render::png::encode(&picture)?)
        .with_context(|| format!("writing {}", out.display()))?;
    let share = |n: u64| n as f64 * 100.0 / squares.max(1) as f64;
    println!(
        "map {}: {areas} areas with attributes, {squares} squares, {:.1} % under water",
        folder.escape_ascii(),
        share(wet)
    );
    println!("  flag  squares      share  under water");
    for (bit, (n, n_wet)) in set.iter().zip(&set_wet).enumerate() {
        println!(
            "  {:#04x}  {n:>10}  {:>5.1} %  {:>5.1} %",
            1u8 << bit,
            share(*n),
            *n_wet as f64 * 100.0 / (*n).max(1) as f64
        );
    }
    let mut common: Vec<(u8, u64)> = values.into_iter().collect();
    common.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let shown: Vec<String> = common
        .iter()
        .take(12)
        .map(|(value, n)| format!("{value:#04x} {:.1} %", share(*n)))
        .collect();
    println!("  values: {}", shown.join(", "));
    println!(
        "wrote {} ({}×{}, a pixel a metre, north up): flags {bits:#04x} tinted red",
        out.display(),
        picture.width(),
        picture.height()
    );
    Ok(ExitCode::SUCCESS)
}

fn ls(vfs: &Vfs) -> Result<ExitCode> {
    let atlas = read_atlas(vfs).unwrap_or_default();
    let folders = map_folders(vfs);
    for folder in &folders {
        let shown = folder.escape_ascii().to_string();
        let placed = atlas
            .iter()
            .find(|e| normalize_name(e.map.as_bytes()) == *folder)
            .map_or_else(
                || "not in the atlas".to_owned(),
                |e| format!("at {} {}", e.base[0], e.base[1]),
            );
        match read_setting(vfs, folder) {
            Ok(setting) => {
                let [w, h] = setting.size;
                let present = (0..w)
                    .flat_map(|x| (0..h).map(move |y| (x, y)))
                    .filter(|&(x, y)| has_area(vfs, folder, x, y))
                    .count();
                println!("{shown}  {w}×{h} areas, {present} present, {placed}");
            }
            Err(error) => println!("{shown}  unreadable: {error}"),
        }
    }
    for entry in &atlas {
        if !folders.contains(&normalize_name(entry.map.as_bytes())) {
            println!("{}  in the atlas, not in the client", entry.map);
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn info(vfs: &Vfs, map: &[u8], locale: Option<&str>) -> Result<ExitCode> {
    let folder = normalize_name(map);
    let folder = folder.strip_suffix(b"/").unwrap_or(&folder).to_vec();
    let setting = read_setting(vfs, &folder).context("reading the map's settings")?;
    let [w, h] = setting.size;
    println!("map {}", folder.escape_ascii());
    println!(
        "  {w}×{h} areas of 128×128 cells of {} cm; heights in steps of {} cm",
        setting.cell_scale, setting.height_scale
    );
    if let Some([x, y]) = setting.base_position {
        println!("  top left corner in the world: {x} {y}");
    }
    let environment = environment_path(&setting);
    println!(
        "  environment: {}",
        environment.as_ref().map_or_else(
            || "none".to_owned(),
            |p| present(vfs, p, &p.escape_ascii().to_string())
        )
    );
    let set = match texture_set_path(vfs, &setting) {
        Some(path) => {
            let set = read_parsed(vfs, &path, TextureSet::parse)?;
            println!(
                "  texture set {}: {} texture numbers",
                path.escape_ascii(),
                set.textures.len()
            );
            for (n, texture) in set.textures.iter().enumerate() {
                let Some(texture) = texture else {
                    println!("    {:>3}  (no texture)", n + 1);
                    continue;
                };
                let path = normalize_name(texture.file.as_bytes());
                println!(
                    "    {:>3}  {}  scale {} {}",
                    n + 1,
                    present(vfs, &path, &texture.file.to_string()),
                    texture.scale[0],
                    texture.scale[1]
                );
            }
            Some(set)
        }
        None => {
            println!("  texture set {}: not in the client", setting.texture_set);
            None
        }
    };
    let properties = PropertyIndex::load(vfs);
    let mut kinds: BTreeMap<String, usize> = BTreeMap::new();
    // Effect scripts: how many objects play each, and where the first is
    // (metres from the map's top left corner).
    let mut effects: BTreeMap<Vec<u8>, (usize, [f32; 2])> = BTreeMap::new();
    let mut unknown = Tally::default();
    // Sound spots: the sound, where (metres), how far it is heard (metres).
    let mut spots: Vec<(String, [f32; 2], f32)> = Vec::new();
    let (mut areas, mut holes, mut objects, mut water) = (0, 0, 0, 0);
    let (mut low, mut high) = (u16::MAX, 0);
    let mut lacking = Tally::default();
    for y in 0..h {
        for x in 0..w {
            if !has_area(vfs, &folder, x, y) {
                holes += 1;
                continue;
            }
            let area = read_area(vfs, &folder, x, y)?;
            areas += 1;
            for v in area.heights.grid.values() {
                low = low.min(*v);
                high = high.max(*v);
            }
            water += area.water.as_ref().map_or(0, |w| w.heights.len());
            if let Some(set) = &set {
                for tile in area.tiles.grid.values() {
                    lacking.record(*tile == 0 || set.texture(*tile).is_some(), || {
                        format!("{tile} in area {x} {y}")
                    });
                }
            }
            for object in &area.objects.objects {
                objects += 1;
                match properties.get(object.property) {
                    Some((_, property)) => {
                        *kinds.entry(kind_name(property.kind())).or_default() += 1;
                        if let Some(effect) = property.effect() {
                            let [px, py, _] = object.position;
                            let at = [px / 100.0, -py / 100.0];
                            effects
                                .entry(normalize_name(effect.as_bytes()))
                                .or_insert((0, at))
                                .0 += 1;
                        }
                    }
                    None => {
                        unknown.record(false, || format!("{} in area {x} {y}", object.property))
                    }
                }
            }
            for spot in &area.ambience.spots {
                let sound = properties
                    .get(spot.property)
                    .and_then(|(_, property)| property.ambience().ok().flatten())
                    .map_or_else(
                        || format!("property {} (not an ambience in the client)", spot.property),
                        |ambience| {
                            let path = normalize_name(ambience.sound.as_bytes());
                            present(vfs, &path, &path.escape_ascii().to_string())
                        },
                    );
                let [px, py, _] = spot.position;
                spots.push((sound, [px / 100.0, -py / 100.0], spot.range / 100.0));
            }
        }
    }
    let scale = setting.height_scale;
    println!(
        "  {areas} areas{}; heights {:.0} to {:.0} cm; {water} water layers",
        if holes > 0 {
            format!(" ({holes} missing)")
        } else {
            String::new()
        },
        f32::from(low) * scale,
        f32::from(high) * scale
    );
    if lacking.bad > 0 {
        println!(
            "  tiles{}",
            lacking.problems("name a texture number the set lacks")
        );
    }
    println!(
        "  {objects} objects: {}{}",
        kinds
            .iter()
            .map(|(k, n)| format!("{k} {n}"))
            .collect::<Vec<_>>()
            .join(", "),
        if unknown.bad > 0 {
            format!(
                ", {} with a property not in the client (e.g. {})",
                unknown.bad,
                unknown.first.unwrap_or_default()
            )
        } else {
            String::new()
        }
    );
    let mut by_count: Vec<_> = effects.iter().collect();
    by_count.sort_by(|a, b| b.1.0.cmp(&a.1.0).then(a.0.cmp(b.0)));
    for (path, (count, [x, y])) in by_count {
        println!(
            "    effect {} ×{count}, the first at {x:.0},{y:.0} m",
            present(vfs, path, &path.escape_ascii().to_string())
        );
    }
    for (sound, [x, y], range) in &spots {
        println!("    sound {sound} at {x:.0},{y:.0} m, heard to {range:.0} m");
    }
    let picture_path = atlas_picture_path(&folder);
    match read_atlas_picture(vfs, &folder)? {
        Some(picture) => println!(
            "  atlas picture {}: {}×{}, {:.1} m a pixel",
            picture_path.escape_ascii(),
            picture.width(),
            picture.height(),
            f64::from(w) * 256.0 / f64::from(picture.width().max(1))
        ),
        None => println!("  atlas picture {}: none", picture_path.escape_ascii()),
    }
    if let Some(locale) = locale {
        let path = points_path(locale, &folder);
        match read_points(vfs, locale, &folder)? {
            Some(list) => {
                let count = |kind: PointKind| list.iter().filter(|p| p.kind == kind).count();
                println!(
                    "  atlas points {}: {} NPCs, {} warps",
                    path.escape_ascii(),
                    count(PointKind::Npc),
                    count(PointKind::Warp)
                );
                for point in &list {
                    let kind = match &point.kind {
                        PointKind::Npc => "NPC".to_owned(),
                        PointKind::Warp => "warp".to_owned(),
                        PointKind::Other(word) => format!("`{word}`"),
                    };
                    let [x, y] = point.position;
                    println!("    {kind} at {},{} m", x / 100, y / 100);
                }
            }
            None => println!("  atlas points {}: none", path.escape_ascii()),
        }
    }
    Ok(ExitCode::SUCCESS)
}

/// The text, followed by "(missing)" when the client lacks `path`.
fn present(vfs: &Vfs, path: &[u8], shown: &str) -> String {
    if vfs.source(path).is_some() {
        shown.to_owned()
    } else {
        format!("{shown} (missing)")
    }
}

fn kind_name(kind: Option<PropertyKind>) -> String {
    match kind {
        Some(PropertyKind::Building) => "buildings".to_owned(),
        Some(PropertyKind::DungeonBlock) => "dungeon blocks".to_owned(),
        Some(PropertyKind::Tree) => "trees".to_owned(),
        Some(PropertyKind::Effect) => "effects".to_owned(),
        Some(PropertyKind::Ambience) => "ambience".to_owned(),
        Some(PropertyKind::Other(other)) => format!("`{other}`"),
        None => "no type".to_owned(),
    }
}

/// What `check` counts over all maps.
#[derive(Default)]
struct Counts {
    maps: usize,
    areas: usize,
    holes: Tally,
    texture_sets: Tally,
    textures: Tally,
    tiles_lacking: Tally,
    environments: Tally,
    attributes: Tally,
    minimaps: Tally,
    /// The areas' pictures by size and format: `256×256 DXT1`.
    minimap_kinds: BTreeMap<String, usize>,
    objects: Tally,
    kinds: BTreeMap<String, usize>,
    models: Tally,
    unreadable_models: Tally,
    /// Models with attributes (`mdatr.md`), their collision shapes by
    /// kind, their height planes and those planes' triangles.
    attributed: usize,
    shapes: BTreeMap<&'static str, usize>,
    height_planes: usize,
    floor_triangles: usize,
    water_layers: usize,
    attribute_flags: BTreeMap<u8, u64>,
    /// Values seen per setting: `CellScale 200`, `MapType Outdoor`, …
    settings: BTreeMap<String, usize>,
    /// Texture scales seen, smallest and largest.
    scales: Option<(f32, f32)>,
    /// Objects tilted (first or second rotation not 0).
    tilted: usize,
    /// Objects with a height offset, and the smallest and largest.
    offsets: (usize, f32, f32),
    /// Objects with more than four lines, and their extra values.
    longer: Tally,
    extra_values: BTreeMap<String, usize>,
    /// Building and dungeon block models already checked.
    models_seen: HashSet<Vec<u8>>,
    /// Texture sets already read.
    sets_seen: BTreeSet<Vec<u8>>,
}

impl Counts {
    fn setting(&mut self, what: String) {
        *self.settings.entry(what).or_default() += 1;
    }
}

fn check(client: &Path, keys: &KeysArg) -> Result<ExitCode> {
    let vfs = open_vfs(client, &keys.load()?)?;
    let mut failures: Vec<String> = Vec::new();
    let mut unreadable: Vec<String> = Vec::new();
    let mut counts = Counts::default();

    let properties = PropertyIndex::load(&vfs);
    for (path, error) in &properties.failures {
        failures.push(format!("{}: {error}", path.escape_ascii()));
    }
    let mut property_kinds: BTreeMap<String, usize> = BTreeMap::new();
    for (_, _, property) in properties.iter() {
        *property_kinds
            .entry(kind_name(property.kind()))
            .or_default() += 1;
    }

    let atlas = match read_atlas(&vfs) {
        Ok(atlas) => atlas,
        Err(error) => {
            failures.push(error.to_string());
            Vec::new()
        }
    };
    let folders = map_folders(&vfs);
    // Map sizes in areas by the folder's last part, for the atlas points.
    let mut sizes: BTreeMap<Vec<u8>, [u32; 2]> = BTreeMap::new();
    for folder in &folders {
        let shown = folder.escape_ascii().to_string();
        let setting = match read_setting(&vfs, folder) {
            Ok(setting) => setting,
            Err(error) => {
                failures.push(error.to_string());
                continue;
            }
        };
        if let Some(name) = folder.rsplit(|b| *b == b'/').next() {
            sizes.insert(name.to_vec(), setting.size);
        }
        counts.maps += 1;
        counts.setting(format!("CellScale {}", setting.cell_scale));
        counts.setting(format!("HeightScale {}", setting.height_scale));
        counts.setting(match setting.view_radius {
            Some(radius) => format!("ViewRadius {radius}"),
            None => "no ViewRadius".to_owned(),
        });
        let [w, h] = setting.size;
        counts.setting(format!("MapSize side {}", w.max(h)));
        if setting.base_position.is_none() {
            counts.setting("no BasePosition".to_owned());
        }
        let property_file = map::map_file(folder, b"mapproperty.txt");
        if vfs.source(&property_file).is_some() {
            match read_parsed(&vfs, &property_file, m2_formats::script::parse) {
                Ok(script) => counts.setting(match script.values("MapType") {
                    Some([value, ..]) => format!("MapType {}", value.text),
                    _ => "no MapType".to_owned(),
                }),
                Err(error) => failures.push(error.to_string()),
            }
        }
        check_map(
            &vfs,
            folder,
            &shown,
            &setting,
            &properties,
            &mut counts,
            &mut failures,
        );
    }
    // Texture set files no map uses: listed, not failed.
    for (name, _, _) in vfs.files() {
        if name.starts_with(map::TEXTURE_SET_FOLDER) && !counts.sets_seen.contains(name) {
            let shown = name.escape_ascii().to_string();
            match read_parsed(&vfs, name, TextureSet::parse) {
                Ok(_) => {}
                Err(error) => unreadable.push(format!("{shown} (no map uses it): {error}")),
            }
        }
    }

    let in_atlas = atlas
        .iter()
        .filter(|e| folders.contains(&normalize_name(e.map.as_bytes())))
        .count();
    println!(
        "atlas: {} maps, {in_atlas} of them in the client; {} map folders",
        atlas.len(),
        folders.len()
    );
    // The atlas pictures of the maps the atlas and the client both have.
    let mut pictures = Tally::default();
    for entry in &atlas {
        let folder = normalize_name(entry.map.as_bytes());
        if !folders.contains(&folder) {
            continue;
        }
        match read_atlas_picture(&vfs, &folder) {
            Ok(picture) => pictures.record(picture.is_some(), || entry.map.to_string()),
            Err(error) => failures.push(error.to_string()),
        }
    }
    println!(
        "  atlas pictures: {}{}",
        pictures.total,
        pictures.problems("maps without one")
    );
    check_points(&vfs, &sizes, &mut failures);
    println!(
        "maps: {} read, {} areas{}",
        counts.maps,
        counts.areas,
        counts.holes.problems("missing (holes in their map)")
    );
    println!(
        "  texture sets: {} named, {} different{}; {} textures{}{}",
        counts.texture_sets.total,
        counts.sets_seen.len(),
        counts.texture_sets.problems("missing"),
        counts.textures.total,
        counts.textures.problems("missing"),
        match counts.scales {
            Some((low, high)) => format!("; scales from {low} to {high}"),
            None => String::new(),
        }
    );
    println!(
        "  tiles: {} areas{}",
        counts.tiles_lacking.total,
        counts
            .tiles_lacking
            .problems("name texture numbers their set lacks")
    );
    println!(
        "  environments: {} named{}",
        counts.environments.total,
        counts.environments.problems("missing")
    );
    println!(
        "  attribute maps: {}{}; minimaps: {}{} ({})",
        counts.attributes.total,
        counts.attributes.problems("missing"),
        counts.minimaps.total,
        counts.minimaps.problems("missing"),
        counts
            .minimap_kinds
            .iter()
            .map(|(k, n)| format!("{k} {n}"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    println!(
        "  attribute flags (squares): {}",
        counts
            .attribute_flags
            .iter()
            .map(|(flags, n)| format!("{flags:#04x} {n}"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    println!("  water layers: {}", counts.water_layers);
    println!(
        "  settings: {}",
        counts
            .settings
            .iter()
            .map(|(k, n)| format!("{k} ({n})"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    println!(
        "  objects: {}{}",
        counts.objects.total,
        counts.objects.problems("with a property not in the client")
    );
    let (offset, low, high) = counts.offsets;
    println!(
        "    {} tilted, {offset} with a height offset (from {low} to {high} cm){}",
        counts.tilted,
        counts.longer.problems("with more than four lines")
    );
    if !counts.extra_values.is_empty() {
        println!(
            "    lines after the fourth: {}",
            counts
                .extra_values
                .iter()
                .map(|(k, n)| format!("`{k}` {n}"))
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    println!(
        "    {}",
        counts
            .kinds
            .iter()
            .map(|(k, n)| format!("{k} {n}"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    println!(
        "  building and dungeon block models: {} distinct{}{}",
        counts.models.total,
        counts.models.problems("missing"),
        counts.unreadable_models.problems("unreadable")
    );
    println!(
        "    with attributes: {} ({}; {} height planes, {} triangles)",
        counts.attributed,
        if counts.shapes.is_empty() {
            "no collision shapes".to_owned()
        } else {
            counts
                .shapes
                .iter()
                .map(|(k, n)| format!("{k} {n}"))
                .collect::<Vec<_>>()
                .join(", ")
        },
        counts.height_planes,
        counts.floor_triangles
    );
    println!(
        "properties: {} ({}){}",
        properties.len(),
        property_kinds
            .iter()
            .map(|(k, n)| format!("{k} {n}"))
            .collect::<Vec<_>>()
            .join(", "),
        match properties.duplicates.first() {
            Some((id, path)) => format!(
                "; {} files repeat an id (e.g. {id} in {})",
                properties.duplicates.len(),
                path.escape_ascii()
            ),
            None => String::new(),
        }
    );
    for line in unreadable.iter().take(SHOWN_PROBLEMS) {
        println!("unreadable {line}");
    }
    for failure in failures.iter().take(SHOWN_PROBLEMS) {
        println!("FAILED {failure}");
    }
    if failures.len() > SHOWN_PROBLEMS {
        println!("… and {} more", failures.len() - SHOWN_PROBLEMS);
    }
    Ok(if failures.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

/// Reads every atlas point file of the client (`…_point.txt`, in every
/// locale) and says how many points of each kind they hold, and how many
/// lie outside their map (`sizes`: maps' sizes in areas by name).
fn check_points(vfs: &Vfs, sizes: &BTreeMap<Vec<u8>, [u32; 2]>, failures: &mut Vec<String>) {
    let mut files = 0;
    let mut kinds: BTreeMap<String, usize> = BTreeMap::new();
    let mut outside = Tally::default();
    let names: Vec<Vec<u8>> = vfs
        .files()
        .map(|(name, _, _)| name.to_vec())
        .filter(|name| name.ends_with(b"_point.txt"))
        .collect();
    for name in &names {
        let list = match read_parsed(vfs, name, points::parse) {
            Ok(list) => list,
            Err(error) => {
                failures.push(error.to_string());
                continue;
            }
        };
        files += 1;
        // `metin2_map_a1_point.txt`, or with a codepage in front:
        // `949_metin2_map_a1_point.txt`.
        let file = name.rsplit(|b| *b == b'/').next().unwrap_or(name);
        let map = file.strip_suffix(b"_point.txt").unwrap_or(file);
        let map = match map.split_first_chunk::<4>() {
            Some(([a, b, c, b'_'], rest)) if [a, b, c].iter().all(|d| d.is_ascii_digit()) => rest,
            _ => map,
        };
        let size = sizes.get(map);
        for point in &list {
            let kind = match &point.kind {
                PointKind::Npc => "NPC".to_owned(),
                PointKind::Warp => "WARP".to_owned(),
                PointKind::Other(word) => format!("`{word}`"),
            };
            *kinds.entry(kind).or_default() += 1;
            if let Some([w, h]) = size {
                let [x, y] = point.position;
                let inside =
                    u64::from(x) < u64::from(*w) * 25_600 && u64::from(y) < u64::from(*h) * 25_600;
                outside.record(inside, || {
                    format!("{} {x} {y} in {}", point.number, name.escape_ascii())
                });
            }
        }
    }
    println!(
        "  atlas points: {files} files, {} points ({}){}",
        kinds.values().sum::<usize>(),
        kinds
            .iter()
            .map(|(k, n)| format!("{k} {n}"))
            .collect::<Vec<_>>()
            .join(", "),
        outside.problems("outside their map")
    );
}

/// Reads the attributes of the model at `path`, if it has any, and counts
/// what they hold.
fn check_model_attributes(vfs: &Vfs, path: &[u8], counts: &mut Counts, failures: &mut Vec<String>) {
    match read_model_attributes(vfs, path) {
        Ok(Some(attributes)) => {
            counts.attributed += 1;
            for collision in &attributes.collisions {
                let kind = match collision.shape {
                    Shape::Plane { .. } => "planes",
                    Shape::Sphere { .. } => "spheres",
                    Shape::Cylinder { .. } => "cylinders",
                };
                *counts.shapes.entry(kind).or_default() += 1;
            }
            counts.height_planes += attributes.heights.len();
            counts.floor_triangles += attributes
                .heights
                .iter()
                .map(|plane| plane.corners.len() / 3)
                .sum::<usize>();
        }
        Ok(None) => {}
        Err(error) => failures.push(error.to_string()),
    }
}

fn check_map(
    vfs: &Vfs,
    folder: &[u8],
    shown: &str,
    setting: &MapSetting,
    properties: &PropertyIndex,
    counts: &mut Counts,
    failures: &mut Vec<String>,
) {
    let set_path = texture_set_path(vfs, setting);
    counts.texture_sets.record(set_path.is_some(), || {
        format!("{} for {shown}", setting.texture_set)
    });
    let set = set_path.and_then(|path| {
        let parsed = read_parsed(vfs, &path, TextureSet::parse);
        let first = counts.sets_seen.insert(path);
        match parsed {
            Ok(set) => {
                if first {
                    for texture in set.textures.iter().flatten() {
                        for scale in texture.scale {
                            counts.scales = Some(match counts.scales {
                                Some((low, high)) => (low.min(scale), high.max(scale)),
                                None => (scale, scale),
                            });
                        }
                        let file = normalize_name(texture.file.as_bytes());
                        counts
                            .textures
                            .record(vfs.source(&file).is_some(), || texture.file.to_string());
                    }
                }
                Some(set)
            }
            Err(error) => {
                if first {
                    failures.push(error.to_string());
                }
                None
            }
        }
    });
    if let Some(path) = environment_path(setting) {
        counts.environments.record(vfs.source(&path).is_some(), || {
            format!("{} for {shown}", path.escape_ascii())
        });
    }
    let [w, h] = setting.size;
    for y in 0..h {
        for x in 0..w {
            let present = has_area(vfs, folder, x, y);
            counts
                .holes
                .record(present, || format!("{x} {y} in {shown}"));
            if !present {
                continue;
            }
            let area = match read_area(vfs, folder, x, y) {
                Ok(area) => area,
                Err(error) => {
                    failures.push(error.to_string());
                    continue;
                }
            };
            counts.areas += 1;
            let area_path = area_folder(folder, x, y).unwrap_or_default();
            let file = |name: &[u8]| [area_path.as_slice(), name].concat();
            let attr = file(b"attr.atr");
            let has_attr = vfs.source(&attr).is_some();
            counts
                .attributes
                .record(has_attr, || attr.escape_ascii().to_string());
            if has_attr {
                match read_parsed(vfs, &attr, AttrMap::parse) {
                    Ok(map) => {
                        for flags in map.grid.values() {
                            *counts.attribute_flags.entry(*flags).or_default() += 1;
                        }
                    }
                    Err(error) => failures.push(error.to_string()),
                }
            }
            let minimap = file(b"minimap.dds");
            let has_minimap = vfs.source(&minimap).is_some();
            counts
                .minimaps
                .record(has_minimap, || minimap.escape_ascii().to_string());
            if has_minimap {
                let kind = vfs
                    .read(&minimap)
                    .map_err(|e| e.to_string())
                    .and_then(|bytes| {
                        let dds = Dds::parse(&bytes).map_err(|e| e.to_string())?;
                        Ok(format!(
                            "{}×{} {}",
                            dds.width,
                            dds.height,
                            dds.format.name()
                        ))
                    });
                match kind {
                    Ok(kind) => *counts.minimap_kinds.entry(kind).or_default() += 1,
                    Err(error) => failures.push(format!("{}: {error}", minimap.escape_ascii())),
                }
            }
            counts.water_layers += area.water.as_ref().map_or(0, |w| w.heights.len());
            if let Some(set) = &set {
                let mut used = [false; 256];
                for tile in area.tiles.grid.values() {
                    if let Some(slot) = used.get_mut(usize::from(*tile)) {
                        *slot = true;
                    }
                }
                let lacking: Vec<String> = (1..=255u8)
                    .filter(|&t| used.get(usize::from(t)) == Some(&true))
                    .filter(|&t| set.texture(t).is_none())
                    .map(|t| t.to_string())
                    .collect();
                counts.tiles_lacking.record(lacking.is_empty(), || {
                    format!(
                        "{} in {} with {} numbers",
                        lacking.join(" "),
                        area_path.escape_ascii(),
                        set.textures.len()
                    )
                });
            }
            for object in &area.objects.objects {
                let [rx, ry, _] = object.rotation;
                counts.tilted += usize::from(rx != 0.0 || ry != 0.0);
                if object.height_offset != 0.0 {
                    let (n, low, high) = counts.offsets;
                    counts.offsets = (
                        n + 1,
                        low.min(object.height_offset),
                        high.max(object.height_offset),
                    );
                }
                counts.longer.record(object.extra.is_empty(), || {
                    area_path.escape_ascii().to_string()
                });
                for row in &object.extra {
                    let values: Vec<String> = row.iter().map(|t| t.text.to_string()).collect();
                    *counts.extra_values.entry(values.join(" ")).or_default() += 1;
                }
                let found = properties.get(object.property);
                counts.objects.record(found.is_some(), || {
                    format!("{} in {}", object.property, area_path.escape_ascii())
                });
                let Some((_, property)) = found else {
                    continue;
                };
                *counts.kinds.entry(kind_name(property.kind())).or_default() += 1;
                if let Some(model) = property.model() {
                    let path = normalize_name(model.as_bytes());
                    if counts.models_seen.insert(path.clone()) {
                        let exists = vfs.source(&path).is_some();
                        counts
                            .models
                            .record(exists, || path.escape_ascii().to_string());
                        if exists {
                            let readable = vfs
                                .read(&path)
                                .ok()
                                .and_then(|bytes| Gr2File::read(&Gr2::parse(&bytes).ok()?).ok())
                                .is_some();
                            counts
                                .unreadable_models
                                .record(readable, || path.escape_ascii().to_string());
                        }
                        check_model_attributes(vfs, &path, counts, failures);
                    }
                }
            }
        }
    }
}

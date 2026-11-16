//! Script commands: `script dump|check`.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::Result;
use clap::Subcommand;
use m2_assets::actor::{NPC_LIST, monster_script, read_npc_list};
use m2_assets::effect::{find_effect_mesh, find_effect_texture, find_fly_effect};
use m2_assets::skills::{JOB_WORDS, skill_icon, skill_motion_paths};
use m2_assets::{Vfs, sub_image_path};
use m2_formats::dds::Dds;
use m2_formats::gr2::Gr2;
use m2_formats::gr2::model::Gr2File;
use m2_formats::mde::{Layout, MeshEffect};
use m2_formats::normalize_name;
use m2_formats::script::effect::{EffectData, Element, Moving};
use m2_formats::script::environment::{self, Environment};
use m2_formats::script::fly::FlyingData;
use m2_formats::script::item_list;
use m2_formats::script::motion::{self, EventKind, MotionData, MotionListEntry};
use m2_formats::script::race::{self, RaceData};
use m2_formats::script::skill_desc;
use m2_formats::script::sound::{self, SoundScript};
use m2_formats::script::sub::SubImage;
use m2_formats::script::{self, Body, Script, Token};
use m2_formats::tga::Tga;

use crate::files::{KeysArg, open_vfs, read_input};

/// How many failures `check` lists by name.
pub(crate) const SHOWN_PROBLEMS: usize = 20;

/// Extensions of the files written in the script syntax.
pub(crate) const SCRIPT_EXTENSIONS: [&str; 7] = ["msm", "msa", "mss", "mse", "msf", "msk", "msenv"];

/// `openm2 script …`
#[derive(Debug, Subcommand)]
pub enum ScriptCommand {
    /// Print a script (.msm, .msa, .mss, .mse, …) one value per line:
    /// `Group.Key value…`, list rows as `List[n] value…`
    Dump {
        /// A file, or with --client a game path
        path: String,
        /// Read `path` from this client folder's archives
        #[arg(long, value_name = "DIR")]
        client: Option<PathBuf>,
        #[command(flatten)]
        keys: KeysArg,
    },
    /// What openm2 reads from a motion script (.msa): the animation, when a
    /// combo takes the next attack, when and how far the motion hits, and
    /// its events
    Motion {
        /// A file, or with --client a game path
        path: String,
        /// Read `path` from this client folder's archives
        #[arg(long, value_name = "DIR")]
        client: Option<PathBuf>,
        #[command(flatten)]
        keys: KeysArg,
    },
    /// Parse every script, motion list and sub-image of a client, and check
    /// the files that model and motion scripts and sub-images name. Exit code
    /// 1 if a script cannot be read or parsed; scripts with values openm2
    /// cannot use are listed as unreadable.
    Check {
        /// The client folder (archives in its `pack` folder)
        #[arg(long, value_name = "DIR")]
        client: PathBuf,
        /// Also list every key path of each script type, with the number of
        /// files that have it
        #[arg(long)]
        census: bool,
        /// Also list the files that have this key path, written as --census
        /// prints it (`ShapeData.ShapeData##.SpecialPath`)
        #[arg(long, value_name = "PATH")]
        find: Option<String>,
        #[command(flatten)]
        keys: KeysArg,
    },
}

/// Runs a `script` command.
pub fn script(command: ScriptCommand) -> Result<ExitCode> {
    match command {
        ScriptCommand::Dump { path, client, keys } => {
            let bytes = read_input(&path, client.as_deref(), &keys)?;
            let parsed = script::parse(&bytes)?;
            let mut lines = Vec::new();
            dump(&parsed, "", &mut lines);
            for line in lines {
                println!("{line}");
            }
            Ok(ExitCode::SUCCESS)
        }
        ScriptCommand::Motion { path, client, keys } => {
            let bytes = read_input(&path, client.as_deref(), &keys)?;
            for line in motion_lines(&MotionData::parse(&bytes)?) {
                println!("{line}");
            }
            Ok(ExitCode::SUCCESS)
        }
        ScriptCommand::Check {
            client,
            census,
            find,
            keys,
        } => check(&client, census, find.as_deref(), &keys),
    }
}

/// A motion script as `script motion` prints it.
fn motion_lines(data: &MotionData) -> Vec<String> {
    let point = |p: [f32; 3]| format!("({:.1}, {:.1}, {:.1})", p[0], p[1], p[2]);
    let flat = |p: [f32; 3]| p[0].hypot(p[1]);
    let number = |n: Option<u32>| n.map_or_else(|| "-".to_owned(), |n| n.to_string());
    let mut lines = vec![format!("animation: {}", data.motion_file)];
    if let Some(duration) = data.duration {
        lines.push(format!("duration: {duration:.3} s"));
    }
    if let Some(moved) = data.accumulation {
        lines.push(format!(
            "moves: {:.1} cm along the ground {}",
            flat(moved),
            point(moved)
        ));
    }
    if let Some(combo) = data.combo {
        let link = combo
            .link
            .map_or_else(String::new, |link| format!(", link {link:.3} s"));
        lines.push(if combo.pre_input > combo.input_limit {
            format!("combo: takes no next attack{link}")
        } else {
            format!(
                "combo: a press from {:.3} s to {:.3} s asks for the next attack, which starts at {:.3} s at the earliest{link}",
                combo.pre_input, combo.input_limit, combo.direct_input
            )
        });
    }
    if let Some(attacking) = &data.attacking {
        lines.push(format!(
            "attack: AttackType {}, AttackingType {}, MotionType {}, HittingType {}, StiffenTime {:.3} s, InvisibleTime {:.3} s, ExternalForce {:.1}, HitLimitCount {}",
            number(attacking.attack_type),
            number(attacking.attacking_type),
            number(attacking.motion_type),
            attacking.hitting_type,
            attacking.stiffen_time,
            attacking.invisible_time,
            attacking.external_force,
            number(attacking.hit_limit),
        ));
        for (n, hit) in attacking.hits.iter().enumerate() {
            if !hit.strikes() {
                lines.push(format!("hit {n}: no time to strike in"));
                continue;
            }
            let reach = hit
                .positions
                .iter()
                .flat_map(|row| [flat(row.from), flat(row.to)])
                .reduce(f32::max);
            lines.push(format!(
                "hit {n}: {:.3} to {:.3} s, bone `{}`, weapon {:.1} cm, {} positions{}",
                hit.start,
                hit.end,
                hit.bone,
                hit.weapon_length,
                hit.positions.len(),
                reach.map_or_else(String::new, |r| format!(", reaching {r:.1} cm"))
            ));
        }
    }
    for event in &data.events {
        let what = match &event.kind {
            EventKind::Effect(effect) | EventKind::SpotEffect(effect) => format!(
                "{} {} at {}{}{}",
                if matches!(event.kind, EventKind::Effect(_)) {
                    "effect"
                } else {
                    "spot effect"
                },
                effect.file,
                point(effect.position),
                effect
                    .bone
                    .as_ref()
                    .map_or_else(String::new, |bone| format!(" on bone `{bone}`")),
                if effect.following { ", following" } else { "" }
            ),
            EventKind::Shake(shake) => format!(
                "shake: power {:.0}, range {:.0} cm, for {:.3} s",
                shake.power, shake.range, shake.during
            ),
            EventKind::Attack(attack) => format!(
                "attack for {:.3} s: {}",
                attack.during,
                attack
                    .spheres
                    .iter()
                    .map(|s| format!("sphere of {:.0} cm at {}", s.radius, point(s.position)))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            EventKind::Fly(fly) => format!(
                "fly {} at {}{}",
                fly.file,
                point(fly.position),
                fly.bone
                    .as_ref()
                    .map_or_else(String::new, |bone| format!(" from bone `{bone}`"))
            ),
            EventKind::Other(kind) => format!("type {kind}"),
        };
        lines.push(format!("event at {:.3} s: {what}", event.time));
    }
    let times = data.hit_times();
    if !times.is_empty() {
        lines.push(format!(
            "hits: {}; reach {:.1} cm",
            times
                .iter()
                .map(|(start, end)| format!("{start:.3} to {end:.3} s"))
                .collect::<Vec<_>>()
                .join(", "),
            data.reach().unwrap_or_default()
        ));
    }
    lines
}

/// One line per value: `Group.Key value…`, `Group.List[n] value…`.
fn dump(script: &Script, prefix: &str, lines: &mut Vec<String>) {
    for entry in &script.entries {
        let path = format!("{prefix}{}", entry.name);
        match &entry.body {
            Body::Values(values) => lines.push(join(&path, values)),
            Body::Group(group) => dump(group, &format!("{path}."), lines),
            Body::List(rows) => {
                for (n, row) in rows.iter().enumerate() {
                    lines.push(join(&format!("{path}[{n}]"), row));
                }
            }
        }
    }
}

fn join(path: &str, values: &[Token]) -> String {
    let mut line = path.to_owned();
    for token in values {
        line.push(' ');
        if token.quoted {
            line.push_str(&format!("\"{}\"", token.text));
        } else {
            line.push_str(&token.text.to_string());
        }
    }
    line
}

/// The key paths of a script, numbers in block names replaced by `#`
/// (`HairData.HairData##.Model`).
fn key_paths(script: &Script, prefix: &str, out: &mut BTreeSet<String>) {
    for entry in &script.entries {
        let name: String = entry
            .name
            .to_string()
            .chars()
            .map(|c| if c.is_ascii_digit() { '#' } else { c })
            .collect();
        let path = format!("{prefix}{name}");
        match &entry.body {
            Body::Values(_) => {
                out.insert(path);
            }
            Body::Group(group) => key_paths(group, &format!("{path}."), out),
            Body::List(_) => {
                out.insert(format!("{path}[]"));
            }
        }
    }
}

/// How many blocks deep the script nests.
fn depth(script: &Script) -> usize {
    script
        .entries
        .iter()
        .map(|entry| match &entry.body {
            Body::Group(group) => 1 + depth(group),
            Body::List(_) => 1,
            Body::Values(_) => 0,
        })
        .max()
        .unwrap_or(0)
}

/// True if any key or block name of the script matches.
fn any_name(script: &Script, matches: &dyn Fn(&[u8]) -> bool) -> bool {
    script.entries.iter().any(|entry| {
        matches(entry.name.as_bytes())
            || matches!(&entry.body, Body::Group(group) if any_name(group, matches))
    })
}

/// What the syntax pass found.
#[derive(Default)]
struct Syntax {
    /// Per extension: files, parsed.
    per_extension: BTreeMap<String, (usize, usize)>,
    /// Files per (extension, script type).
    types: BTreeMap<(String, String), usize>,
    /// Files per key path, per "extension script-type".
    paths: BTreeMap<String, BTreeMap<String, usize>>,
    /// Size and name of the largest script.
    largest: (usize, String),
    /// Depth and name of the most deeply nested script.
    deepest: (usize, String),
    /// Files that have the `--find` path.
    found: Vec<String>,
    /// Unusual things: count and first file, per kind.
    notes: BTreeMap<&'static str, (usize, String)>,
}

impl Syntax {
    fn note(&mut self, kind: &'static str, file: &str) {
        self.notes
            .entry(kind)
            .or_insert_with(|| (0, file.to_owned()))
            .0 += 1;
    }
}

/// References checked, how many failed, and the first failure.
#[derive(Default)]
pub(crate) struct Tally {
    pub(crate) total: usize,
    pub(crate) bad: usize,
    pub(crate) first: Option<String>,
}

impl Tally {
    pub(crate) fn record(&mut self, ok: bool, example: impl FnOnce() -> String) {
        self.total += 1;
        if !ok {
            self.bad += 1;
            if self.first.is_none() {
                self.first = Some(example());
            }
        }
    }

    /// `; N <what> (e.g. …)`, or nothing when none failed.
    pub(crate) fn problems(&self, what: &str) -> String {
        match &self.first {
            Some(first) => format!("; {} {what} (e.g. {first})", self.bad),
            None => String::new(),
        }
    }
}

/// The skies' colour bands and the pictures they name: the cloud textures,
/// and the box's pictures of those drawn with them.
fn check_skies(environments: &[(String, Environment)], vfs: &Vfs) {
    let mut named = 0;
    let mut missing = Vec::new();
    let mut uneven = Vec::new();
    for (shown, environment) in environments {
        let Some(sky) = &environment.sky else {
            continue;
        };
        let bands = (sky.upper_bands + sky.lower_bands) as usize;
        if sky.gradient.len() != 2 * bands {
            uneven.push(format!(
                "{shown}: {} gradient colours for {bands} bands",
                sky.gradient.len()
            ));
        }
        let faces = sky.faces.iter().filter(|_| sky.textured);
        for name in faces.chain([&sky.cloud_texture]) {
            if name.is_empty() {
                continue;
            }
            named += 1;
            let path = normalize_name(name.as_bytes());
            if vfs.source(&path).is_none() {
                missing.push(format!("{shown}: {}", path.escape_ascii()));
            }
        }
    }
    println!(
        "  skies: {named} pictures named (clouds and the boxes' faces), {} missing; {} with as many gradient colours as two per band",
        missing.len(),
        environments.iter().filter(|(_, e)| e.sky.is_some()).count() - uneven.len()
    );
    for line in missing.iter().chain(&uneven).take(10) {
        println!("    {line}");
    }
}

fn check(client: &Path, census: bool, find: Option<&str>, keys: &KeysArg) -> Result<ExitCode> {
    let vfs = open_vfs(client, &keys.load()?)?;
    let mut failures = Vec::new();
    let mut unreadable = Vec::new();
    let mut syntax = Syntax::default();
    let mut races = Vec::new();
    let mut motions = Vec::new();
    // Rows of every `List HitPosition` in the motion scripts read.
    let mut hit_rows = 0;
    let mut lists = Vec::new();
    let mut environments = Vec::new();
    let mut subs = Vec::new();
    let mut effects = Vec::new();
    let mut flights = Vec::new();
    let mut sounds = Vec::new();
    let names: Vec<Vec<u8>> = vfs.files().map(|(name, _, _)| name.to_vec()).collect();
    for name in &names {
        let shown = name.escape_ascii().to_string();
        let extension = shown.rsplit_once('.').map_or("", |(_, e)| e).to_owned();
        let is_script = SCRIPT_EXTENSIONS.contains(&extension.as_str());
        let is_list = extension == "txt" && file_name(name).starts_with(b"motlist");
        if extension == "sub" {
            match vfs.read(name).map_err(|e| e.to_string()).and_then(|bytes| {
                parse_counted(&bytes, &shown, &extension, find, &mut syntax)
                    .and_then(|parsed| SubImage::read(&parsed))
                    .map_err(|e| e.to_string())
            }) {
                Ok(sub) => subs.push((name.clone(), shown, sub)),
                Err(error) => failures.push(format!("{shown}: {error}")),
            }
            continue;
        }
        if !is_script && extension != "txt" {
            continue;
        }
        let bytes = match vfs.read(name) {
            Ok(bytes) => bytes,
            Err(error) => {
                failures.push(format!("{shown}: {error}"));
                continue;
            }
        };
        if is_list {
            match motion::parse_list(&bytes) {
                Ok(list) => lists.push((name.clone(), shown, list)),
                Err(error) => failures.push(format!("{shown}: {error}")),
            }
            continue;
        }
        if !is_script && !bytes.trim_ascii_start().starts_with(b"ScriptType") {
            continue;
        }
        let parsed = match parse_counted(&bytes, &shown, &extension, find, &mut syntax) {
            Ok(parsed) => parsed,
            Err(error) => {
                failures.push(format!("{shown}: {error}"));
                continue;
            }
        };
        let is_type = |kind: &str| {
            parsed
                .script_type()
                .is_some_and(|t| t.as_bytes().eq_ignore_ascii_case(kind.as_bytes()))
        };
        if is_type(race::SCRIPT_TYPE) {
            match RaceData::read(&parsed) {
                Ok(data) => races.push((name.clone(), shown, data)),
                Err(error) => unreadable.push(format!("{shown}: {error}")),
            }
        } else if is_type(motion::SCRIPT_TYPE) {
            match MotionData::read(&parsed) {
                Ok(data) => {
                    hit_rows += list_rows(&parsed, "HitPosition");
                    motions.push((shown, data));
                }
                Err(error) => unreadable.push(format!("{shown}: {error}")),
            }
        } else if is_type(environment::SCRIPT_TYPE) {
            match Environment::read(&parsed) {
                Ok(data) => environments.push((shown, data)),
                Err(error) => unreadable.push(format!("{shown}: {error}")),
            }
        } else if extension == "mse" {
            match EffectData::read(&parsed) {
                Ok(data) => effects.push((name.clone(), shown, data)),
                Err(error) => unreadable.push(format!("{shown}: {error}")),
            }
        } else if extension == "msf" {
            match FlyingData::read(&parsed) {
                Ok(data) => flights.push((name.clone(), data)),
                Err(error) => unreadable.push(format!("{shown}: {error}")),
            }
        } else if is_type(sound::SCRIPT_TYPE) {
            match SoundScript::read(&parsed) {
                Ok(data) => sounds.push((name.clone(), shown, data)),
                Err(error) => unreadable.push(format!("{shown}: {error}")),
            }
        }
    }

    print_syntax(&syntax);
    let mut models = Models::new(&vfs);
    check_races(&races, &mut models);
    check_motions(&motions, hit_rows, &mut models);
    check_lists(&lists, &vfs);
    check_npc_list(&vfs, &mut failures);
    check_item_lists(&vfs, &names, &mut failures);
    check_skill_descriptions(&vfs, &names, &mut failures);
    check_subs(&subs, &vfs);
    check_effects(&effects, &names, &vfs);
    check_flights(&flights, &vfs);
    check_sounds(&sounds, &vfs);
    let count =
        |has: &dyn Fn(&Environment) -> bool| environments.iter().filter(|(_, e)| has(e)).count();
    println!(
        "environment data: {} scripts; {} with a light direction, {} with background light, {} with a material, {} with fog ({} enabled), {} with a sky ({} drawn with pictures)",
        environments.len(),
        count(&|e| e.light_direction.is_some()),
        count(&|e| e.background.is_some()),
        count(&|e| e.material.is_some()),
        count(&|e| e.fog.is_some()),
        count(&|e| e.fog.is_some_and(|f| f.enabled)),
        count(&|e| e.sky.is_some()),
        count(&|e| e.sky.as_ref().is_some_and(|s| s.textured)),
    );
    check_skies(&environments, &vfs);
    if census {
        for (kind, counted) in &syntax.paths {
            println!("keys of {kind}:");
            for (path, files) in counted {
                println!("  {path} {files}");
            }
        }
    }
    if let Some(path) = find {
        println!("{} files have {path}:", syntax.found.len());
        for file in &syntax.found {
            println!("  {file}");
        }
    }
    for line in unreadable.iter().take(SHOWN_PROBLEMS) {
        println!("unreadable {line}");
    }
    if unreadable.len() > SHOWN_PROBLEMS {
        println!(
            "… and {} more unreadable",
            unreadable.len() - SHOWN_PROBLEMS
        );
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

/// Parses one script and counts what the syntax pass reports.
fn parse_counted(
    bytes: &[u8],
    shown: &str,
    extension: &str,
    find: Option<&str>,
    syntax: &mut Syntax,
) -> Result<Script, m2_formats::FormatError> {
    let counts = syntax
        .per_extension
        .entry(extension.to_owned())
        .or_default();
    counts.0 += 1;
    let parsed = script::parse(bytes);
    if parsed.is_ok() {
        counts.1 += 1;
    }
    if bytes.len() > syntax.largest.0 {
        syntax.largest = (bytes.len(), shown.to_owned());
    }
    if !bytes.contains(&b'\r') {
        syntax.note("line ends without \\r", shown);
    }
    if bytes.contains(&0) {
        syntax.note("NUL bytes", shown);
    }
    let parsed = parsed?;
    let depth = depth(&parsed);
    if depth > syntax.deepest.0 {
        syntax.deepest = (depth, shown.to_owned());
    }
    let script_type = parsed
        .script_type()
        .map_or_else(|| "(none)".to_owned(), ToString::to_string);
    *syntax
        .types
        .entry((extension.to_owned(), script_type.clone()))
        .or_default() += 1;
    if any_name(&parsed, &|name| {
        name.starts_with(b"//") || name.starts_with(b"#") || name.starts_with(b";")
    }) {
        syntax.note("keys that look like comments", shown);
    }
    let mut file_paths = BTreeSet::new();
    key_paths(&parsed, "", &mut file_paths);
    if find.is_some_and(|f| file_paths.contains(f)) {
        syntax.found.push(shown.to_owned());
    }
    let counted = syntax
        .paths
        .entry(format!("{extension} {script_type}"))
        .or_default();
    for path in file_paths {
        *counted.entry(path).or_default() += 1;
    }
    Ok(parsed)
}

fn print_syntax(syntax: &Syntax) {
    let total: usize = syntax.per_extension.values().map(|c| c.0).sum();
    let parsed: usize = syntax.per_extension.values().map(|c| c.1).sum();
    println!("{total} scripts: parsed {parsed}");
    for (extension, (files, ok)) in &syntax.per_extension {
        println!("  .{extension}: {files} files, parsed {ok}");
    }
    println!(
        "  script types: {}",
        syntax
            .types
            .iter()
            .map(|((e, t), n)| format!("{t} (.{e}) {n}"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    println!(
        "  largest: {} bytes ({})",
        syntax.largest.0, syntax.largest.1
    );
    println!(
        "  deepest: blocks {} deep ({})",
        syntax.deepest.0, syntax.deepest.1
    );
    for (kind, (files, first)) in &syntax.notes {
        println!("  note: {kind} in {files} files (e.g. {first})");
    }
}

/// What `check` needs from a GR2 file: its texture file names (lowercase)
/// and its first animation's length.
type ModelFacts = (Vec<Vec<u8>>, Option<f32>);

/// GR2 files, each read once.
struct Models<'a> {
    vfs: &'a Vfs,
    read: HashMap<Vec<u8>, Option<ModelFacts>>,
}

impl<'a> Models<'a> {
    fn new(vfs: &'a Vfs) -> Self {
        Self {
            vfs,
            read: HashMap::new(),
        }
    }

    fn exists(&self, path: &[u8]) -> bool {
        self.vfs.source(path).is_some()
    }

    /// The facts of the GR2 file at `path`, or `None` if it cannot be read.
    fn get(&mut self, path: &[u8]) -> Option<&ModelFacts> {
        let vfs = self.vfs;
        self.read
            .entry(normalize_name(path))
            .or_insert_with_key(|key| {
                let bytes = vfs.read(key).ok()?;
                let file = Gr2File::read(&Gr2::parse(&bytes).ok()?).ok()?;
                let textures = file
                    .textures
                    .iter()
                    .map(|t| file_name(t.file_name.as_bytes()))
                    .collect();
                Some((textures, file.animations.first().map(|a| a.duration)))
            })
            .as_ref()
    }
}

/// The file name of a path, lowercase.
fn file_name(path: &[u8]) -> Vec<u8> {
    let start = path
        .iter()
        .rposition(|b| matches!(b, b'/' | b'\\'))
        .map_or(0, |at| at + 1);
    path.get(start..).unwrap_or_default().to_ascii_lowercase()
}

/// The folder part of a game path, with its trailing `/`.
fn folder(path: &[u8]) -> &[u8] {
    let end = path
        .iter()
        .rposition(|b| matches!(b, b'/' | b'\\'))
        .map_or(0, |at| at + 1);
    path.get(..end).unwrap_or_default()
}

fn check_races(races: &[(Vec<u8>, String, RaceData)], models: &mut Models<'_>) {
    let mut base = Tally::default();
    let mut parts = Tally::default();
    let mut sources = Tally::default();
    let mut targets = Tally::default();
    let mut lists = Tally::default();
    let mut repeated = Tally::default();
    let (mut shapes, mut hairs) = (0, 0);
    // Collision blocks: scripts with any, blocks, spheres, those on a
    // bone, and blocks by `CollisionType`.
    let (mut colliding, mut blocks, mut spheres, mut attached) = (0, 0, 0, 0);
    let mut collision_types = BTreeMap::new();
    // Attached effects and metin stones' smoke, and whether their files are
    // in the client.
    let (mut with_effects, mut smoking, mut stages) = (0, 0, 0);
    let mut effect_files = Tally::default();
    for (name, shown, race) in races {
        with_effects += usize::from(!race.effects.is_empty());
        let smoke = race
            .smoke
            .iter()
            .flat_map(|s| s.stages.iter().map(|(_, f)| f));
        smoking += usize::from(race.smoke.is_some());
        stages += race.smoke.as_ref().map_or(0, |s| s.stages.len());
        for file in race.effects.iter().map(|e| &e.file).chain(smoke) {
            effect_files.record(models.exists(&normalize_name(file.as_bytes())), || {
                format!("{file} in {shown}")
            });
        }
        colliding += usize::from(!race.collisions.is_empty());
        for collision in &race.collisions {
            blocks += 1;
            spheres += collision.spheres.len();
            attached += usize::from(collision.attached);
            *collision_types.entry(collision.collision_type).or_insert(0) += 1;
        }
        if let Some(model) = &race.base_model {
            base.record(models.exists(model.as_bytes()), || {
                format!("{model} in {shown}")
            });
        }
        if let Some(list) = &race.motion_list {
            let path = [folder(name), list.as_bytes()].concat();
            lists.record(models.exists(&path), || format!("{list} in {shown}"));
        }
        shapes += race.shapes.entries.len();
        hairs += race.hairs.entries.len();
        for table in [&race.shapes, &race.hairs] {
            let mut seen = BTreeSet::new();
            for entry in &table.entries {
                repeated.record(seen.insert(entry.index), || {
                    format!("index {} in {shown}", entry.index)
                });
                let model = table.resolve(entry, &entry.model);
                let exists = models.exists(&model);
                parts.record(exists, || format!("{} in {shown}", model.escape_ascii()));
                let textures = match exists {
                    true => models.get(&model).map(|(t, _)| t.clone()),
                    false => None,
                };
                for skin in &entry.skins {
                    if let Some(textures) = &textures {
                        let source = file_name(skin.source.as_bytes());
                        sources.record(textures.contains(&source), || {
                            format!("{} for {} in {shown}", skin.source, model.escape_ascii())
                        });
                    }
                    if let Some(target) = &skin.target {
                        let path = table.resolve(entry, target);
                        targets.record(models.exists(&path), || {
                            format!("{} in {shown}", path.escape_ascii())
                        });
                    }
                }
            }
        }
    }
    println!(
        "race data: {} scripts, {} base models{}",
        races.len(),
        base.total,
        base.problems("missing")
    );
    println!(
        "  {shapes} shapes and {hairs} hair styles: {} models{}",
        parts.total,
        parts.problems("missing")
    );
    if repeated.bad > 0 {
        println!(
            "  indexes listed twice in one block: {}{}",
            repeated.bad,
            repeated.problems("(the first one counts)")
        );
    }
    println!(
        "  skins: {} sources looked for among their model's textures{}",
        sources.total,
        sources.problems("not found")
    );
    println!(
        "  skins: {} targets{}",
        targets.total,
        targets.problems("missing")
    );
    let types: Vec<String> = collision_types
        .iter()
        .map(|(kind, count)| format!("{kind}: {count}"))
        .collect();
    println!(
        "  collision spheres: {spheres} in {blocks} blocks of {colliding} scripts, {attached} blocks on a bone; CollisionType {}",
        types.join(", ")
    );
    println!(
        "  effects: attached in {with_effects} scripts, smoke in {smoking} ({stages} stages); {} effect files named{}",
        effect_files.total,
        effect_files.problems("missing")
    );
    if lists.total > 0 {
        println!(
            "  motion lists named: {}{}",
            lists.total,
            lists.problems("missing")
        );
    }
}

fn check_motions(motions: &[(String, MotionData)], hit_rows: usize, models: &mut Models<'_>) {
    let mut files = Tally::default();
    let mut lengths = Tally::default();
    let mut largest = (0.0f32, String::new());
    for (shown, motion) in motions {
        let path = motion.motion_file.as_bytes();
        let exists = models.exists(path);
        files.record(exists, || format!("{} in {shown}", motion.motion_file));
        let (Some(duration), true) = (motion.duration, exists) else {
            continue;
        };
        let Some(length) = models.get(path).and_then(|(_, length)| *length) else {
            continue;
        };
        let difference = (duration - length).abs();
        let example = || format!("{shown}: {duration} s, the animation {length} s");
        if difference > largest.0 {
            largest = (difference, example());
        }
        lengths.record(difference <= 0.001, example);
    }
    println!(
        "motion data: {} scripts, {} motion files{}",
        motions.len(),
        files.total,
        files.problems("missing")
    );
    println!(
        "  MotionDuration against the animation's length: {} compared{}",
        lengths.total,
        lengths.problems("differ by more than 1 ms")
    );
    if largest.0 > 0.001 {
        println!("  largest difference: {:.3} s ({})", largest.0, largest.1);
    }
    check_attacks(motions, hit_rows, models);
}

/// Rows of the lists called `name`, at any depth.
fn list_rows(script: &Script, name: &str) -> usize {
    script
        .entries
        .iter()
        .map(|entry| match &entry.body {
            Body::List(rows) if entry.name.as_bytes().eq_ignore_ascii_case(name.as_bytes()) => {
                rows.len()
            }
            Body::Group(group) => list_rows(group, name),
            Body::List(_) | Body::Values(_) => 0,
        })
        .sum()
}

/// What the flying effects hold, and whether the effects they name are in
/// the client.
fn check_flights(flights: &[(Vec<u8>, FlyingData)], vfs: &Vfs) {
    let attached: usize = flights.iter().map(|(_, f)| f.attached.len()).sum();
    let mut types: BTreeMap<u32, usize> = BTreeMap::new();
    let mut tails = 0;
    for attach in flights.iter().flat_map(|(_, f)| &f.attached) {
        *types.entry(attach.fly_type).or_default() += 1;
        tails += usize::from(attach.tail.is_some());
    }
    let count = |has: &dyn Fn(&FlyingData) -> bool| flights.iter().filter(|(_, f)| has(f)).count();
    let mut files = Tally::default();
    for (path, flight) in flights {
        for file in flight.effects() {
            let found = find_fly_effect(vfs, path, file.as_bytes()).is_some();
            files.record(found, || format!("{file} in {}", path.escape_ascii()));
        }
    }
    println!(
        "flying effects: {} scripts, {attached} attached effects (fly types {}), {tails} with a tail; {} homing, {} with a bomb effect, {} spreading",
        flights.len(),
        types
            .iter()
            .map(|(kind, n)| format!("{kind}: {n}"))
            .collect::<Vec<_>>()
            .join(", "),
        count(&|f| f.homing),
        count(&|f| f.bomb_effect.is_some()),
        count(&|f| f.spreading),
    );
    println!(
        "  effects they name: {}{}",
        files.total,
        files.problems("missing")
    );
}

/// The combo, attack and event data of the motion scripts: how much there
/// is, whether the hit positions are a weapon's length apart, and whether
/// the effect and flying files the events name are in the client.
fn check_attacks(motions: &[(String, MotionData)], hit_rows: usize, models: &mut Models<'_>) {
    let mut attacking = (0, 0);
    let (mut hits, mut striking, mut rows) = (0, 0, 0);
    let mut lengths = Tally::default();
    let (mut combos, mut closed) = (0, 0);
    let mut kinds: BTreeMap<&str, usize> = BTreeMap::new();
    let mut spheres = 0;
    let mut most = (0, String::new());
    let mut files = Tally::default();
    for (shown, motion) in motions {
        if let Some(combo) = &motion.combo {
            combos += 1;
            if combo.pre_input > combo.input_limit {
                closed += 1;
            }
        }
        let mut blocks = motion.events.len();
        if let Some(data) = &motion.attacking {
            attacking.0 += 1;
            if data.attacking_type.is_some() {
                attacking.1 += 1;
            }
            blocks = blocks.max(data.hits.len());
            for hit in &data.hits {
                hits += 1;
                striking += usize::from(hit.strikes());
                for row in &hit.positions {
                    rows += 1;
                    let apart = (0..3)
                        .map(|i| row.to[i] - row.from[i])
                        .map(|d| d * d)
                        .sum::<f32>()
                        .sqrt();
                    lengths.record((apart - hit.weapon_length).abs() <= 1.0, || {
                        format!(
                            "{apart:.1} cm for WeaponLength {} in {shown}",
                            hit.weapon_length
                        )
                    });
                }
            }
        }
        for event in &motion.events {
            let (kind, file) = match &event.kind {
                EventKind::Effect(effect) => ("effect", Some(&effect.file)),
                EventKind::Shake(_) => ("shake", None),
                EventKind::Attack(attack) => {
                    spheres += attack.spheres.len();
                    blocks = blocks.max(attack.spheres.len());
                    ("attack", None)
                }
                EventKind::Fly(fly) => ("fly", Some(&fly.file)),
                EventKind::SpotEffect(effect) => ("spot effect", Some(&effect.file)),
                EventKind::Other(_) => ("other", None),
            };
            *kinds.entry(kind).or_default() += 1;
            if let Some(file) = file.filter(|f| !f.is_empty()) {
                files.record(models.exists(file.as_bytes()), || {
                    format!("{file} in {shown}")
                });
            }
        }
        if blocks > most.0 {
            most = (blocks, shown.clone());
        }
    }
    println!(
        "  attack data: {} scripts ({} in the older layout), {hits} hits ({striking} strike), {rows} hit positions ({} left out: not seven numbers)",
        attacking.0,
        attacking.1,
        hit_rows.saturating_sub(rows)
    );
    println!(
        "  hit positions whose two points are WeaponLength apart (1 cm): {}{}",
        lengths.total,
        lengths.problems("are not")
    );
    println!("  combo input: {combos} scripts, {closed} with an empty window");
    println!(
        "  events: {} ({}), {spheres} attack spheres",
        kinds.values().sum::<usize>(),
        kinds
            .iter()
            .map(|(kind, n)| format!("{n} {kind}"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    println!(
        "  effect and flying files named by events: {}{}",
        files.total,
        files.problems("missing")
    );
    if most.0 > 0 {
        println!(
            "  most hits, events or spheres in one block: {} ({})",
            most.0, most.1
        );
    }
}

/// The monster list: how many monsters it names and how many of them lead
/// to a model script in the client.
fn check_npc_list(vfs: &Vfs, failures: &mut Vec<String>) {
    if vfs.source(NPC_LIST).is_none() {
        println!("monster list: none");
        return;
    }
    let lines = match read_npc_list(vfs) {
        Ok(lines) => lines,
        Err(error) => {
            failures.push(error.to_string());
            return;
        }
    };
    let vnums: BTreeSet<u32> = lines.iter().map(|l| l.vnum).filter(|v| *v != 0).collect();
    let mut scripts = Tally::default();
    for vnum in &vnums {
        scripts.record(monster_script(vfs, &lines, *vnum).is_some(), || {
            vnum.to_string()
        });
    }
    let folders = lines.iter().filter(|l| l.vnum == 0).count();
    let paths = lines
        .iter()
        .filter(|l| l.vnum != 0 && l.name.as_bytes().starts_with(b"#"))
        .count();
    println!(
        "monster list: {} lines ({} naming a folder, {} a folder path), {} monsters{}",
        lines.len(),
        folders,
        paths,
        vnums.len(),
        scripts.problems("without a model script in the client")
    );
}

/// What the item lists of one group of folders hold.
#[derive(Default)]
struct ItemLists {
    files: usize,
    lines: usize,
    /// Numbers listed again in their own file.
    twice: usize,
    models: usize,
    icons: Tally,
    found_models: Tally,
}

/// The item lists (`item_list.txt`): those of the locales, which openm2
/// reads, and the others. Their lines, numbers listed twice, and whether the
/// icons and ground models they name are in the client.
fn check_item_lists(vfs: &Vfs, names: &[Vec<u8>], failures: &mut Vec<String>) {
    let mut groups: BTreeMap<&str, ItemLists> = BTreeMap::new();
    for name in names
        .iter()
        .filter(|name| file_name(name).ends_with(b"item_list.txt"))
    {
        let shown = name.escape_ascii().to_string();
        let parsed = match vfs.read(name) {
            Ok(bytes) => item_list::parse(&bytes).map_err(|e| e.to_string()),
            Err(error) => Err(error.to_string()),
        };
        let list = match parsed {
            Ok(list) => list,
            Err(error) => {
                failures.push(format!("{shown}: {error}"));
                continue;
            }
        };
        let group = if name.starts_with(b"locale/") {
            "locale"
        } else {
            "other"
        };
        let tally = groups.entry(group).or_default();
        tally.files += 1;
        tally.lines += list.len();
        let mut seen = BTreeSet::new();
        for line in &list {
            if !seen.insert(line.vnum) {
                tally.twice += 1;
            }
            let icon = normalize_name(line.icon.as_bytes());
            tally.icons.record(vfs.source(&icon).is_some(), || {
                format!("{} in {shown}", line.icon)
            });
            if let Some(model) = &line.model {
                tally.models += 1;
                let path = normalize_name(model.as_bytes());
                tally.found_models.record(vfs.source(&path).is_some(), || {
                    format!("{model} in {shown}")
                });
            }
        }
    }
    if groups.is_empty() {
        println!("item lists: none");
    }
    for (group, tally) in &groups {
        println!(
            "item lists ({group}): {} files, {} lines ({} numbers listed again in their file), {} with a ground model{}{}",
            tally.files,
            tally.lines,
            tally.twice,
            tally.models,
            tally.icons.problems("icons not in the client"),
            tally
                .found_models
                .problems("ground models not in the client"),
        );
    }
}

/// The locales' skill descriptions (`skilldesc.txt`): their lines, the
/// skills a job uses, and whether those skills' icons (by grade) and
/// motions (by grade, for both genders' folders) are in the client.
fn check_skill_descriptions(vfs: &Vfs, names: &[Vec<u8>], failures: &mut Vec<String>) {
    let (mut files, mut lines, mut used) = (0, 0, 0);
    let (mut icons, mut motions, mut own) = (Tally::default(), Tally::default(), Tally::default());
    for name in names
        .iter()
        .filter(|name| name.starts_with(b"locale/") && file_name(name) == b"skilldesc.txt")
    {
        let shown = name.escape_ascii().to_string();
        let parsed = match vfs.read(name) {
            Ok(bytes) => skill_desc::parse(&bytes).map_err(|e| e.to_string()),
            Err(error) => Err(error.to_string()),
        };
        let list = match parsed {
            Ok(list) => list,
            Err(error) => {
                failures.push(format!("{shown}: {error}"));
                continue;
            }
        };
        files += 1;
        lines += list.len();
        for line in &list {
            let Some(job) = JOB_WORDS
                .iter()
                .position(|word| *word == line.job.as_bytes())
            else {
                continue;
            };
            if line.motion.is_none() || line.icon.is_empty() {
                continue;
            }
            used += 1;
            let folder = String::from_utf8_lossy(JOB_WORDS[job]).to_ascii_lowercase();
            for grade in 0..4 {
                let icon = skill_icon(line, grade).unwrap_or_default();
                icons.record(vfs.source(&icon).is_some(), || {
                    format!("{} in {shown}", icon.escape_ascii())
                });
                for pc in ["pc", "pc2"] {
                    let model = format!("d:/ymir work/{pc}/{folder}/x.gr2");
                    let paths = skill_motion_paths(model.as_bytes(), line.icon.as_bytes(), grade);
                    let found = paths.iter().position(|path| vfs.source(path).is_some());
                    motions.record(found.is_some(), || {
                        format!("skill {} of {pc} in {shown}", line.vnum)
                    });
                    own.record(found == Some(0), String::new);
                }
            }
        }
    }
    if files == 0 {
        println!("skill descriptions: none");
        return;
    }
    println!(
        "skill descriptions: {files} files, {lines} lines, {used} skills used by a job; {} grades' motions (both genders), {} played with another grade's or `_me`{}{}",
        motions.total,
        own.bad,
        icons.problems("icons not in the client"),
        motions.problems("skills without a motion"),
    );
}

fn check_lists(lists: &[(Vec<u8>, String, Vec<MotionListEntry>)], vfs: &Vfs) {
    let mut scripts = Tally::default();
    let mut names: BTreeMap<String, usize> = BTreeMap::new();
    let mut modes: BTreeMap<String, usize> = BTreeMap::new();
    let mut weights = Tally::default();
    for (name, shown, list) in lists {
        // Weights per motion, its numbered variants counted together
        // (`WAIT`, `WAIT1`, `WAIT2`).
        let mut sums: BTreeMap<String, u32> = BTreeMap::new();
        for entry in list {
            let motion = entry.name.to_string();
            let motion = motion.trim_end_matches(|c: char| c.is_ascii_digit());
            *sums.entry(motion.to_owned()).or_default() += entry.weight;
        }
        for (motion, sum) in &sums {
            weights.record(*sum == 100, || format!("{motion} {sum} in {shown}"));
        }
        for entry in list {
            let path = entry.script_path(name);
            scripts.record(vfs.source(&path).is_some(), || {
                format!("{} in {shown}", entry.file)
            });
            *names.entry(entry.name.to_string()).or_default() += 1;
            *modes.entry(entry.mode.to_string()).or_default() += 1;
        }
    }
    let join = |counts: &BTreeMap<String, usize>| {
        counts
            .iter()
            .map(|(k, n)| format!("{k} {n}"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    println!(
        "motion lists: {} files, {} motions{}",
        lists.len(),
        scripts.total,
        scripts.problems("missing")
    );
    println!(
        "  weights of a motion and its numbered variants: {} motions{}",
        weights.total,
        weights.problems("do not add up to 100")
    );
    println!("  modes: {}", join(&modes));
    println!("  names: {}", join(&names));
}

/// Sub-images: whether the image they cut from is in the client
/// (`m2_assets::sub_image_path`), and where their rectangles end compared
/// with the image's size.
fn check_subs(subs: &[(Vec<u8>, String, SubImage)], vfs: &Vfs) {
    let mut sizes: HashMap<Vec<u8>, Option<(u32, u32)>> = HashMap::new();
    let mut versions: BTreeMap<String, Tally> = BTreeMap::new();
    let (mut at_edge, mut inside, mut beyond) = (0usize, 0usize, 0usize);
    let mut first_beyond = None;
    for (name, shown, sub) in subs {
        let image = sub_image_path(name, sub);
        let found = vfs.source(&image).is_some();
        versions
            .entry(sub.version.to_string())
            .or_default()
            .record(found, || format!("{shown}: {}", image.escape_ascii()));
        if !found {
            continue;
        }
        let size = *sizes.entry(image.clone()).or_insert_with(|| {
            let bytes = vfs.read(&image).ok()?;
            if image.ends_with(b".tga") {
                Tga::parse(&bytes)
                    .ok()
                    .map(|t| (u32::from(t.width), u32::from(t.height)))
            } else if image.ends_with(b".dds") {
                Dds::parse(&bytes).ok().map(|d| (d.width, d.height))
            } else {
                None
            }
        });
        let Some((width, height)) = size else {
            continue;
        };
        if sub.right > width || sub.bottom > height {
            beyond += 1;
            first_beyond.get_or_insert_with(|| {
                format!(
                    "{shown}: right {}, bottom {} in {width}×{height}",
                    sub.right, sub.bottom
                )
            });
        } else if sub.right == width || sub.bottom == height {
            at_edge += 1;
        } else {
            inside += 1;
        }
    }
    println!("sub-images: {} files", subs.len());
    for (version, tally) in &versions {
        println!(
            "  version {version}: {}{}",
            tally.total,
            tally.problems("cut from an image the client lacks")
        );
    }
    println!(
        "  rectangles (DDS and TGA images): {inside} inside the image, {at_edge} ending on its right or bottom edge, {beyond} past it{}",
        first_beyond.map_or_else(String::new, |e| format!(" (e.g. {e})"))
    );
}

/// Counts per value, printed as `0 12, 1 5`.
fn counts<K: Ord + std::fmt::Debug>(counted: &BTreeMap<K, usize>) -> String {
    counted
        .iter()
        .map(|(value, n)| format!("{value:?} {n}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// What the sound scripts name, and what they belong to: the motion
/// (`.msa`) or effect (`.mse`) at the same path under `d:/ymir work/`.
fn check_sounds(scripts: &[(Vec<u8>, String, SoundScript)], vfs: &Vfs) {
    let (mut sounds, mut most, mut latest) = (0usize, 0usize, 0.0f32);
    let (mut motions, mut effects) = (0usize, 0usize);
    let mut files = Tally::default();
    let mut kinds: BTreeMap<String, usize> = BTreeMap::new();
    for (name, shown, script) in scripts {
        sounds += script.sounds.len();
        most = most.max(script.sounds.len());
        let owner = name
            .strip_prefix(b"sound/")
            .and_then(|rest| rest.strip_suffix(b".mss"))
            .map(|stem| [b"d:/ymir work/".as_slice(), stem].concat());
        let has = |extension: &[u8]| {
            owner.as_ref().is_some_and(|owner| {
                vfs.source(&[owner.as_slice(), extension].concat())
                    .is_some()
            })
        };
        if has(b".msa") {
            motions += 1;
        } else if has(b".mse") {
            effects += 1;
        }
        for sound in &script.sounds {
            latest = latest.max(sound.time);
            let file = normalize_name(sound.file.as_bytes());
            let extension = file
                .rsplit(|b| *b == b'.')
                .next()
                .map(|e| String::from_utf8_lossy(e).into_owned())
                .unwrap_or_default();
            *kinds.entry(extension).or_insert(0) += 1;
            files.record(vfs.source(&file).is_some(), || {
                format!("{} in {shown}", sound.file)
            });
        }
    }
    println!(
        "sound scripts: {} scripts, {sounds} sounds (at most {most} in one, the latest {latest:.3} s in); {motions} beside a motion script, {effects} beside an effect script",
        scripts.len()
    );
    println!(
        "  sound files named: {} ({}){}",
        files.total,
        counts(&kinds),
        files.problems("missing")
    );
}

/// Effect scripts: what their particles and meshes are made of, and whether
/// the textures and mesh files they name are in the client; then every mesh
/// effect file of the client.
fn check_effects(effects: &[(Vec<u8>, String, EffectData)], names: &[Vec<u8>], vfs: &Vfs) {
    let (mut particles, mut meshes, mut bounded) = (0usize, 0usize, 0usize);
    let mut shapes = BTreeMap::new();
    let mut advanced = BTreeMap::new();
    let mut billboards = BTreeMap::new();
    let mut rotations = BTreeMap::new();
    let mut animations = BTreeMap::new();
    let mut blends = BTreeMap::new();
    let mut operations = BTreeMap::new();
    let mut mesh_billboards = BTreeMap::new();
    let mut mesh_blends = BTreeMap::new();
    let (mut from_edge, mut attached, mut stretched) = (0usize, 0usize, 0usize);
    let (mut emitter_keys, mut past_cycle) = (0usize, 0usize);
    let (mut particle_keys, mut past_life) = (0usize, 0usize);
    let (mut direct, mut bezier) = (0usize, 0usize);
    let mut textures = Tally::default();
    let mut mesh_files = Tally::default();
    let mut mesh_unreadable = Tally::default();
    let mut element_counts = Tally::default();
    let mut mesh_textures = Tally::default();
    let mut read: HashMap<Vec<u8>, Option<MeshEffect>> = HashMap::new();
    for (name, shown, effect) in effects {
        bounded += usize::from(effect.bounding_radius.is_some());
        for element in &effect.elements {
            let keys = match element {
                Element::Particle(p) => &p.positions,
                Element::Mesh(m) => &m.positions,
            };
            for key in keys {
                match key.moving {
                    Moving::Direct => direct += 1,
                    Moving::Bezier(_) => bezier += 1,
                }
            }
            match element {
                Element::Particle(p) => {
                    particles += 1;
                    let (e, look) = (&p.emitter, &p.particle);
                    *shapes.entry(e.shape).or_insert(0) += 1;
                    *advanced.entry(e.advanced_type).or_insert(0) += 1;
                    from_edge += usize::from(e.from_edge);
                    for curve in [
                        &e.emitting_size,
                        &e.angular_velocity,
                        &e.velocity,
                        &e.rate,
                        &e.life_time,
                        &e.size_x,
                        &e.size_y,
                    ]
                    .into_iter()
                    .chain(&e.direction)
                    {
                        emitter_keys += curve.0.len();
                        past_cycle += curve
                            .0
                            .iter()
                            .filter(|(t, _)| *t > e.cycle_length + 1e-3)
                            .count();
                    }
                    *billboards.entry(look.billboard).or_insert(0) += 1;
                    *rotations.entry(look.rotation_type).or_insert(0) += 1;
                    *animations.entry(look.texture_animation).or_insert(0) += 1;
                    *blends.entry((look.src_blend, look.dest_blend)).or_insert(0) += 1;
                    *operations.entry(look.colour_op).or_insert(0) += 1;
                    attached += usize::from(look.attach);
                    stretched += usize::from(look.stretch);
                    for curve in [
                        &look.air_resistance,
                        &look.alpha,
                        &look.gravity,
                        &look.rotation,
                    ]
                    .into_iter()
                    .chain(&look.colour)
                    .chain(&look.scale)
                    {
                        particle_keys += curve.0.len();
                        past_life += curve.0.iter().filter(|(t, _)| *t > 1.0 + 1e-3).count();
                    }
                    for texture in &look.textures {
                        textures.record(
                            find_effect_texture(vfs, name, texture.as_bytes()).is_some(),
                            || format!("{texture} in {shown}"),
                        );
                    }
                }
                Element::Mesh(m) => {
                    meshes += 1;
                    for look in &m.meshes {
                        *mesh_billboards.entry(look.billboard).or_insert(0) += 1;
                        let blend = look.blending.then_some((look.src_blend, look.dest_blend));
                        *mesh_blends.entry(blend).or_insert(0) += 1;
                    }
                    let path = find_effect_mesh(vfs, name, m.file.as_bytes());
                    mesh_files.record(path.is_some(), || format!("{} in {shown}", m.file));
                    let Some(path) = path else {
                        continue;
                    };
                    let file = read.entry(path.clone()).or_insert_with(|| {
                        vfs.read(&path)
                            .ok()
                            .and_then(|bytes| MeshEffect::parse(&bytes).ok())
                    });
                    mesh_unreadable.record(file.is_some(), || path.escape_ascii().to_string());
                    if let Some(file) = file {
                        element_counts.record(file.meshes.len() == m.meshes.len(), || {
                            format!(
                                "{shown}: {} elements for {} meshes",
                                m.meshes.len(),
                                file.meshes.len()
                            )
                        });
                    }
                }
            }
        }
    }
    for (path, file) in &read {
        for mesh in file.iter().flat_map(|f| &f.meshes) {
            mesh_textures.record(
                find_effect_texture(vfs, path, mesh.texture.as_bytes()).is_some(),
                || format!("{} in {}", mesh.texture, path.escape_ascii()),
            );
        }
    }
    println!(
        "effect data: {} scripts, {particles} particle emitters, {meshes} mesh elements; {bounded} with a bounding sphere",
        effects.len()
    );
    println!(
        "  emitter shapes: {}; advanced types: {}; {from_edge} from the edge",
        counts(&shapes),
        counts(&advanced)
    );
    println!(
        "  emitter keys: {emitter_keys}, {past_cycle} past the cycle's length; particle keys: {particle_keys}, {past_life} past 1; position keys: {direct} straight, {bezier} Bézier"
    );
    println!(
        "  particles: billboards {}; rotations {}; texture animations {}; colour operations {}; {attached} attached, {stretched} stretched",
        counts(&billboards),
        counts(&rotations),
        counts(&animations),
        counts(&operations)
    );
    println!(
        "  particle blending (source, destination): {}",
        counts(&blends)
    );
    println!(
        "  particle textures: {}{}",
        textures.total,
        textures.problems("missing")
    );
    println!(
        "  mesh elements: billboards {}; blending {}",
        counts(&mesh_billboards),
        counts(&mesh_blends)
    );
    println!(
        "  mesh files named: {}{}; {} different{}{}",
        mesh_files.total,
        mesh_files.problems("missing"),
        read.len(),
        mesh_unreadable.problems("unreadable"),
        element_counts.problems("with another number of elements than meshes"),
    );
    println!(
        "  their meshes' textures: {}{}",
        mesh_textures.total,
        mesh_textures.problems("missing")
    );

    let mut layouts = BTreeMap::new();
    let mut files = Tally::default();
    let (mut most_frames, mut most_meshes) = (0usize, 0usize);
    for name in names.iter().filter(|n| n.ends_with(b".mde")) {
        let parsed = vfs
            .read(name)
            .map_err(|e| e.to_string())
            .and_then(|bytes| MeshEffect::parse(&bytes).map_err(|e| e.to_string()));
        files.record(parsed.is_ok(), || match &parsed {
            Err(error) => format!("{}: {error}", name.escape_ascii()),
            Ok(_) => String::new(),
        });
        if let Ok(file) = parsed {
            let layout = match file.layout {
                Layout::EffectData => "EffectData",
                Layout::MdeData002 => "MDEData002",
            };
            *layouts.entry(layout).or_insert(0) += 1;
            most_frames = most_frames.max(file.frame_count());
            most_meshes = most_meshes.max(file.meshes.len());
        }
    }
    println!(
        "mesh effect files: {}; layouts {}; up to {most_meshes} meshes and {most_frames} frames{}",
        files.total,
        counts(&layouts),
        files.problems("unreadable")
    );
}

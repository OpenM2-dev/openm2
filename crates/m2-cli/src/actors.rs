//! Characters and monsters for `openm2 play`: their models from an asset
//! folder's actors, with the motions they stand, run, fight and use skills
//! with (how those hit, the effects and sounds they play), their size,
//! collision spheres and attached effects, and for characters what they
//! wear (`docs/specs/render.md`, "Playing"). Items on the ground are models
//! too, without motions.

use std::collections::BTreeMap;

use anyhow::{Context, Result, bail};
use glam::{Mat4, Vec3};
use m2_assets::actor::{
    FISHING_MOTIONS, GENERAL_MOTIONS, Grip, MAX_COMBO, MONSTER_ATTACK, MONSTER_DAMAGE,
    MONSTER_DEAD, MONSTER_RUN, PLAYER_ATTACKS, PLAYER_DAMAGES, PLAYER_DEAD, RIDING_MOTIONS,
    combo_motion, find_motion, hand_bones,
};
use m2_assets::actor_file::{ActorFile, MOTION_SUFFIX, MotionFile};
use m2_assets::effect_file::EffectFile;
use m2_assets::folder::{FOLDER_FILE, resolve};
use m2_assets::skills::{Skills, skill_motion_names};
use m2_formats::model::Animation;
use m2_formats::script::motion::{self, ComboInput, EventKind, MotionData, MotionListEntry};
use m2_formats::script::race::{AttachedEffect, Collision, RaceData, Smoke};
use m2_formats::script::sound::SoundScript;
use m2_formats::{RawStr, normalize_name};
use m2_render::{AnimationBinding, Bounds, Pose};

use crate::fight::{HitSphere, Strike};
use crate::source::Source;
use crate::view::{Files, LoadedPart, PartSpec, file_name, load_parts, read_animation};

/// The set of players' skill motions.
const SKILL_MOTIONS: &[u8] = b"skill";

/// A motion an actor plays, bound to its skeleton.
pub(crate) struct Motion {
    pub animation: Animation,
    pub binding: AnimationBinding,
    /// Centimetres a second the motion moves the actor forward (its
    /// `Accumulation` over its duration), for running.
    pub speed: Option<f32>,
    /// How often it is picked among its variants: a monster's motion list
    /// weight, 1 for players' motions.
    pub weight: u32,
    /// When the next attack of a combo may be asked for (players' attacks).
    pub combo: Option<ComboInput>,
    /// How it hits (attacks).
    pub strikes: Vec<Strike>,
    /// How far from the actor it hits, in centimetres along the ground.
    pub reach: Option<f32>,
    /// The effects its events play, in time order.
    pub effects: Vec<MotionEffect>,
    /// The flying effects its fly events shoot, in time order.
    pub flights: Vec<MotionFly>,
    /// The sounds its sound script plays, in time order.
    pub sounds: Vec<MotionSound>,
}

/// Something a motion does at a time into it.
pub(crate) trait Timed {
    /// Seconds into the motion.
    fn time(&self) -> f32;
}

impl Timed for MotionEffect {
    fn time(&self) -> f32 {
        self.time
    }
}

/// A sound a motion or an effect plays: one of its file's sounds.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct MotionSound {
    /// Seconds into the motion.
    pub time: f32,
    /// The sound file's path in the asset folder.
    pub file: Vec<u8>,
}

impl Timed for MotionSound {
    fn time(&self) -> f32 {
        self.time
    }
}

impl MotionSound {
    /// The sounds an asset folder's effect file plays; none if they can't
    /// be read (a note says why).
    pub(crate) fn of_effect(files: &Files<'_>, effect: &[u8]) -> Vec<Self> {
        match files {
            Files::Assets(folder) => {
                let read = std::str::from_utf8(effect)
                    .map_err(|_| "not UTF-8".to_owned())
                    .and_then(|path| {
                        let text = folder.read_text(path).map_err(|e| e.to_string())?;
                        let file = EffectFile::parse(&text)?;
                        Ok(file.to_effect(&|r| resolve(path, r))?.1)
                    });
                match read {
                    Ok(script) => Self::of_script(&script),
                    Err(error) => {
                        eprintln!("note: {}: {error}", effect.escape_ascii());
                        Vec::new()
                    }
                }
            }
            Files::Disk => Vec::new(),
        }
    }

    /// The sounds of a sound script, in time order.
    pub(crate) fn of_script(script: &SoundScript) -> Vec<Self> {
        let mut sounds: Vec<Self> = script
            .sounds
            .iter()
            .map(|sound| Self {
                time: sound.time,
                file: normalize_name(sound.file.as_bytes()),
            })
            .collect();
        sounds.sort_by(|a, b| a.time.total_cmp(&b.time));
        sounds
    }
}

/// An effect a motion plays (its motion script's effect events, types 1
/// and 10).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct MotionEffect {
    /// Seconds into the motion.
    pub time: f32,
    /// The effect script's game path.
    pub file: RawStr,
    /// Centimetres from the bone, or from the actor's feet.
    pub position: Vec3,
    /// The bone it starts from.
    pub bone: Option<RawStr>,
    /// It moves with the actor (`FollowingEnable`); otherwise it stays
    /// where it started.
    pub following: bool,
    /// A fishing effect (`FishingEffectFlag`): it plays at
    /// the actor's float on the water, unturned, in place of the last.
    pub fishing: bool,
}

/// A flying effect a motion shoots at a time into it (its fly events, type
/// 6), at what the server said it shot at (`render.md`, "Flying effects").
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct MotionFly {
    /// Seconds into the motion.
    pub time: f32,
    /// The flying effect file's path.
    pub file: RawStr,
    /// Centimetres from the bone, or from the actor's feet.
    pub position: Vec3,
    /// The bone it leaves from.
    pub bone: Option<RawStr>,
}

impl Timed for MotionFly {
    fn time(&self) -> f32 {
        self.time
    }
}

impl MotionFly {
    /// The fly events of a motion script.
    fn of(data: &MotionData) -> Vec<Self> {
        let mut flights: Vec<Self> = data
            .events
            .iter()
            .filter_map(|event| match &event.kind {
                EventKind::Fly(fly) => Some(Self {
                    time: event.time,
                    file: fly.file.clone(),
                    position: Vec3::from(fly.position),
                    bone: fly.bone.clone(),
                }),
                _ => None,
            })
            .collect();
        flights.sort_by(|a, b| a.time.total_cmp(&b.time));
        flights
    }
}

impl MotionEffect {
    /// The effect events of a motion script.
    fn of(data: &MotionData) -> Vec<Self> {
        let mut effects: Vec<Self> = data
            .events
            .iter()
            .filter_map(|event| match &event.kind {
                EventKind::Effect(effect) | EventKind::SpotEffect(effect) => Some(Self {
                    time: event.time,
                    file: effect.file.clone(),
                    position: Vec3::from(effect.position),
                    bone: effect.bone.clone(),
                    following: effect.following,
                    fishing: effect.fishing.unwrap_or(false),
                }),
                EventKind::Shake(_)
                | EventKind::Attack(_)
                | EventKind::Fly(_)
                | EventKind::Other(_) => None,
            })
            .collect();
        effects.sort_by(|a, b| a.time.total_cmp(&b.time));
        effects
    }
}

/// A character or monster model with its motions.
pub(crate) struct ActorModel {
    /// The parts drawn on the main skeleton, the body first.
    pub parts: Vec<LoadedPart>,
    /// Per part, the body's bone it is held at (a weapon), or `None` for a
    /// part that follows the body's bones (hair).
    pub held: Vec<Option<RawStr>>,
    /// The motion it stands with.
    pub wait: Option<Motion>,
    /// The motion it runs with.
    pub run: Option<Motion>,
    /// The motions it fights with.
    pub fight: FightMotions,
    /// The box its body and hair fill while it stands, in its own space
    /// (feet at the origin): for picking it and walking up to it.
    pub bounds: Option<Bounds>,
    /// Its model script's collision spheres while it stands, in its own
    /// space: for hitting it.
    pub spheres: Vec<HitSphere>,
    /// The effects its model script attaches to it, playing for as long as
    /// it shows.
    pub attached: Vec<AttachedEffect>,
    /// A metin stone's smoke, by stage.
    pub smoke: Option<Smoke>,
    /// Per part, what it shines as and how much, 0 to 1 (a player's armour
    /// and weapon); none for an actor that doesn't.
    pub shine: Vec<Option<(Shining, f32)>>,
    /// Problems worth telling the user.
    pub notes: Vec<String>,
}

/// The motions an actor fights with; any of them may be missing.
#[derive(Default)]
pub(crate) struct FightMotions {
    /// Players with a weapon: its combo, step 1 first (`combo_01.msa`, …).
    pub combo: Vec<Motion>,
    /// Single attacks: players' `attack.msa`, `attack_1.msa`, …; monsters'
    /// `NORMAL_ATTACK` and its variants.
    pub attack: Vec<Motion>,
    /// Flinching when hit: players' `damage.msa`, …; monsters'
    /// `FRONT_DAMAGE` and its variants.
    pub damage: Vec<Motion>,
    /// Falling dead: players' `dead.msa`, monsters' `FRONT_DEAD`.
    pub dead: Option<Motion>,
    /// Players' skills: their job's skill motions, each read once.
    pub skills: Vec<SkillMotion>,
    /// Which of `skills` plays a skill motion number at a grade (0 normal,
    /// 1 master, 2 grand master, 3 perfect master).
    pub skill_grades: BTreeMap<(u8, u8), usize>,
    /// Players holding a fishing pole: its fishing motions.
    pub fishing: FishingMotions,
}

/// The motions of a player holding a fishing pole, from the client's
/// `fishing/` folder; any of them may be missing.
#[derive(Default)]
pub(crate) struct FishingMotions {
    /// Casting the line (`throw`), once.
    pub throw: Option<Motion>,
    /// Waiting with the line out (`fishing_wait`), round and round.
    pub wait: Option<Motion>,
    /// A fish biting (`fishing_react`), once.
    pub react: Option<Motion>,
    /// Pulling a catch out (`fishing_catch`), once.
    pub catch: Option<Motion>,
    /// What bit got away (`fishing_fail`), once.
    pub fail: Option<Motion>,
    /// Reeling in before a bite (`fishing_cancel`), once.
    pub cancel: Option<Motion>,
}

/// One of the fishing motions played once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FishingMotion {
    Throw,
    React,
    Catch,
    Fail,
    Cancel,
}

impl FishingMotions {
    /// The motion `motion` names.
    pub fn get(&self, motion: FishingMotion) -> Option<&Motion> {
        match motion {
            FishingMotion::Throw => self.throw.as_ref(),
            FishingMotion::React => self.react.as_ref(),
            FishingMotion::Catch => self.catch.as_ref(),
            FishingMotion::Fail => self.fail.as_ref(),
            FishingMotion::Cancel => self.cancel.as_ref(),
        }
    }
}

/// A skill's motion (`docs/specs/protocol/openm2/skills.md`).
pub(crate) struct SkillMotion {
    /// The skill's number.
    pub skill: u8,
    pub motion: Motion,
}

/// A motion an actor plays once, over standing and running.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Once {
    /// A step of the combo, from 0.
    Combo(usize),
    /// One of the single attacks.
    Attack(usize),
    /// One of the flinches.
    Damage(usize),
    /// Falling dead: it stays in its last pose.
    Dead,
    /// One of the skill motions.
    Skill(usize),
    /// One of the fishing motions.
    Fishing(FishingMotion),
}

impl Once {
    /// True for the attacks, skills and fishing: hits do not stop them.
    pub fn is_attack(self) -> bool {
        matches!(
            self,
            Once::Combo(_) | Once::Attack(_) | Once::Skill(_) | Once::Fishing(_)
        )
    }
}

impl FightMotions {
    /// The motion of an attack move: combo step `step` (from 1) when it is
    /// one and the actor has it, else a single attack picked by weight with
    /// `seed`, else the combo's first step.
    pub fn attack_for(&self, step: Option<u8>, seed: u64) -> Option<Once> {
        let combo = step
            .and_then(|step| usize::from(step).checked_sub(1))
            .filter(|n| *n < self.combo.len())
            .map(Once::Combo);
        combo
            .or_else(|| pick(&self.attack, seed).map(Once::Attack))
            .or_else(|| (!self.combo.is_empty()).then_some(Once::Combo(0)))
    }

    /// A flinch picked by weight with `seed`.
    pub fn damage_for(&self, seed: u64) -> Option<Once> {
        pick(&self.damage, seed).map(Once::Damage)
    }

    /// The motion of skill motion `number` at `grade`, else at normal grade.
    pub fn skill_for(&self, number: u8, grade: u8) -> Option<Once> {
        self.skill_grades
            .get(&(number, grade.min(3)))
            .or_else(|| self.skill_grades.get(&(number, 0)))
            .map(|index| Once::Skill(*index))
    }

    /// The skill whose motion `once` is.
    pub fn skill_of(&self, once: Once) -> Option<u8> {
        match once {
            Once::Skill(n) => self.skills.get(n).map(|s| s.skill),
            _ => None,
        }
    }

    /// The motion `once` names.
    pub fn get(&self, once: Once) -> Option<&Motion> {
        match once {
            Once::Combo(n) => self.combo.get(n),
            Once::Attack(n) => self.attack.get(n),
            Once::Damage(n) => self.damage.get(n),
            Once::Dead => self.dead.as_ref(),
            Once::Skill(n) => self.skills.get(n).map(|s| &s.motion),
            Once::Fishing(motion) => self.fishing.get(motion),
        }
    }
}

/// One of `motions`, each as likely as its weight (all alike when the
/// weights are 0), chosen by `seed`.
fn pick(motions: &[Motion], seed: u64) -> Option<usize> {
    pick_weighted(motions.iter().map(|m| m.weight), seed)
}

fn pick_weighted(weights: impl Iterator<Item = u32> + Clone, seed: u64) -> Option<usize> {
    let count = weights.clone().count();
    if count == 0 {
        return None;
    }
    let total: u64 = weights.clone().map(u64::from).sum();
    if total == 0 {
        return usize::try_from(seed % count as u64).ok();
    }
    let mut left = seed % total;
    for (n, weight) in weights.enumerate() {
        let weight = u64::from(weight);
        if left < weight {
            return Some(n);
        }
        left -= weight;
    }
    Some(count - 1)
}

impl ActorModel {
    /// Skinning palettes for every part, posed by `motion` at `time`
    /// (seconds; it loops), or in the rest pose.
    pub fn palettes(&self, motion: Option<&Motion>, time: f32) -> Vec<Vec<Mat4>> {
        match self.pose(motion, time) {
            Some(pose) => self.palettes_of(&pose),
            None => Vec::new(),
        }
    }

    /// The body's pose in `motion` at `time` (seconds; it loops), or its
    /// rest pose; `None` without a body.
    pub fn pose(&self, motion: Option<&Motion>, time: f32) -> Option<Pose> {
        let skeleton = &self.parts.first()?.part.skeleton;
        Some(match motion {
            Some(m) if m.animation.duration > 0.0 => Pose::animated(
                skeleton,
                &m.animation,
                &m.binding,
                time.rem_euclid(m.animation.duration),
            ),
            Some(m) => Pose::animated(skeleton, &m.animation, &m.binding, 0.0),
            None => Pose::rest(skeleton),
        })
    }

    /// Skinning palettes for every part in the body's `pose`.
    pub fn palettes_of(&self, pose: &Pose) -> Vec<Vec<Mat4>> {
        self.parts
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let skeleton = &p.part.skeleton;
                if i == 0 {
                    return pose.palette(skeleton);
                }
                let held = self
                    .held
                    .get(i)
                    .and_then(Option::as_ref)
                    .and_then(|bone| pose.held(bone, skeleton));
                held.unwrap_or_else(|| pose.follow(skeleton))
                    .palette(skeleton)
            })
            .collect()
    }
}

/// Where a rider sits on a mount posed `pose`, in the mount's space: at its
/// seat bone ([`m2_assets::actor::SEAT_BONE`]), where it is and not how it
/// turns; a mount without one has its rider at 70% of its standing height
/// (openm2's choice; `docs/specs/render.md`, "Riding").
pub(crate) fn seat(mount: &ActorModel, pose: &Pose) -> Vec3 {
    pose.find(&RawStr::from(m2_assets::actor::SEAT_BONE))
        .map(|bone| bone.w_axis.truncate())
        .unwrap_or_else(|| {
            let height = mount.bounds.map_or(0.0, |bounds| bounds.max.z);
            Vec3::Z * height * 0.7
        })
}

/// What a part shines as: each has its own shine map.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Shining {
    /// A body armour, on its shape's own textures.
    Armour,
    /// A weapon, on all of its.
    Weapon,
}

/// What a player character wears, as far as it shows (from the server's
/// parts and the item table).
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Look {
    /// The body armour's shape index.
    pub shape: Option<u32>,
    /// The weapon: its model's game path and how it is held.
    pub weapon: Option<(String, Grip)>,
    /// Riding a mount: it moves with its riding motions.
    pub riding: bool,
    /// How much the body armour and the weapon shine, 0 to 1 (their
    /// items' shine).
    pub shine: [f32; 2],
}

/// A player character of `race` (class and gender) in the shape `look`
/// asks for (else its base outfit), with hair style `hair` if it has it
/// (else its first), holding the weapon `look` names, with the motions of
/// its job's skills in `skills`: from the asset folder's actor of that
/// race. Riding (`look.riding`), its motions are the riding ones: the
/// weapon's riding folder, then [`RIDING_MOTIONS`], and no job skills.
pub(crate) fn load_player(
    source: &Source,
    race: u16,
    hair: u16,
    look: &Look,
    skills: Option<&Skills>,
) -> Result<ActorModel> {
    let files = source.files();
    let entry = source
        .index
        .players
        .iter()
        .find(|p| p.race == race)
        .with_context(|| format!("{FOLDER_FILE} has no actor for player race {race}"))?;
    let path = resolve(FOLDER_FILE, &entry.actor).map_err(anyhow::Error::msg)?;
    let (file, data) = read_actor(&files, &path)?;
    let name = path.clone();
    let motions = PlayerMotions { file, path };
    let base = data
        .base_model
        .as_ref()
        .with_context(|| format!("{name} has no base model"))?;
    let mut notes = Vec::new();
    let shape = look.shape.and_then(|index| {
        let entry = data.shapes.find(index);
        if entry.is_none() {
            notes.push(format!("{name} has no shape {index}: the base outfit"));
        }
        entry
    });
    let mut specs = vec![match shape {
        Some(entry) => PartSpec::from_script(&data.shapes, entry),
        None => PartSpec::plain(&base.to_string()),
    }];
    // The armour's shine is on the textures its shape's skins bring, as
    // the client gives it: never the face, the hair or a bare body.
    let armour_skins: Vec<String> = match shape {
        Some(_) if look.shine[0] > 0.0 => specs[0].skins.iter().map(|(n, _)| n.clone()).collect(),
        _ => Vec::new(),
    };
    let hair = data
        .hairs
        .find(u32::from(hair))
        .or_else(|| data.hairs.entries.first());
    if let Some(entry) = hair {
        specs.push(PartSpec::from_script(&data.hairs, entry));
    }
    let weapon = look.weapon.as_ref().map(|(_, grip)| match look.riding {
        true => grip.riding,
        false => grip.motions,
    });
    let own = match look.riding {
        true => RIDING_MOTIONS,
        false => GENERAL_MOTIONS,
    };
    let folders: Vec<&[u8]> = weapon.into_iter().chain([own]).collect();
    let motion_paths = |name: &[u8]| {
        folders
            .iter()
            .filter_map(|folder| motions.path(folder, name))
            .collect::<Vec<_>>()
    };
    let mut model = finish(
        &files,
        &specs,
        &motion_paths(b"wait"),
        &motion_paths(b"run"),
    )?;
    model.notes.splice(0..0, notes);
    place_spheres(&mut model, &data.collisions);
    model.attached.clone_from(&data.effects);
    // The fight: the weapon's combo, then each single attack, flinch and
    // fall from the first folder that has it.
    let mut loader = Loader::new(&files, &model.parts);
    let mut fight = FightMotions::default();
    if let Some(weapon) = weapon {
        for step in 1..=MAX_COMBO {
            let found = motions
                .path(weapon, &combo_motion(step))
                .and_then(|path| loader.load(&path, 1));
            match found {
                Some(motion) => fight.combo.push(motion),
                None => break,
            }
        }
    }
    let mut first = |name: &[u8]| {
        folders.iter().find_map(|folder| {
            motions
                .path(folder, name)
                .and_then(|path| loader.load(&path, 1))
        })
    };
    fight.attack = PLAYER_ATTACKS
        .iter()
        .filter_map(|name| first(name))
        .collect();
    fight.damage = PLAYER_DAMAGES
        .iter()
        .filter_map(|name| first(name))
        .collect();
    fight.dead = first(PLAYER_DEAD);
    // A fishing pole's motions.
    if weapon == Some(FISHING_MOTIONS) {
        let mut fishing = |name: &[u8]| {
            motions
                .path(FISHING_MOTIONS, name)
                .and_then(|path| loader.load(&path, 1))
        };
        fight.fishing = FishingMotions {
            throw: fishing(b"throw"),
            wait: fishing(b"fishing_wait"),
            react: fishing(b"fishing_react"),
            catch: fishing(b"fishing_catch"),
            fail: fishing(b"fishing_fail"),
            cancel: fishing(b"fishing_cancel"),
        };
    }
    if let (Some(skills), Some(job)) = (skills, u8::try_from(race % 4).ok())
        && !look.riding
    {
        loader.skills(&mut fight, skills, job, &motions);
    }
    model.notes.append(&mut loader.notes);
    model.fight = fight;
    if let Some((path, grip)) = &look.weapon {
        hold(&files, &mut model, path, *grip);
    }
    let body_shine = if armour_skins.is_empty() {
        0.0
    } else {
        look.shine[0]
    };
    model.shine = part_shine(&model.held, body_shine, look.shine[1]);
    // Which textures shine: the armour's skins on the body, all of a
    // shining weapon's.
    for (n, (part, shine)) in model.parts.iter_mut().zip(&model.shine).enumerate() {
        let Some((what, _)) = shine else {
            continue;
        };
        for (texture, slot) in part.part.textures.iter().zip(&mut part.textures) {
            let shines = match what {
                Shining::Weapon => true,
                Shining::Armour => {
                    n == 0 && armour_skins.contains(&file_name(&texture.file.to_string()))
                }
            };
            if let Some(data) = slot.take() {
                *slot = Some(data.with_shine(shines));
            }
        }
    }
    Ok(model)
}

/// What each part of a player shines as and how much, by what holds it
/// (`held`): the body (the first part) as its armour, each held part (the
/// weapon in a hand) as the weapon, the rest (hair) not at all; nothing
/// without a shine.
fn part_shine(held: &[Option<RawStr>], body: f32, weapon: f32) -> Vec<Option<(Shining, f32)>> {
    held.iter()
        .enumerate()
        .map(|(n, held)| match held {
            Some(_) => Some((Shining::Weapon, weapon)),
            None if n == 0 => Some((Shining::Armour, body)),
            None => None,
        })
        .map(|shine| shine.filter(|(_, s)| *s > 0.0))
        .collect()
}

/// Where a player's motions are, by set (`general`, `onehand_sword`,
/// `skill`) and name (`run`, `combo_01`): an actor file's motions, the
/// actor at `path`.
struct PlayerMotions {
    file: ActorFile,
    path: String,
}

impl PlayerMotions {
    /// The path of the actor's motion file of motion `name` of `set`;
    /// `None` if the actor has no such motion.
    fn path(&self, set: &[u8], name: &[u8]) -> Option<String> {
        let set = std::str::from_utf8(set).ok()?;
        let name = std::str::from_utf8(name).ok()?;
        let entry = self.file.motion(Some(set), name)?;
        resolve(&self.path, &entry.file).ok()
    }
}

/// Reads the motions an actor may or may not have.
struct Loader<'a> {
    files: &'a Files<'a>,
    parts: &'a [LoadedPart],
    /// Motions that are there but do not read.
    notes: Vec<String>,
    /// The animations read so far, by path: motions share them.
    animations: BTreeMap<String, Animation>,
}

impl<'a> Loader<'a> {
    fn new(files: &'a Files<'a>, parts: &'a [LoadedPart]) -> Self {
        Self {
            files,
            parts,
            notes: Vec::new(),
            animations: BTreeMap::new(),
        }
    }

    /// The motion file at `path` with weight `weight`; `None` if there is
    /// no such file, or (noted) if it does not read.
    fn load(&mut self, path: &str, weight: u32) -> Option<Motion> {
        if !self.files.has(path.as_bytes()) {
            return None;
        }
        match load_motion_with(self.files, path, self.parts, Some(&mut self.animations)) {
            Ok(motion) => Some(Motion { weight, ..motion }),
            Err(error) => {
                self.notes.push(format!("{error:#}"));
                None
            }
        }
    }

    /// The motions of job `job`'s skills (0 warrior, 1 ninja, 2 sura, 3
    /// shaman) of a character whose motions are `motions`: for each skill
    /// and grade the first of its motions there is
    /// (`m2_assets::skills::skill_motion_names`), each read once.
    fn skills(
        &mut self,
        fight: &mut FightMotions,
        skills: &Skills,
        job: u8,
        motions: &PlayerMotions,
    ) {
        let mut by_path: BTreeMap<String, usize> = BTreeMap::new();
        for line in skills.of_job(job) {
            let (Ok(skill), Some(Ok(number))) =
                (u8::try_from(line.vnum), line.motion.map(u8::try_from))
            else {
                continue;
            };
            for grade in 0..4 {
                let found = skill_motion_names(line.icon.as_bytes(), grade)
                    .into_iter()
                    .filter_map(|name| motions.path(SKILL_MOTIONS, &name))
                    .find(|path| self.files.has(path.as_bytes()));
                let Some(path) = found else {
                    continue;
                };
                let index = match by_path.get(&path) {
                    Some(index) => *index,
                    None => {
                        let Some(motion) = self.load(&path, 1) else {
                            continue;
                        };
                        fight.skills.push(SkillMotion { skill, motion });
                        let index = fight.skills.len() - 1;
                        by_path.insert(path, index);
                        index
                    }
                };
                fight.skill_grades.insert((number, grade), index);
            }
        }
    }

    /// The fight motions a monster's motion list names: each with its
    /// numbered variants and their weights, under the first of its names
    /// the list has.
    fn monster_fight(&mut self, list: &[MotionListEntry], list_path: &str) -> FightMotions {
        let variants = |names: &[&str]| {
            names
                .iter()
                .map(|name| motion::variants(list, name))
                .find(|found| !found.is_empty())
                .unwrap_or_default()
        };
        let mut load_all = |names: &[&str]| -> Vec<Motion> {
            variants(names)
                .into_iter()
                .filter_map(|e| {
                    let path = e.script_path(list_path.as_bytes());
                    self.load(&String::from_utf8_lossy(&path), e.weight)
                })
                .collect()
        };
        let attack = load_all(&[MONSTER_ATTACK]);
        let damage = load_all(&MONSTER_DAMAGE);
        let dead = load_all(&MONSTER_DEAD).into_iter().next();
        FightMotions {
            combo: Vec::new(),
            attack,
            damage,
            dead,
            ..FightMotions::default()
        }
    }
}

/// Puts the weapon model at `path` into the hands `grip` says.
fn hold(files: &Files<'_>, model: &mut ActorModel, path: &str, grip: Grip) {
    let Some(body) = model.parts.first() else {
        return;
    };
    let bones = hand_bones(&body.part.skeleton, grip.hands);
    if bones.is_empty() {
        model
            .notes
            .push(format!("the body has no bone to hold {path} with"));
        return;
    }
    for bone in bones {
        match load_parts(files, &[PartSpec::plain(path)], &[], &mut model.notes) {
            Ok(parts) => {
                for part in parts {
                    model.parts.push(part);
                    model.held.push(Some(bone.clone()));
                }
            }
            Err(error) => {
                model.notes.push(format!("{error:#}"));
                return;
            }
        }
    }
}

/// Monster or NPC number `vnum`: by the asset folder's actor of that
/// monster.
pub(crate) fn load_monster(source: &Source, vnum: u16) -> Result<ActorModel> {
    let files = source.files();
    let actor = source
        .index
        .monsters
        .iter()
        .find(|m| m.vnum == u32::from(vnum))
        .and_then(|m| m.actor.as_ref())
        .with_context(|| format!("{FOLDER_FILE} has no actor for monster {vnum}"))?;
    let path = resolve(FOLDER_FILE, actor).map_err(anyhow::Error::msg)?;
    let (file, data) = read_actor(&files, &path)?;
    // The actor's motions as a motion list of the motion files' paths.
    let list: Vec<MotionListEntry> = file
        .motions
        .iter()
        .filter_map(|m| {
            let motion = resolve(&path, &m.file).ok()?;
            Some(MotionListEntry {
                mode: RawStr::from_bytes(m.set.as_bytes()),
                name: RawStr::from_bytes(m.name.as_bytes()),
                file: RawStr::from_bytes(motion.as_bytes()),
                weight: m.weight,
            })
        })
        .collect();
    let list_path = String::new();
    let base = data
        .base_model
        .as_ref()
        .with_context(|| format!("monster {vnum} has no base model"))?;
    let specs = [PartSpec::plain(&base.to_string())];
    let find = |names: &[&str]| {
        find_motion(&list, list_path.as_bytes(), names)
            .map(|path| String::from_utf8_lossy(&path).into_owned())
    };
    let (wait, run) = (find(&["WAIT"]), find(&MONSTER_RUN));
    let mut model = finish(
        &files,
        &specs,
        &wait.into_iter().collect::<Vec<_>>(),
        &run.into_iter().collect::<Vec<_>>(),
    )?;
    place_spheres(&mut model, &data.collisions);
    model.attached.clone_from(&data.effects);
    model.smoke.clone_from(&data.smoke);
    let mut loader = Loader::new(&files, &model.parts);
    let fight = loader.monster_fight(&list, &list_path);
    model.notes.append(&mut loader.notes);
    model.fight = fight;
    Ok(model)
}

/// An item's model on the ground (`ground.rs`): one part, no motions.
pub(crate) fn load_item(files: &Files<'_>, path: &str) -> Result<ActorModel> {
    finish(files, &[PartSpec::plain(path)], &[], &[])
}

fn files_read(files: &Files<'_>, path: &str) -> Result<Vec<u8>> {
    match files {
        Files::Assets(_) => files.read(path),
        Files::Disk => bail!("{path}: actors and motions are read from an asset folder"),
    }
}

/// The actor file at `path` of an asset folder, and what it says in the
/// client's types (its files as paths of the folder).
fn read_actor(files: &Files<'_>, path: &str) -> Result<(ActorFile, RaceData)> {
    let text = String::from_utf8(files_read(files, path)?)
        .with_context(|| format!("{path} is not UTF-8 text"))?;
    let file = ActorFile::parse(&text)
        .map_err(anyhow::Error::msg)
        .with_context(|| format!("reading {path}"))?;
    let data = file
        .to_race(&|reference| resolve(path, reference))
        .map_err(anyhow::Error::msg)
        .with_context(|| format!("reading {path}"))?;
    Ok((file, data))
}

/// Reads the parts and the two motions, each the first of its paths that
/// reads.
fn finish(
    files: &Files<'_>,
    specs: &[PartSpec],
    wait: &[String],
    run: &[String],
) -> Result<ActorModel> {
    let mut notes = Vec::new();
    let parts = load_parts(files, specs, &[], &mut notes)?;
    let mut motion = |paths: &[String]| -> Option<Motion> {
        let mut last = None;
        for path in paths {
            match load_motion(files, path, &parts) {
                Ok(motion) => return Some(motion),
                Err(error) => last = Some(error),
            }
        }
        if let Some(error) = last {
            notes.push(format!("{error:#}"));
        }
        None
    };
    let wait = motion(wait);
    let run = motion(run);
    let mut model = ActorModel {
        held: vec![None; parts.len()],
        parts,
        wait,
        run,
        fight: FightMotions::default(),
        bounds: None,
        spheres: Vec::new(),
        attached: Vec::new(),
        smoke: None,
        shine: Vec::new(),
        notes,
    };
    model.bounds = standing_bounds(&model);
    Ok(model)
}

/// The box around a model's parts over eight poses of its wait motion (the
/// rest pose without one).
pub(crate) fn standing_bounds(model: &ActorModel) -> Option<Bounds> {
    let motion = model.wait.as_ref();
    let duration = motion.map_or(0.0, |m| m.animation.duration);
    let mut bounds: Option<Bounds> = None;
    for k in 0..8 {
        let time = duration * k as f32 / 8.0;
        for (part, palette) in model.parts.iter().zip(model.palettes(motion, time)) {
            if let Some(b) = part.part.bounds(&palette) {
                bounds = Some(bounds.map_or(b, |a| a.union(b)));
            }
        }
    }
    bounds
}

/// Puts the collision spheres of a model script on the model, where they
/// are while it stands: from its origin, or from their bone in the first
/// pose of its wait motion (the rest pose without one), the offset in the
/// model's axes. Spheres on a bone the model lacks are noted and placed
/// from the origin.
fn place_spheres(model: &mut ActorModel, collisions: &[Collision]) {
    let Some(body) = model.parts.first() else {
        return;
    };
    let skeleton = &body.part.skeleton;
    let pose = match &model.wait {
        Some(m) => Pose::animated(skeleton, &m.animation, &m.binding, 0.0),
        None => Pose::rest(skeleton),
    };
    let mut spheres = Vec::new();
    for collision in collisions {
        let from = if collision.attached {
            match pose.find(&collision.bone) {
                Some(bone) => bone.w_axis.truncate(),
                None => {
                    model.notes.push(format!(
                        "the model has no bone {} for its collision spheres",
                        collision.bone
                    ));
                    Vec3::ZERO
                }
            }
        } else {
            Vec3::ZERO
        };
        spheres.extend(collision.spheres.iter().map(|sphere| HitSphere {
            centre: from + Vec3::from_array(sphere.position),
            radius: sphere.radius,
        }));
    }
    model.spheres = spheres;
}

/// A motion file of an asset folder, its animation, and how fast it moves
/// forward.
fn load_motion(files: &Files<'_>, path: &str, parts: &[LoadedPart]) -> Result<Motion> {
    load_motion_with(files, path, parts, None)
}

/// [`load_motion`], taking its animation from `animations` if it was read
/// before (and keeping it there).
fn load_motion_with(
    files: &Files<'_>,
    path: &str,
    parts: &[LoadedPart],
    animations: Option<&mut BTreeMap<String, Animation>>,
) -> Result<Motion> {
    if !path.to_ascii_lowercase().ends_with(MOTION_SUFFIX) {
        bail!("{path} is not a motion file ({MOTION_SUFFIX})");
    }
    let text = String::from_utf8(files_read(files, path)?)
        .with_context(|| format!("{path} is not UTF-8 text"))?;
    let (data, script) = MotionFile::parse(&text)
        .and_then(|file| file.to_motion(&|reference| resolve(path, reference)))
        .map_err(anyhow::Error::msg)
        .with_context(|| format!("reading {path}"))?;
    let sounds = MotionSound::of_script(&script);
    let gr2 = data.motion_file.to_string();
    let key = String::from_utf8_lossy(&normalize_name(gr2.as_bytes())).into_owned();
    let read = || read_animation(files, &gr2);
    let animation = match animations {
        Some(cache) => match cache.get(&key) {
            Some(animation) => animation.clone(),
            None => {
                let animation = read()?;
                cache.insert(key, animation.clone());
                animation
            }
        },
        None => read()?,
    };
    let skeleton = parts.first().map(|p| &p.part.skeleton);
    let binding = AnimationBinding::new(skeleton.unwrap_or(&Default::default()), &animation);
    let speed = data.forward_speed(Some(animation.duration));
    Ok(Motion {
        animation,
        binding,
        speed,
        weight: 1,
        combo: data.combo,
        strikes: Strike::of(&data),
        reach: data.reach(),
        effects: MotionEffect::of(&data),
        flights: MotionFly::of(&data),
        sounds,
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use m2_formats::model::Skeleton;

    use super::*;

    /// A motion of `duration` seconds that moves nothing.
    pub(crate) fn motion(duration: f32, weight: u32) -> Motion {
        let animation = Animation {
            name: RawStr::from("test"),
            duration,
            track_groups: Vec::new(),
        };
        Motion {
            binding: AnimationBinding::new(&Skeleton::default(), &animation),
            animation,
            speed: None,
            weight,
            combo: None,
            strikes: Vec::new(),
            reach: None,
            effects: Vec::new(),
            flights: Vec::new(),
            sounds: Vec::new(),
        }
    }

    #[test]
    fn variants_are_picked_by_weight() {
        let weights = [50, 30, 20];
        let picks: Vec<_> = [0, 49, 50, 79, 80, 99, 100]
            .into_iter()
            .map(|seed| pick_weighted(weights.into_iter(), seed))
            .collect();
        assert_eq!(
            picks,
            [0, 0, 1, 1, 2, 2, 0].map(Some),
            "the seed walks the weights, round and round"
        );
        assert_eq!(pick_weighted([0, 0].into_iter(), 3), Some(1), "all alike");
        assert_eq!(pick_weighted([].into_iter(), 3), None);
    }

    #[test]
    fn attack_moves_play_the_combo_step_or_a_single_attack() {
        let fight = FightMotions {
            combo: vec![motion(1.0, 1), motion(1.0, 1)],
            attack: vec![motion(1.0, 1)],
            damage: vec![motion(0.5, 1)],
            ..FightMotions::default()
        };
        assert_eq!(fight.attack_for(Some(2), 0), Some(Once::Combo(1)));
        // A step the weapon does not have, or a single attack.
        assert_eq!(fight.attack_for(Some(3), 0), Some(Once::Attack(0)));
        assert_eq!(fight.attack_for(Some(0), 0), Some(Once::Attack(0)));
        assert_eq!(fight.attack_for(None, 7), Some(Once::Attack(0)));
        // Without single attacks, the combo's first step.
        let combo_only = FightMotions {
            combo: vec![motion(1.0, 1)],
            ..FightMotions::default()
        };
        assert_eq!(combo_only.attack_for(None, 0), Some(Once::Combo(0)));
        assert_eq!(FightMotions::default().attack_for(Some(1), 0), None);
        assert_eq!(fight.damage_for(5), Some(Once::Damage(0)));
        assert!(fight.get(Once::Dead).is_none());
        assert!(Once::Combo(0).is_attack() && !Once::Damage(0).is_attack());
        assert!(Once::Skill(0).is_attack());
    }

    #[test]
    fn skill_moves_play_their_grades_motion() {
        let skill = |skill| SkillMotion {
            skill,
            motion: motion(1.0, 1),
        };
        let fight = FightMotions {
            skills: vec![skill(1), skill(1), skill(2)],
            skill_grades: [((1, 0), 0), ((1, 1), 1), ((1, 3), 1), ((2, 0), 2)]
                .into_iter()
                .collect(),
            ..FightMotions::default()
        };
        assert_eq!(fight.skill_for(1, 0), Some(Once::Skill(0)));
        assert_eq!(fight.skill_for(1, 1), Some(Once::Skill(1)));
        assert_eq!(fight.skill_for(1, 9), Some(Once::Skill(1)), "past perfect");
        // A grade without its own: normal's.
        assert_eq!(fight.skill_for(2, 2), Some(Once::Skill(2)));
        assert_eq!(fight.skill_for(3, 0), None);
        assert_eq!(fight.skill_of(Once::Skill(2)), Some(2));
        assert_eq!(fight.skill_of(Once::Dead), None);
        assert!(fight.get(Once::Skill(1)).is_some());
    }

    #[test]
    fn the_body_shines_as_its_armour_the_weapons_as_the_weapon() {
        let hand = Some(RawStr::from("equip_right"));
        // Body, hair, a dagger in each hand.
        let held = [None, None, hand.clone(), hand];
        let (armour, weapon) = (Shining::Armour, Shining::Weapon);
        assert_eq!(
            part_shine(&held, 0.3, 1.0),
            [
                Some((armour, 0.3)),
                None,
                Some((weapon, 1.0)),
                Some((weapon, 1.0))
            ]
        );
        assert_eq!(part_shine(&[None], 0.0, 0.65), [None]);
        assert!(part_shine(&[], 1.0, 1.0).is_empty());
    }

    /// An import of the user's own client (`openm2 assets import`), from
    /// `OPENM2_ASSETS_DIR`; `None`, and a note, without it.
    pub(crate) fn imported() -> Option<Source> {
        let Some(dir) = std::env::var_os("OPENM2_ASSETS_DIR") else {
            eprintln!("skipped: set OPENM2_ASSETS_DIR to an import of a real client");
            return None;
        };
        Some(Source::open(dir.as_ref()).unwrap())
    }

    /// A warrior holding the client's first sword.
    pub(crate) fn sword() -> Look {
        Look {
            shape: None,
            weapon: Some((
                "ymir work/item/weapon/00010.glb".to_owned(),
                Grip {
                    motions: b"onehand_sword",
                    riding: b"horse_onehand_sword",
                    hands: m2_assets::actor::Hands::Right,
                },
            )),
            riding: false,
            shine: [0.0; 2],
        }
    }

    /// Against an import of the user's own client: skipped unless
    /// `OPENM2_ASSETS_DIR` is set.
    #[test]
    fn a_riding_warrior_has_the_riding_motions() {
        let Some(source) = imported() else {
            return;
        };
        let skills = source.skills().unwrap();
        let riding = Look {
            riding: true,
            ..sword()
        };
        let warrior = load_player(&source, 0, 0, &riding, Some(&skills)).unwrap();
        let fight = &warrior.fight;
        // The riding sword's three steps; no single attack; one flinch.
        assert_eq!(
            (fight.combo.len(), fight.attack.len(), fight.damage.len()),
            (3, 0, 1),
            "{:?}",
            warrior.notes
        );
        assert!(fight.skills.is_empty() && fight.dead.is_some());
        let on_foot = load_player(&source, 0, 0, &sword(), None).unwrap();
        // It waits sitting: another pose than standing.
        let waiting = |m: &ActorModel| m.pose(m.wait.as_ref(), 0.0);
        assert!(waiting(&warrior).is_some());
        assert_ne!(waiting(&warrior), waiting(&on_foot));
    }

    /// Against an import of the user's own client: skipped unless
    /// `OPENM2_ASSETS_DIR` is set.
    #[test]
    fn a_warrior_with_a_sword_and_a_wild_dog_fight_with_the_clients_motions() {
        let Some(source) = imported() else {
            return;
        };
        let skills = source.skills().unwrap();
        let warrior = load_player(&source, 0, 0, &sword(), Some(&skills)).unwrap();
        let fight = &warrior.fight;
        assert_eq!(
            (fight.combo.len(), fight.attack.len(), fight.damage.len()),
            (7, 3, 4),
            "{:?}",
            warrior.notes
        );
        // The warrior's ten skills, each with its four grades; Three-Way
        // Cut's sphere events hit.
        assert_eq!(fight.skill_grades.len(), 40, "{:?}", warrior.notes);
        let samyeon = fight.skill_for(1, 0).unwrap();
        assert_eq!(fight.skill_of(samyeon), Some(1));
        assert_eq!(fight.get(samyeon).unwrap().strikes.len(), 3);
        assert_ne!(fight.skill_for(1, 0), fight.skill_for(1, 2));
        assert!(fight.dead.is_some(), "{:?}", warrior.notes);
        // The first swing hits with the sword and reaches 2 m; the chain's
        // fourth step takes no next attack.
        let first = &fight.combo[0];
        assert!(!first.strikes.is_empty() && first.combo.is_some());
        assert!(
            (first.reach.unwrap() - 202.6).abs() < 0.1,
            "{:?}",
            first.reach
        );
        let fourth = fight.combo[3].combo.unwrap();
        assert!(fourth.pre_input > fourth.input_limit, "{fourth:?}");
        let bounds = warrior.bounds.unwrap();
        assert!((150.0..250.0).contains(&bounds.max.z), "{bounds:?}");
        let dog = load_monster(&source, 101).unwrap();
        let fight = &dog.fight;
        assert_eq!(
            (fight.attack.len(), fight.damage.len()),
            (2, 1),
            "{:?}",
            dog.notes
        );
        assert!(fight.dead.is_some(), "{:?}", dog.notes);
        assert_eq!(
            fight.attack.iter().map(|m| m.weight).collect::<Vec<_>>(),
            [50, 50]
        );
        assert!(fight.attack.iter().all(|m| !m.strikes.is_empty()));
        // Its two spheres, from its feet (not on a bone).
        let spheres: Vec<([f32; 3], f32)> = dog
            .spheres
            .iter()
            .map(|s| (s.centre.to_array(), s.radius))
            .collect();
        assert_eq!(
            spheres,
            [([0.0, -10.0, 100.0], 60.0), ([0.0, -10.0, 80.0], 90.0)]
        );
        // The warrior's are on its pelvis (`Bip01`), about 90 cm up.
        assert_eq!(warrior.spheres.len(), 2, "{:?}", warrior.notes);
        for sphere in &warrior.spheres {
            assert!((70.0..110.0).contains(&sphere.centre.z), "{sphere:?}");
        }
    }
}

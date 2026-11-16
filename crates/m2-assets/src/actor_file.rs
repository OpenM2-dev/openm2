//! Actor and motion files of an asset folder (`docs/specs/assets.md`,
//! "Actors" and "Motions"): openm2's TOML for what the client's model
//! scripts, motion scripts, motion lists and motion sound scripts say.
//!
//! - An **actor** (`*.actor.toml`, [`ActorFile`]): the models of its body by
//!   shape and of its hair, each with the textures that replace the model's
//!   own; its collision spheres; the effects it carries; a metin stone's
//!   smoke; its motions, by set and name.
//! - A **motion** (`*.motion.toml`, [`MotionFile`]): the animation, its
//!   length and how far it moves, the combo window, where it hits, what
//!   happens at given times, and its sounds.
//!
//! Lengths are metres and times seconds, in the model's axes as the
//! renderer has them: +Z up, forward −Y (as Blender shows a model).
//! Numbers are written with the fewest digits that read back as the same
//! value. Files name others by references (`crate::folder::resolve`).
//!
//! Both convert from the client's types ([`ActorFile::from_race`],
//! [`MotionFile::from_motion`]), for `openm2 assets import`, and back
//! ([`ActorFile::to_race`], [`MotionFile::to_motion`]), so the client's
//! scripts and an asset folder are drawn and played by the same code.

use serde::{Deserialize, Serialize};

use m2_formats::RawStr;
use m2_formats::script::motion::{
    AttackEvent, Attacking, ComboInput, Effect, EventKind, Fly, Hit, HitPosition, MotionData,
    MotionEvent, Shake, Sphere,
};
use m2_formats::script::race::{
    AttachedEffect, Collision, PartEntry, PartList, RaceData, Skin, Smoke,
};
use m2_formats::script::sound::{SoundScript, TimedSound};

use crate::folder::{FORMAT, from_toml};

/// The end of an actor file's name.
pub const ACTOR_SUFFIX: &str = ".actor.toml";
/// The end of a motion file's name.
pub const MOTION_SUFFIX: &str = ".motion.toml";

/// The fewest digits of `value` that `same` accepts, as a number.
pub fn shortest(value: f64, same: impl Fn(f64) -> bool) -> f64 {
    for digits in 0..17 {
        if let Ok(candidate) = format!("{value:.digits$e}").parse::<f64>()
            && same(candidate)
        {
            return candidate;
        }
    }
    value
}

/// Centimetres as metres, with the fewest digits that read back as the
/// same centimetres ([`centimetres`]).
pub fn metres(cm: f32) -> f64 {
    if !cm.is_finite() {
        return f64::from(cm);
    }
    shortest(f64::from(cm) / 100.0, |m| centimetres(m) == cm)
}

/// Metres as centimetres.
pub fn centimetres(m: f64) -> f32 {
    (m * 100.0) as f32
}

/// A time or another plain number, with the fewest digits that read back
/// as the same f32.
pub fn plain(value: f32) -> f64 {
    if !value.is_finite() {
        return f64::from(value);
    }
    shortest(f64::from(value), |v| v as f32 == value)
}

fn point_m(p: [f32; 3]) -> [f64; 3] {
    p.map(metres)
}

fn point_cm(p: [f64; 3]) -> [f32; 3] {
    p.map(centimetres)
}

fn is_one(weight: &u32) -> bool {
    *weight == 1
}

fn one() -> u32 {
    1
}

fn is_false(value: &bool) -> bool {
    !*value
}

/// An actor file.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActorFile {
    /// [`FORMAT`].
    pub format: u32,
    /// The model drawn before a shape is chosen, or the only one
    /// (`BaseModelFileName`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Body models, chosen by shape index.
    #[serde(default, rename = "shape", skip_serializing_if = "Vec::is_empty")]
    pub shapes: Vec<PartFile>,
    /// Hair models, chosen by hair index.
    #[serde(default, rename = "hair", skip_serializing_if = "Vec::is_empty")]
    pub hairs: Vec<PartFile>,
    /// Collision spheres.
    #[serde(default, rename = "collision", skip_serializing_if = "Vec::is_empty")]
    pub collisions: Vec<CollisionFile>,
    /// Effects that play for as long as the actor shows.
    #[serde(default, rename = "effect", skip_serializing_if = "Vec::is_empty")]
    pub effects: Vec<AttachedEffectFile>,
    /// A metin stone's smoke.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub smoke: Option<SmokeFile>,
    /// Motions.
    #[serde(default, rename = "motion", skip_serializing_if = "Vec::is_empty")]
    pub motions: Vec<MotionEntry>,
}

/// A shape or a hair style.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PartFile {
    /// The number the game picks it by.
    pub index: u32,
    /// The model (`.glb`).
    pub model: String,
    /// Textures of the model drawn with others.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skins: Vec<SkinFile>,
}

/// A texture of a model drawn with another.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkinFile {
    /// The file name of the texture the model shows (case ignored).
    pub source: String,
    /// The texture drawn instead (`.png`).
    pub target: String,
}

/// Collision spheres.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CollisionFile {
    /// `CollisionType`: the client has 1 and 3; what each is for is open.
    pub kind: u32,
    /// The bone the spheres are placed from; without one, the model's
    /// origin.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bone: Option<String>,
    /// The spheres.
    pub spheres: Vec<SphereFile>,
}

/// A sphere.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SphereFile {
    /// Its radius, metres.
    pub radius: f64,
    /// Its centre, metres from the bone or the origin.
    pub at: [f64; 3],
}

/// An effect an actor carries.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttachedEffectFile {
    /// The bone it is placed from; without one, the model's origin.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bone: Option<String>,
    /// The effect.
    pub file: String,
    /// Where, metres from the bone or the origin.
    pub at: [f64; 3],
    /// How it is turned: degrees about x, y and z.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn: Option<[f64; 3]>,
}

/// A metin stone's smoke: an effect per stage, on a bone.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SmokeFile {
    /// The bone.
    pub bone: String,
    /// The stages.
    pub stages: Vec<SmokeStage>,
}

/// One stage of the smoke.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SmokeStage {
    /// From 0, stronger as it goes.
    pub stage: u32,
    /// The effect.
    pub file: String,
}

/// One of an actor's motions.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MotionEntry {
    /// Players': the motion's folder (`general`, `onehand_sword`, `skill`);
    /// monsters': the mode of their motion list (`GENERAL`).
    pub set: String,
    /// Players': the motion's name (`run`, `combo_01`); monsters': the
    /// list's (`RUN`, `WAIT1`).
    pub name: String,
    /// The motion file.
    pub file: String,
    /// How often it is picked among a motion's numbered variants.
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub weight: u32,
}

/// A motion file.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MotionFile {
    /// [`FORMAT`].
    pub format: u32,
    /// The animation: the first of a `.glb` file.
    pub animation: String,
    /// Its length in seconds, when not the animation's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration: Option<f64>,
    /// How far it moves the actor, metres (walking, running, attacking).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub moves: Option<[f64; 3]>,
    /// When the next attack of a combo may be asked for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub combo: Option<ComboFile>,
    /// When and where it hits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attack: Option<AttackFile>,
    /// What happens at given times, in order.
    #[serde(default, rename = "event", skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<EventFile>,
    /// Sounds at given times.
    #[serde(default, rename = "sound", skip_serializing_if = "Vec::is_empty")]
    pub sounds: Vec<SoundFile>,
}

/// The combo window, in seconds into the motion.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComboFile {
    /// The earliest a press counts for the next attack.
    pub pre_input: f64,
    /// The earliest the next attack starts.
    pub direct_input: f64,
    /// The latest a press counts.
    pub input_limit: f64,
    /// `LinkTime`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<f64>,
}

/// How a motion hits (the client's `AttackingData`; what most numbers mean
/// is open, `docs/specs/formats/script.md`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttackFile {
    /// `AttackType` (the newer layout).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attack_type: Option<u32>,
    /// `AttackingType` (the older layout, with exactly one hit).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attacking_type: Option<u32>,
    /// `MotionType`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub motion_type: Option<u32>,
    /// `HittingType`.
    pub hitting_type: u32,
    /// Seconds.
    pub stiffen_time: f64,
    /// Seconds.
    pub invisible_time: f64,
    /// `ExternalForce`.
    pub external_force: f64,
    /// `HitLimitCount`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hit_limit: Option<u32>,
    /// The hits.
    #[serde(default, rename = "hit", skip_serializing_if = "Vec::is_empty")]
    pub hits: Vec<HitFile>,
}

/// One hit: while it lasts, the bone or its weapon sweeps through its
/// positions.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HitFile {
    /// Seconds into the motion.
    pub start: f64,
    /// Seconds into the motion.
    pub end: f64,
    /// The bone that strikes; none for none.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub bone: String,
    /// Metres; 0 for a fist or a claw.
    pub weapon_length: f64,
    /// Rows of a time (seconds) and two points (metres): the bone, the
    /// weapon's tip.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub positions: Vec<[f64; 7]>,
}

/// Something a motion does at a time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum EventFile {
    /// An effect on a bone or where the actor is.
    Effect(EffectEvent),
    /// An effect that doesn't attach to a bone.
    SpotEffect(EffectEvent),
    /// The screen shaking (openm2's reading).
    Shake {
        /// Seconds into the motion.
        time: f64,
        /// Seconds.
        during: f64,
        /// `Power`.
        power: f64,
        /// Metres.
        range: f64,
    },
    /// Spheres that hit what they touch for a while.
    Attack(AttackEventFile),
    /// Something that flies off: an arrow, a spell.
    Fly {
        /// Seconds into the motion.
        time: f64,
        /// The flying effect.
        file: String,
        /// Metres from the bone or the actor.
        at: [f64; 3],
        /// The bone it starts from.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        bone: Option<String>,
    },
    /// An event of another type, kept by number.
    Other {
        /// Seconds into the motion.
        time: f64,
        /// The client's `MotionEventType`.
        number: u32,
    },
}

/// An effect event.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EffectEvent {
    /// Seconds into the motion.
    pub time: f64,
    /// The effect.
    pub file: String,
    /// Metres from the bone or the actor.
    pub at: [f64; 3],
    /// The bone it starts from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bone: Option<String>,
    /// It moves with the actor.
    #[serde(default, skip_serializing_if = "is_false")]
    pub following: bool,
    /// `IndependentFlag`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub independent: Option<bool>,
    /// `FishingEffectFlag`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fishing: Option<bool>,
}

/// An attack event.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttackEventFile {
    /// Seconds into the motion.
    pub time: f64,
    /// Seconds.
    pub during: f64,
    /// `EnableHitProcess`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enable_hit: Option<bool>,
    /// `AttackType`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attack_type: Option<u32>,
    /// `HittingType`.
    pub hitting_type: u32,
    /// Seconds.
    pub stiffen_time: f64,
    /// Seconds.
    pub invisible_time: f64,
    /// `ExternalForce`.
    pub external_force: f64,
    /// `HitLimitCount`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hit_limit: Option<u32>,
    /// `CollisionType`.
    pub collision_type: u32,
    /// The spheres.
    pub spheres: Vec<SphereFile>,
}

/// A sound at a time.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SoundFile {
    /// Seconds into the motion.
    pub time: f64,
    /// The sound (`.wav` or `.mp3`).
    pub file: String,
}

/// Turns names (bones and the like) to text and back.
fn text(raw: &RawStr) -> String {
    raw.to_text().into_owned()
}

fn raw(text: &str) -> RawStr {
    RawStr::from_bytes(text.as_bytes())
}

/// What a client file names, and how a file of the folder names it.
pub trait References {
    /// The reference to write for a file the client names by `game_path`.
    fn refer(&mut self, game_path: &[u8]) -> String;
    /// The file name of the texture at `game_path` as the folder has it
    /// (a skin's source).
    fn texture_name(&mut self, game_path: &[u8]) -> String;
}

impl ActorFile {
    /// Parses an actor file.
    pub fn parse(text: &str) -> Result<Self, String> {
        from_toml(text, |file: &Self| file.format)
    }

    /// The actor of a model script at `script` (a game path); the motions
    /// are the caller's.
    pub fn from_race(race: &RaceData, refs: &mut dyn References) -> Self {
        let parts = |list: &PartList, refs: &mut dyn References| -> Vec<PartFile> {
            list.entries
                .iter()
                .map(|entry| PartFile {
                    index: entry.index,
                    model: refs.refer(&list.resolve(entry, &entry.model)),
                    skins: entry
                        .skins
                        .iter()
                        .filter_map(|skin| {
                            let target = skin.target.as_ref()?;
                            Some(SkinFile {
                                source: refs.texture_name(&list.resolve(entry, &skin.source)),
                                target: refs.refer(&list.resolve(entry, target)),
                            })
                        })
                        .collect(),
                })
                .collect()
        };
        Self {
            format: FORMAT,
            model: race.base_model.as_ref().map(|m| refs.refer(m.as_bytes())),
            shapes: parts(&race.shapes, refs),
            hairs: parts(&race.hairs, refs),
            collisions: race
                .collisions
                .iter()
                .map(|c| CollisionFile {
                    kind: c.collision_type,
                    bone: c.attached.then(|| text(&c.bone)),
                    spheres: c.spheres.iter().map(sphere_file).collect(),
                })
                .collect(),
            effects: race
                .effects
                .iter()
                .map(|e| AttachedEffectFile {
                    bone: e.attached.then(|| text(&e.bone)),
                    file: refs.refer(e.file.as_bytes()),
                    at: point_m(e.position),
                    turn: e.rotation.map(|r| r.map(plain)),
                })
                .collect(),
            smoke: race.smoke.as_ref().map(|smoke| SmokeFile {
                bone: text(&smoke.bone),
                stages: smoke
                    .stages
                    .iter()
                    .map(|(stage, file)| SmokeStage {
                        stage: *stage,
                        file: refs.refer(file.as_bytes()),
                    })
                    .collect(),
            }),
            motions: Vec::new(),
        }
    }

    /// The actor as the client's model script says it, its references
    /// made paths of the folder by `resolve`. Shapes and hair styles name
    /// their models and skins by path (no `PathName`).
    pub fn to_race(
        &self,
        resolve: &dyn Fn(&str) -> Result<String, String>,
    ) -> Result<RaceData, String> {
        let parts = |parts: &[PartFile]| -> Result<PartList, String> {
            Ok(PartList {
                path: RawStr::default(),
                entries: parts
                    .iter()
                    .map(|part| {
                        Ok(PartEntry {
                            index: part.index,
                            special_path: None,
                            model: raw(&resolve(&part.model)?),
                            skins: part
                                .skins
                                .iter()
                                .map(|skin| {
                                    Ok(Skin {
                                        source: raw(&skin.source),
                                        target: Some(raw(&resolve(&skin.target)?)),
                                    })
                                })
                                .collect::<Result<_, String>>()?,
                        })
                    })
                    .collect::<Result<_, String>>()?,
            })
        };
        Ok(RaceData {
            base_model: match &self.model {
                Some(model) => Some(raw(&resolve(model)?)),
                None => None,
            },
            attribute_file: None,
            motion_list: None,
            shapes: parts(&self.shapes)?,
            hairs: parts(&self.hairs)?,
            collisions: self
                .collisions
                .iter()
                .map(|c| Collision {
                    collision_type: c.kind,
                    attached: c.bone.is_some(),
                    bone: c.bone.as_deref().map(raw).unwrap_or_default(),
                    spheres: c.spheres.iter().map(sphere).collect(),
                })
                .collect(),
            effects: self
                .effects
                .iter()
                .map(|e| {
                    Ok(AttachedEffect {
                        attached: e.bone.is_some(),
                        bone: e.bone.as_deref().map(raw).unwrap_or_default(),
                        file: raw(&resolve(&e.file)?),
                        position: point_cm(e.at),
                        rotation: e.turn.map(|t| t.map(|v| v as f32)),
                    })
                })
                .collect::<Result<_, String>>()?,
            smoke: match &self.smoke {
                Some(smoke) => Some(Smoke {
                    bone: raw(&smoke.bone),
                    stages: smoke
                        .stages
                        .iter()
                        .map(|s| Ok((s.stage, raw(&resolve(&s.file)?))))
                        .collect::<Result<_, String>>()?,
                }),
                None => None,
            },
        })
    }

    /// The motion called `name` (case ignored), in `set` when one is given:
    /// the first that matches.
    pub fn motion(&self, set: Option<&str>, name: &str) -> Option<&MotionEntry> {
        self.motions.iter().find(|m| {
            m.name.eq_ignore_ascii_case(name)
                && set.is_none_or(|set| m.set.eq_ignore_ascii_case(set))
        })
    }
}

fn sphere_file(s: &Sphere) -> SphereFile {
    SphereFile {
        radius: metres(s.radius),
        at: point_m(s.position),
    }
}

fn sphere(s: &SphereFile) -> Sphere {
    Sphere {
        radius: centimetres(s.radius),
        position: point_cm(s.at),
    }
}

impl MotionFile {
    /// Parses a motion file.
    pub fn parse(text: &str) -> Result<Self, String> {
        from_toml(text, |file: &Self| file.format)
    }

    /// The motion of a motion script and its sound script.
    pub fn from_motion(
        data: &MotionData,
        sounds: Option<&SoundScript>,
        refs: &mut dyn References,
    ) -> Self {
        let mut sounds: Vec<SoundFile> = sounds
            .iter()
            .flat_map(|s| &s.sounds)
            .map(|s| SoundFile {
                time: plain(s.time),
                file: refs.refer(s.file.as_bytes()),
            })
            .collect();
        sounds.sort_by(|a, b| a.time.total_cmp(&b.time));
        Self {
            format: FORMAT,
            animation: refs.refer(data.motion_file.as_bytes()),
            duration: data.duration.map(plain),
            moves: data.accumulation.map(point_m),
            combo: data.combo.map(|c| ComboFile {
                pre_input: plain(c.pre_input),
                direct_input: plain(c.direct_input),
                input_limit: plain(c.input_limit),
                link: c.link.map(plain),
            }),
            attack: data.attacking.as_ref().map(|a| AttackFile {
                attack_type: a.attack_type,
                attacking_type: a.attacking_type,
                motion_type: a.motion_type,
                hitting_type: a.hitting_type,
                stiffen_time: plain(a.stiffen_time),
                invisible_time: plain(a.invisible_time),
                external_force: plain(a.external_force),
                hit_limit: a.hit_limit,
                hits: a
                    .hits
                    .iter()
                    .map(|h| HitFile {
                        start: plain(h.start),
                        end: plain(h.end),
                        bone: text(&h.bone),
                        weapon_length: metres(h.weapon_length),
                        positions: h
                            .positions
                            .iter()
                            .map(|row| {
                                let [a, b, c] = row.from.map(metres);
                                let [d, e, f] = row.to.map(metres);
                                [plain(row.time), a, b, c, d, e, f]
                            })
                            .collect(),
                    })
                    .collect(),
            }),
            events: data
                .events
                .iter()
                .map(|event| {
                    let time = plain(event.time);
                    let effect = |e: &Effect, refs: &mut dyn References| EffectEvent {
                        time,
                        file: refs.refer(e.file.as_bytes()),
                        at: point_m(e.position),
                        bone: e.bone.as_ref().map(text),
                        following: e.following,
                        independent: e.independent,
                        fishing: e.fishing,
                    };
                    match &event.kind {
                        EventKind::Effect(e) => EventFile::Effect(effect(e, refs)),
                        EventKind::SpotEffect(e) => EventFile::SpotEffect(effect(e, refs)),
                        EventKind::Shake(s) => EventFile::Shake {
                            time,
                            during: plain(s.during),
                            power: plain(s.power),
                            range: metres(s.range),
                        },
                        EventKind::Attack(a) => EventFile::Attack(AttackEventFile {
                            time,
                            during: plain(a.during),
                            enable_hit: a.enable_hit,
                            attack_type: a.attack_type,
                            hitting_type: a.hitting_type,
                            stiffen_time: plain(a.stiffen_time),
                            invisible_time: plain(a.invisible_time),
                            external_force: plain(a.external_force),
                            hit_limit: a.hit_limit,
                            collision_type: a.collision_type,
                            spheres: a.spheres.iter().map(sphere_file).collect(),
                        }),
                        EventKind::Fly(f) => EventFile::Fly {
                            time,
                            file: refs.refer(f.file.as_bytes()),
                            at: point_m(f.position),
                            bone: f.bone.as_ref().map(text),
                        },
                        EventKind::Other(number) => EventFile::Other {
                            time,
                            number: *number,
                        },
                    }
                })
                .collect(),
            sounds,
        }
    }

    /// The motion as the client's motion script and sound script say it,
    /// its references made paths of the folder by `resolve`.
    pub fn to_motion(
        &self,
        resolve: &dyn Fn(&str) -> Result<String, String>,
    ) -> Result<(MotionData, SoundScript), String> {
        let t = |v: f64| v as f32;
        let effect = |e: &EffectEvent| -> Result<(f32, Effect), String> {
            Ok((
                t(e.time),
                Effect {
                    file: raw(&resolve(&e.file)?),
                    position: point_cm(e.at),
                    bone: e.bone.as_deref().map(raw),
                    following: e.following,
                    independent: e.independent,
                    fishing: e.fishing,
                },
            ))
        };
        let events = self
            .events
            .iter()
            .map(|event| {
                let (time, kind) = match event {
                    EventFile::Effect(e) => {
                        let (time, e) = effect(e)?;
                        (time, EventKind::Effect(e))
                    }
                    EventFile::SpotEffect(e) => {
                        let (time, e) = effect(e)?;
                        (time, EventKind::SpotEffect(e))
                    }
                    EventFile::Shake {
                        time,
                        during,
                        power,
                        range,
                    } => (
                        t(*time),
                        EventKind::Shake(Shake {
                            during: t(*during),
                            power: t(*power),
                            range: centimetres(*range),
                        }),
                    ),
                    EventFile::Attack(a) => (
                        t(a.time),
                        EventKind::Attack(AttackEvent {
                            during: t(a.during),
                            enable_hit: a.enable_hit,
                            attack_type: a.attack_type,
                            hitting_type: a.hitting_type,
                            stiffen_time: t(a.stiffen_time),
                            invisible_time: t(a.invisible_time),
                            external_force: t(a.external_force),
                            hit_limit: a.hit_limit,
                            collision_type: a.collision_type,
                            spheres: a.spheres.iter().map(sphere).collect(),
                        }),
                    ),
                    EventFile::Fly {
                        time,
                        file,
                        at,
                        bone,
                    } => (
                        t(*time),
                        EventKind::Fly(Fly {
                            file: raw(&resolve(file)?),
                            position: point_cm(*at),
                            bone: bone.as_deref().map(raw),
                        }),
                    ),
                    EventFile::Other { time, number } => (t(*time), EventKind::Other(*number)),
                };
                Ok(MotionEvent { time, kind })
            })
            .collect::<Result<_, String>>()?;
        let data = MotionData {
            motion_file: raw(&resolve(&self.animation)?),
            duration: self.duration.map(t),
            accumulation: self.moves.map(point_cm),
            combo: self.combo.map(|c| ComboInput {
                pre_input: t(c.pre_input),
                direct_input: t(c.direct_input),
                input_limit: t(c.input_limit),
                link: c.link.map(t),
            }),
            attacking: self.attack.as_ref().map(|a| Attacking {
                attack_type: a.attack_type,
                attacking_type: a.attacking_type,
                motion_type: a.motion_type,
                hitting_type: a.hitting_type,
                stiffen_time: t(a.stiffen_time),
                invisible_time: t(a.invisible_time),
                external_force: t(a.external_force),
                hit_limit: a.hit_limit,
                hits: a
                    .hits
                    .iter()
                    .map(|h| Hit {
                        start: t(h.start),
                        end: t(h.end),
                        bone: raw(&h.bone),
                        weapon_length: centimetres(h.weapon_length),
                        positions: h
                            .positions
                            .iter()
                            .map(|[time, a, b, c, d, e, f]| HitPosition {
                                time: t(*time),
                                from: point_cm([*a, *b, *c]),
                                to: point_cm([*d, *e, *f]),
                            })
                            .collect(),
                    })
                    .collect(),
            }),
            events,
        };
        let sounds = SoundScript {
            sounds: self
                .sounds
                .iter()
                .map(|s| {
                    Ok(TimedSound {
                        time: t(s.time),
                        file: raw(&resolve(&s.file)?),
                    })
                })
                .collect::<Result<_, String>>()?,
        };
        Ok((data, sounds))
    }

    /// Every reference of the file, with what it names: the animation,
    /// effects, flying effects and sounds.
    pub fn references(&self) -> Vec<(&'static str, &str)> {
        let mut out = vec![("animation", self.animation.as_str())];
        for event in &self.events {
            match event {
                EventFile::Effect(e) | EventFile::SpotEffect(e) => out.push(("effect", &e.file)),
                EventFile::Fly { file, .. } => out.push(("flying effect", file)),
                EventFile::Shake { .. } | EventFile::Attack(_) | EventFile::Other { .. } => {}
            }
        }
        out.extend(self.sounds.iter().map(|s| ("sound", s.file.as_str())));
        out
    }
}

impl ActorFile {
    /// Every reference of the file, with what it names: models, textures,
    /// effects and motions.
    pub fn references(&self) -> Vec<(&'static str, &str)> {
        let mut out = Vec::new();
        out.extend(self.model.iter().map(|m| ("model", m.as_str())));
        for part in self.shapes.iter().chain(&self.hairs) {
            out.push(("model", &part.model));
            out.extend(part.skins.iter().map(|s| ("texture", s.target.as_str())));
        }
        out.extend(self.effects.iter().map(|e| ("effect", e.file.as_str())));
        out.extend(
            self.smoke
                .iter()
                .flat_map(|s| &s.stages)
                .map(|s| ("effect", s.file.as_str())),
        );
        out.extend(self.motions.iter().map(|m| ("motion", m.file.as_str())));
        out
    }
}

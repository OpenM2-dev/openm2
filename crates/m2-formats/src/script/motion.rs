//! Motion scripts (`.msa`, `ScriptType MotionData`) and the monsters'
//! motion lists (`motlist.txt`) (`docs/specs/formats/script.md`, "Motion
//! data" and "Motion lists").
//!
//! A motion script points at the GR2 file holding the animation, and says
//! when a player may chain the next attack, where the motion hits and what
//! happens at given times:
//!
//! ```text
//! ScriptType               MotionData
//! MotionFileName           "d:/ymir work/pc/warrior/onehand_sword/combo_01.gr2"
//! MotionDuration           1.000000
//! Group ComboInputData
//! {
//!     PreInputTime         0.167094
//!     DirectInputTime      0.533333
//!     InputLimitTime       0.602564
//! }
//! Group AttackingData
//! {
//!     AttackType           0
//!     HittingType          2
//!     StiffenTime          0.000000
//!     InvisibleTime        0.100000
//!     ExternalForce        8.000000
//!     MotionType           2
//!     HitDataCount         1
//!     Group HitData00
//!     {
//!         AttackingStartTime   0.192308
//!         AttackingEndTime     0.315385
//!         AttackingBone        "equip_right_hand"
//!         WeaponLength         130.000000
//!         List HitPosition
//!         {
//!             0.192308 -46.99 -13.97 146.68 -54.27 108.74 188.98
//!         }
//!     }
//! }
//! ```
//!
//! A motion list names a monster's motions, one per line: mode, name,
//! motion script (beside the list) and weight.
//!
//! ```text
//! GENERAL WAIT 00.msa 65
//! GENERAL WAIT1 00_1.msa 35
//! GENERAL RUN 03.msa 100
//! ```

use crate::error::FormatError;
use crate::raw_str::RawStr;
use crate::script::{self, MAX_LEN, Script, Token, check_type};

/// The `ScriptType` of a motion script.
pub const SCRIPT_TYPE: &str = "MotionData";

/// Most blocks one count may announce (`HitDataCount`,
/// `MotionEventDataCount`, `SphereDataCount`; the client's most is 10).
pub const MAX_BLOCKS: u32 = 256;

/// Name of the structure in errors.
const WHAT: &str = "motion data";

/// What a motion script says. Loops and skill cancelling stay in the
/// [`Script`] tree.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MotionData {
    /// `MotionFileName`: the GR2 file with the animation (a game path).
    pub motion_file: RawStr,
    /// `MotionDuration`: the length in seconds.
    pub duration: Option<f32>,
    /// `Accumulation`: how far the motion moves the character, x y z in
    /// centimetres, forward being −y (walking, running and attacking
    /// motions).
    pub accumulation: Option<[f32; 3]>,
    /// `Group ComboInputData`: when the next attack of a combo may be asked
    /// for (players' attacks).
    pub combo: Option<ComboInput>,
    /// `Group AttackingData`: when and where the motion hits.
    pub attacking: Option<Attacking>,
    /// `Group MotionEventData`: what happens at given times, in file order.
    pub events: Vec<MotionEvent>,
}

/// `Group ComboInputData`, in seconds into the motion. What the client does
/// with each is not confirmed; openm2's reading is in the spec.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ComboInput {
    /// `PreInputTime`: the earliest a press counts for the next attack.
    pub pre_input: f32,
    /// `DirectInputTime`: the earliest the next attack starts.
    pub direct_input: f32,
    /// `InputLimitTime`: the latest a press counts.
    pub input_limit: f32,
    /// `LinkTime` (59 of the client's 168).
    pub link: Option<f32>,
}

impl ComboInput {
    /// True if a press `time` seconds into the motion asks for the next
    /// attack: from `PreInputTime` to `InputLimitTime` (openm2's reading).
    /// The last motion of a chain has an empty window, or one only at its
    /// very end.
    pub fn accepts(&self, time: f32) -> bool {
        self.pre_input <= time && time <= self.input_limit
    }
}

/// `Group AttackingData`. The client writes it in two layouts: the older
/// (47 scripts) has one hit's keys in the group itself and `AttackingType`;
/// the newer (371) lists its hits in `HitData00`, … with `HitDataCount`, and
/// has `AttackType` and `MotionType`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Attacking {
    /// `AttackType` (newer layout): 0 or 1, meaning not known.
    pub attack_type: Option<u32>,
    /// `AttackingType` (older layout): 0, 1 or 2, meaning not known.
    /// Writing it chooses the older layout.
    pub attacking_type: Option<u32>,
    /// `MotionType` (newer layout): 0 to 3, meaning not known (2 in the
    /// players' combos).
    pub motion_type: Option<u32>,
    /// `HittingType`: 0, 1 or 2, meaning not known.
    pub hitting_type: u32,
    /// `StiffenTime`, seconds.
    pub stiffen_time: f32,
    /// `InvisibleTime`, seconds.
    pub invisible_time: f32,
    /// `ExternalForce`.
    pub external_force: f32,
    /// `HitLimitCount` (newer layout, most scripts).
    pub hit_limit: Option<u32>,
    /// The hits: one in the older layout, `HitDataCount` in the newer (0 to
    /// 3 in the client).
    pub hits: Vec<Hit>,
}

/// One hit of an attack: while it lasts, the weapon (or a bone) sweeps
/// through the points of `positions`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Hit {
    /// `AttackingStartTime`, seconds into the motion.
    pub start: f32,
    /// `AttackingEndTime`, seconds into the motion. A hit with no time
    /// (start and end 0) does nothing: such motions hit with an attack
    /// event instead.
    pub end: f32,
    /// `AttackingBone`: the bone that strikes (`equip_right_hand`), empty
    /// for none.
    pub bone: RawStr,
    /// `WeaponLength`, centimetres: 0 for a fist or a claw.
    pub weapon_length: f32,
    /// `List HitPosition`: where the strike is over time. Rows that are not
    /// seven numbers are left out (one in the client has text run into a
    /// number).
    pub positions: Vec<HitPosition>,
}

/// One row of `List HitPosition`: seven numbers.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct HitPosition {
    /// Seconds into the motion; the rows are 1/120 s apart.
    pub time: f32,
    /// The first point, centimetres in the model's space (forward is −y,
    /// up +z): where the striking bone is.
    pub from: [f32; 3],
    /// The second point: `WeaponLength` from the first along the weapon,
    /// its tip. The same as `from` without a weapon.
    pub to: [f32; 3],
}

impl Hit {
    /// True if the hit has a time to strike in.
    pub fn strikes(&self) -> bool {
        self.end > self.start
    }

    /// Where the strike is `time` seconds into the motion: the line from
    /// the bone to the weapon's tip, between the two rows around that time
    /// (the first or last row outside them). `None` outside the hit's time
    /// or without rows.
    pub fn segment_at(&self, time: f32) -> Option<([f32; 3], [f32; 3])> {
        if !(self.start <= time && time <= self.end) {
            return None;
        }
        let after = self.positions.iter().position(|row| row.time >= time);
        let (before, after) = match after {
            Some(0) => (self.positions.first()?, self.positions.first()?),
            Some(n) => (self.positions.get(n - 1)?, self.positions.get(n)?),
            None => (self.positions.last()?, self.positions.last()?),
        };
        let span = after.time - before.time;
        let t = if span > 0.0 {
            ((time - before.time) / span).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let mix = |a: [f32; 3], b: [f32; 3]| {
            [
                a[0] + (b[0] - a[0]) * t,
                a[1] + (b[1] - a[1]) * t,
                a[2] + (b[2] - a[2]) * t,
            ]
        };
        Some((mix(before.from, after.from), mix(before.to, after.to)))
    }
}

/// One `Event##` block of `Group MotionEventData`.
#[derive(Debug, Clone, PartialEq)]
pub struct MotionEvent {
    /// `StartingTime`, seconds into the motion.
    pub time: f32,
    /// `MotionEventType` and what goes with it.
    pub kind: EventKind,
}

/// What a motion event does, by its `MotionEventType`.
#[derive(Debug, Clone, PartialEq)]
pub enum EventKind {
    /// Type 1: an effect, on a bone or where the character is.
    Effect(Effect),
    /// Type 2: `DuringTime`, `Power` and `AffectingRange`. What it does is
    /// openm2's guess: it comes just before the ground-stomping hits
    /// (warriors' `combo_04` and `combo_07`), so the screen shaking.
    Shake(Shake),
    /// Type 4: spheres around the character that hit what they touch for a
    /// while.
    Attack(AttackEvent),
    /// Type 6: something that flies off: an arrow, a spell
    /// (`FlyFileName`, a `.msf` script).
    Fly(Fly),
    /// Type 10: an effect that does not attach to a bone.
    SpotEffect(Effect),
    /// Any other type, kept by number. Types 7, 8 and 9 (37 each, in
    /// skills and bosses' special attacks) carry only a time.
    Other(u32),
}

/// An effect event (types 1 and 10).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Effect {
    /// `EffectFileName`: an effect script (`.mse`).
    pub file: RawStr,
    /// `EffectPosition`, centimetres from the bone or the character.
    pub position: [f32; 3],
    /// `AttachingBoneName`, when `AttachingEnable` is 1.
    pub bone: Option<RawStr>,
    /// `FollowingEnable`: the effect moves with the character.
    pub following: bool,
    /// `IndependentFlag` (type 1, most of them).
    pub independent: Option<bool>,
    /// `FishingEffectFlag` (type 10, most of them).
    pub fishing: Option<bool>,
}

/// A type 2 event.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Shake {
    /// `DuringTime`, seconds.
    pub during: f32,
    /// `Power`.
    pub power: f32,
    /// `AffectingRange`, centimetres.
    pub range: f32,
}

/// An attack event (type 4): spheres that hit from `StartingTime` for
/// `during` seconds.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AttackEvent {
    /// `DuringTime`, seconds.
    pub during: f32,
    /// `EnableHitProcess` (most of them).
    pub enable_hit: Option<bool>,
    /// `AttackType`: 0 or 1, meaning not known.
    pub attack_type: Option<u32>,
    /// `HittingType`: 1 or 2, meaning not known.
    pub hitting_type: u32,
    /// `StiffenTime`, seconds.
    pub stiffen_time: f32,
    /// `InvisibleTime`, seconds.
    pub invisible_time: f32,
    /// `ExternalForce`.
    pub external_force: f32,
    /// `HitLimitCount` (most of them).
    pub hit_limit: Option<u32>,
    /// `CollisionType`: 4 in all but 4 events, which have 0.
    pub collision_type: u32,
    /// `SphereData00`, … (`SphereDataCount`).
    pub spheres: Vec<Sphere>,
}

/// A sphere of an attack event.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Sphere {
    /// `Radius`, centimetres.
    pub radius: f32,
    /// `Position`, centimetres in the model's space (forward is −y).
    pub position: [f32; 3],
}

/// A fly event (type 6).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Fly {
    /// `FlyFileName` (55 scripts write `flyfilename`): a flying effect
    /// script (`.msf`).
    pub file: RawStr,
    /// `FlyPosition`, centimetres from the bone or the character.
    pub position: [f32; 3],
    /// `AttachingBoneName`, when `AttachingEnable` is 1.
    pub bone: Option<RawStr>,
}

impl MotionData {
    /// Parses a motion script.
    pub fn parse(bytes: &[u8]) -> Result<Self, FormatError> {
        Self::read(&script::parse(bytes)?)
    }

    /// Reads a parsed script. Fails if its `ScriptType` is not `MotionData`,
    /// it has no `MotionFileName`, or a block lacks a key the client always
    /// writes.
    pub fn read(script: &Script) -> Result<Self, FormatError> {
        check_type(script, SCRIPT_TYPE, WHAT)?;
        Ok(Self {
            motion_file: script.text("MotionFileName", WHAT)?.ok_or_else(|| {
                FormatError::Damaged {
                    what: WHAT,
                    detail: "no MotionFileName".to_owned(),
                }
            })?,
            duration: script.number("MotionDuration", WHAT)?,
            accumulation: script.numbers("Accumulation", WHAT)?,
            combo: match script.group("ComboInputData") {
                Some(group) => Some(ComboInput::read(group)?),
                None => None,
            },
            attacking: match script.group("AttackingData") {
                Some(group) => Some(Attacking::read(group)?),
                None => None,
            },
            events: match script.group("MotionEventData") {
                Some(group) => counted(group, "MotionEventDataCount", "Event", "MotionEventData")?
                    .into_iter()
                    .map(|(name, block)| MotionEvent::read(block, &name))
                    .collect::<Result<_, _>>()?,
                None => Vec::new(),
            },
        })
    }

    /// The script for this data, for test data. Fails for attack data in
    /// the older layout (`attacking_type`) without exactly one hit.
    pub fn to_script(&self) -> Result<Script, FormatError> {
        let mut script = Script::default();
        script.push_values("ScriptType", vec![Token::bare(SCRIPT_TYPE)]);
        script.push_values(
            "MotionFileName",
            vec![Token::quoted(self.motion_file.clone())],
        );
        if let Some(duration) = self.duration {
            script.push_values("MotionDuration", vec![number(duration)]);
        }
        if let Some(accumulation) = self.accumulation {
            script.push_values("Accumulation", accumulation.map(number).to_vec());
        }
        if let Some(combo) = self.combo {
            script.push_group("ComboInputData", combo.to_script());
        }
        if let Some(attacking) = &self.attacking {
            script.push_group("AttackingData", attacking.to_script()?);
        }
        if !self.events.is_empty() {
            let mut group = Script::default();
            group.push_values("MotionEventDataCount", vec![whole(self.events.len())]);
            for (n, event) in self.events.iter().enumerate() {
                group.push_group(&format!("Event{n:02}"), event.to_script());
            }
            script.push_group("MotionEventData", group);
        }
        Ok(script)
    }

    /// Writes the script, for test data.
    pub fn write(&self) -> Result<Vec<u8>, FormatError> {
        script::write(&self.to_script()?)
    }

    /// The hits that strike, then the attack events, as (start, end) in
    /// seconds into the motion, in that order.
    pub fn hit_times(&self) -> Vec<(f32, f32)> {
        let hits = self
            .attacking
            .iter()
            .flat_map(|a| &a.hits)
            .filter(|hit| hit.strikes())
            .map(|hit| (hit.start, hit.end));
        let events = self.events.iter().filter_map(|event| match &event.kind {
            EventKind::Attack(attack) => Some((event.time, event.time + attack.during)),
            _ => None,
        });
        hits.chain(events).collect()
    }

    /// How fast the motion moves the character forward, in centimetres a
    /// second: its `Accumulation` along the ground over its
    /// `MotionDuration`, or over `animation_duration` (its GR2 animation's
    /// length) when the script gives none. `None` for a motion that moves
    /// nothing or lasts no time.
    pub fn forward_speed(&self, animation_duration: Option<f32>) -> Option<f32> {
        let duration = self.duration.or(animation_duration)?;
        let [x, y, _] = self.accumulation?;
        let distance = x.hypot(y);
        (distance > 0.0 && duration > 0.0).then(|| distance / duration)
    }

    /// How far from the character the motion hits, in centimetres along the
    /// ground: the farthest point of its hit positions, or edge of its
    /// attack spheres. `None` for a motion that does not hit.
    pub fn reach(&self) -> Option<f32> {
        let flat = |p: [f32; 3]| p[0].hypot(p[1]);
        let hits = self
            .attacking
            .iter()
            .flat_map(|a| &a.hits)
            .filter(|hit| hit.strikes())
            .flat_map(|hit| &hit.positions)
            .flat_map(|row| [flat(row.from), flat(row.to)]);
        let spheres = self
            .events
            .iter()
            .filter_map(|event| match &event.kind {
                EventKind::Attack(attack) => Some(&attack.spheres),
                _ => None,
            })
            .flatten()
            .map(|sphere| flat(sphere.position) + sphere.radius);
        hits.chain(spheres).reduce(f32::max)
    }
}

impl ComboInput {
    fn read(group: &Script) -> Result<Self, FormatError> {
        const BLOCK: &str = "ComboInputData";
        Ok(Self {
            pre_input: required(group, "PreInputTime", BLOCK)?,
            direct_input: required(group, "DirectInputTime", BLOCK)?,
            input_limit: required(group, "InputLimitTime", BLOCK)?,
            link: group.number("LinkTime", WHAT)?,
        })
    }

    fn to_script(self) -> Script {
        let mut group = Script::default();
        group.push_values("PreInputTime", vec![number(self.pre_input)]);
        group.push_values("DirectInputTime", vec![number(self.direct_input)]);
        group.push_values("InputLimitTime", vec![number(self.input_limit)]);
        if let Some(link) = self.link {
            group.push_values("LinkTime", vec![number(link)]);
        }
        group
    }
}

impl Attacking {
    /// Reads either layout: the newer when the group has `HitDataCount`.
    fn read(group: &Script) -> Result<Self, FormatError> {
        const BLOCK: &str = "AttackingData";
        let hits = if group.get("HitDataCount").is_some() {
            counted(group, "HitDataCount", "HitData", BLOCK)?
                .into_iter()
                .map(|(name, block)| Hit::read(block, &format!("{BLOCK}.{name}")))
                .collect::<Result<_, _>>()?
        } else {
            vec![Hit::read(group, BLOCK)?]
        };
        Ok(Self {
            attack_type: group.number("AttackType", WHAT)?,
            attacking_type: group.number("AttackingType", WHAT)?,
            motion_type: group.number("MotionType", WHAT)?,
            hitting_type: required(group, "HittingType", BLOCK)?,
            stiffen_time: required(group, "StiffenTime", BLOCK)?,
            invisible_time: required(group, "InvisibleTime", BLOCK)?,
            external_force: required(group, "ExternalForce", BLOCK)?,
            hit_limit: group.number("HitLimitCount", WHAT)?,
            hits,
        })
    }

    fn to_script(&self) -> Result<Script, FormatError> {
        let mut group = Script::default();
        let older = self.attacking_type.is_some();
        if let Some(value) = self.attacking_type {
            group.push_values("AttackingType", vec![whole(value)]);
        }
        if let Some(value) = self.attack_type {
            group.push_values("AttackType", vec![whole(value)]);
        }
        group.push_values("HittingType", vec![whole(self.hitting_type)]);
        group.push_values("StiffenTime", vec![number(self.stiffen_time)]);
        group.push_values("InvisibleTime", vec![number(self.invisible_time)]);
        group.push_values("ExternalForce", vec![number(self.external_force)]);
        if let Some(value) = self.hit_limit {
            group.push_values("HitLimitCount", vec![whole(value)]);
        }
        if let Some(value) = self.motion_type {
            group.push_values("MotionType", vec![whole(value)]);
        }
        if older {
            let [hit] = self.hits.as_slice() else {
                return Err(FormatError::Unsupported {
                    what: WHAT,
                    detail: format!(
                        "the older attack layout (AttackingType) holds one hit, not {}",
                        self.hits.len()
                    ),
                });
            };
            hit.write_into(&mut group);
        } else {
            group.push_values("HitDataCount", vec![whole(self.hits.len())]);
            for (n, hit) in self.hits.iter().enumerate() {
                let mut block = Script::default();
                hit.write_into(&mut block);
                group.push_group(&format!("HitData{n:02}"), block);
            }
        }
        Ok(group)
    }
}

impl Hit {
    fn read(group: &Script, path: &str) -> Result<Self, FormatError> {
        let positions = group
            .list("HitPosition")
            .unwrap_or_default()
            .iter()
            .filter_map(|row| row_numbers::<7>(row))
            .map(|[time, x1, y1, z1, x2, y2, z2]| HitPosition {
                time,
                from: [x1, y1, z1],
                to: [x2, y2, z2],
            })
            .collect();
        Ok(Self {
            start: required(group, "AttackingStartTime", path)?,
            end: required(group, "AttackingEndTime", path)?,
            bone: group.text("AttackingBone", WHAT)?.unwrap_or_default(),
            weapon_length: group.number("WeaponLength", WHAT)?.unwrap_or_default(),
            positions,
        })
    }

    fn write_into(&self, group: &mut Script) {
        group.push_values("AttackingStartTime", vec![number(self.start)]);
        group.push_values("AttackingEndTime", vec![number(self.end)]);
        group.push_values("AttackingBone", vec![Token::quoted(self.bone.clone())]);
        group.push_values("WeaponLength", vec![number(self.weapon_length)]);
        let rows = self
            .positions
            .iter()
            .map(|row| {
                [row.time]
                    .into_iter()
                    .chain(row.from)
                    .chain(row.to)
                    .map(number)
                    .collect()
            })
            .collect();
        group.push_list("HitPosition", rows);
    }
}

impl MotionEvent {
    fn read(block: &Script, name: &str) -> Result<Self, FormatError> {
        let path = format!("MotionEventData.{name}");
        let path = path.as_str();
        let kind: u32 = required(block, "MotionEventType", path)?;
        let effect = || -> Result<Effect, FormatError> {
            Ok(Effect {
                file: block.text("EffectFileName", WHAT)?.unwrap_or_default(),
                position: block.numbers("EffectPosition", WHAT)?.unwrap_or_default(),
                bone: attached_bone(block)?,
                following: flag(block, "FollowingEnable")?.unwrap_or(false),
                independent: flag(block, "IndependentFlag")?,
                fishing: flag(block, "FishingEffectFlag")?,
            })
        };
        Ok(Self {
            time: required(block, "StartingTime", path)?,
            kind: match kind {
                1 => EventKind::Effect(effect()?),
                2 => EventKind::Shake(Shake {
                    during: required(block, "DuringTime", path)?,
                    power: required(block, "Power", path)?,
                    range: required(block, "AffectingRange", path)?,
                }),
                4 => EventKind::Attack(AttackEvent {
                    during: required(block, "DuringTime", path)?,
                    enable_hit: flag(block, "EnableHitProcess")?,
                    attack_type: block.number("AttackType", WHAT)?,
                    hitting_type: required(block, "HittingType", path)?,
                    stiffen_time: required(block, "StiffenTime", path)?,
                    invisible_time: required(block, "InvisibleTime", path)?,
                    external_force: required(block, "ExternalForce", path)?,
                    hit_limit: block.number("HitLimitCount", WHAT)?,
                    collision_type: block.number("CollisionType", WHAT)?.unwrap_or_default(),
                    spheres: counted(block, "SphereDataCount", "SphereData", path)?
                        .into_iter()
                        .map(|(sphere, data)| {
                            let path = format!("{path}.{sphere}");
                            Ok(Sphere {
                                radius: required(data, "Radius", &path)?,
                                position: data.numbers("Position", WHAT)?.unwrap_or_default(),
                            })
                        })
                        .collect::<Result<_, FormatError>>()?,
                }),
                6 => EventKind::Fly(Fly {
                    file: block.text("FlyFileName", WHAT)?.unwrap_or_default(),
                    position: block.numbers("FlyPosition", WHAT)?.unwrap_or_default(),
                    bone: attached_bone(block)?,
                }),
                10 => EventKind::SpotEffect(effect()?),
                other => EventKind::Other(other),
            },
        })
    }

    fn to_script(&self) -> Script {
        let mut block = Script::default();
        let kind = match &self.kind {
            EventKind::Effect(_) => 1,
            EventKind::Shake(_) => 2,
            EventKind::Attack(_) => 4,
            EventKind::Fly(_) => 6,
            EventKind::SpotEffect(_) => 10,
            EventKind::Other(kind) => *kind,
        };
        block.push_values("MotionEventType", vec![whole(kind)]);
        block.push_values("StartingTime", vec![number(self.time)]);
        match &self.kind {
            EventKind::Effect(effect) | EventKind::SpotEffect(effect) => {
                if let Some(independent) = effect.independent {
                    block.push_values("IndependentFlag", vec![flag_token(independent)]);
                }
                // Type 10 has no bone keys unless it names a bone.
                if matches!(self.kind, EventKind::Effect(_)) || effect.bone.is_some() {
                    push_bone(&mut block, effect.bone.as_ref());
                }
                block.push_values("FollowingEnable", vec![flag_token(effect.following)]);
                block.push_values("EffectFileName", vec![Token::quoted(effect.file.clone())]);
                block.push_values("EffectPosition", effect.position.map(number).to_vec());
                if let Some(fishing) = effect.fishing {
                    block.push_values("FishingEffectFlag", vec![flag_token(fishing)]);
                }
            }
            EventKind::Shake(shake) => {
                block.push_values("DuringTime", vec![number(shake.during)]);
                block.push_values("Power", vec![number(shake.power)]);
                block.push_values("AffectingRange", vec![number(shake.range)]);
            }
            EventKind::Attack(attack) => {
                block.push_values("DuringTime", vec![number(attack.during)]);
                if let Some(enable) = attack.enable_hit {
                    block.push_values("EnableHitProcess", vec![flag_token(enable)]);
                }
                if let Some(value) = attack.attack_type {
                    block.push_values("AttackType", vec![whole(value)]);
                }
                block.push_values("HittingType", vec![whole(attack.hitting_type)]);
                block.push_values("StiffenTime", vec![number(attack.stiffen_time)]);
                block.push_values("InvisibleTime", vec![number(attack.invisible_time)]);
                block.push_values("ExternalForce", vec![number(attack.external_force)]);
                if let Some(value) = attack.hit_limit {
                    block.push_values("HitLimitCount", vec![whole(value)]);
                }
                block.push_values("CollisionType", vec![whole(attack.collision_type)]);
                block.push_values("SphereDataCount", vec![whole(attack.spheres.len())]);
                for (n, sphere) in attack.spheres.iter().enumerate() {
                    let mut data = Script::default();
                    data.push_values("Radius", vec![number(sphere.radius)]);
                    data.push_values("Position", sphere.position.map(number).to_vec());
                    block.push_group(&format!("SphereData{n:02}"), data);
                }
            }
            EventKind::Fly(fly) => {
                push_bone(&mut block, fly.bone.as_ref());
                block.push_values("FlyFileName", vec![Token::quoted(fly.file.clone())]);
                block.push_values("FlyPosition", fly.position.map(number).to_vec());
            }
            EventKind::Other(_) => {}
        }
        block
    }
}

/// The blocks `{prefix}00`, `{prefix}01`, … up to the count in `count_key`
/// (none without it), with their names. A block missing below the count is
/// an error; blocks past it are ignored.
fn counted<'a>(
    group: &'a Script,
    count_key: &str,
    prefix: &str,
    path: &str,
) -> Result<Vec<(String, &'a Script)>, FormatError> {
    let count: u32 = group.number(count_key, WHAT)?.unwrap_or(0);
    if count > MAX_BLOCKS {
        return Err(FormatError::TooLarge {
            what: WHAT,
            size: u64::from(count),
            limit: u64::from(MAX_BLOCKS),
        });
    }
    (0..count)
        .map(|n| {
            let name = format!("{prefix}{n:02}");
            let block = group.group(&name).ok_or_else(|| FormatError::Damaged {
                what: WHAT,
                detail: format!("{path}: {count_key} is {count}, but there is no block {name}"),
            })?;
            Ok((name, block))
        })
        .collect()
}

/// A number the client always writes in this block.
fn required<T: std::str::FromStr>(group: &Script, key: &str, path: &str) -> Result<T, FormatError> {
    group
        .number(key, WHAT)?
        .ok_or_else(|| FormatError::Damaged {
            what: WHAT,
            detail: format!("{path} has no {key}"),
        })
}

/// A 0/1 value; the client writes some as `0.000000`.
fn flag(group: &Script, key: &str) -> Result<Option<bool>, FormatError> {
    Ok(group.number::<f32>(key, WHAT)?.map(|n| n != 0.0))
}

/// `AttachingBoneName` when `AttachingEnable` is set.
fn attached_bone(block: &Script) -> Result<Option<RawStr>, FormatError> {
    if !flag(block, "AttachingEnable")?.unwrap_or(false) {
        return Ok(None);
    }
    Ok(Some(
        block.text("AttachingBoneName", WHAT)?.unwrap_or_default(),
    ))
}

fn push_bone(block: &mut Script, bone: Option<&RawStr>) {
    block.push_values("AttachingEnable", vec![flag_token(bone.is_some())]);
    block.push_values(
        "AttachingBoneName",
        vec![Token::quoted(bone.cloned().unwrap_or_default())],
    );
}

/// The row as exactly `N` numbers.
fn row_numbers<const N: usize>(row: &[Token]) -> Option<[f32; N]> {
    if row.len() != N {
        return None;
    }
    let mut out = [0.0; N];
    for (slot, token) in out.iter_mut().zip(row) {
        *slot = std::str::from_utf8(token.text.as_bytes())
            .ok()?
            .parse()
            .ok()?;
    }
    Some(out)
}

/// A number written like the client's scripts: six decimals.
fn number(value: f32) -> Token {
    Token::bare(format!("{value:.6}").as_str())
}

/// A whole number.
fn whole(value: impl ToString) -> Token {
    Token::bare(value.to_string().as_str())
}

fn flag_token(on: bool) -> Token {
    Token::bare(if on { "1" } else { "0" })
}

/// One line of a motion list.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MotionListEntry {
    /// The mode the motion belongs to: `GENERAL` for monsters.
    pub mode: RawStr,
    /// The motion's name: `WAIT`, `RUN`, `NORMAL_ATTACK`, …
    pub name: RawStr,
    /// The motion script, a file name beside the list.
    pub file: RawStr,
    /// The weight. A motion and its numbered variants usually add up to 100
    /// (`WAIT 65`, `WAIT1 35`), but not always; how the client uses it is
    /// not confirmed.
    pub weight: u32,
}

impl MotionListEntry {
    /// The game path of the motion script, for a list at `list_path`: the
    /// list's folder followed by the file name.
    pub fn script_path(&self, list_path: &[u8]) -> Vec<u8> {
        let folder_end = list_path
            .iter()
            .rposition(|b| matches!(b, b'/' | b'\\'))
            .map_or(0, |at| at + 1);
        let folder = list_path.get(..folder_end).unwrap_or_default();
        [folder, self.file.as_bytes()].concat()
    }

    /// True if this is motion `name` or one of its numbered variants
    /// (`NORMAL_ATTACK`, `NORMAL_ATTACK1`), ignoring case.
    pub fn is_variant_of(&self, name: &str) -> bool {
        let own = self.name.as_bytes();
        own.get(..name.len())
            .is_some_and(|start| start.eq_ignore_ascii_case(name.as_bytes()))
            && own
                .get(name.len()..)
                .is_some_and(|rest| rest.iter().all(u8::is_ascii_digit))
    }
}

/// The entries of motion `name` and its numbered variants, in list order.
pub fn variants<'a>(list: &'a [MotionListEntry], name: &str) -> Vec<&'a MotionListEntry> {
    list.iter().filter(|e| e.is_variant_of(name)).collect()
}

/// Parses a motion list: one motion per line, four values separated by
/// spaces or tabs (mode, name, file, weight). Blank lines are skipped.
pub fn parse_list(bytes: &[u8]) -> Result<Vec<MotionListEntry>, FormatError> {
    const WHAT: &str = "motion list";
    if bytes.len() > MAX_LEN {
        return Err(FormatError::TooLarge {
            what: WHAT,
            size: bytes.len() as u64,
            limit: MAX_LEN as u64,
        });
    }
    script::token_lines(bytes)?
        .into_iter()
        .map(|(line, tokens)| {
            let damaged = |detail: String| FormatError::Damaged {
                what: WHAT,
                detail: format!("line {line}: {detail}"),
            };
            let [mode, name, file, weight] = <[Token; 4]>::try_from(tokens).map_err(|tokens| {
                damaged(format!(
                    "expected mode, name, file and weight, found {} values",
                    tokens.len()
                ))
            })?;
            let weight = std::str::from_utf8(weight.text.as_bytes())
                .ok()
                .and_then(|text| text.parse().ok())
                .ok_or_else(|| damaged(format!("the weight `{}` is not a number", weight.text)))?;
            Ok(MotionListEntry {
                mode: mode.text,
                name: name.text,
                file: file.text,
                weight,
            })
        })
        .collect()
}

/// Writes a motion list, for test data. Fails for a value that is empty or
/// holds whitespace.
pub fn write_list(entries: &[MotionListEntry]) -> Result<Vec<u8>, FormatError> {
    let mut out = Vec::new();
    for entry in entries {
        for field in [&entry.mode, &entry.name, &entry.file] {
            let text = field.as_bytes();
            if text.is_empty() || text.iter().any(|b| b.is_ascii_whitespace() || *b == b'"') {
                return Err(FormatError::Unsupported {
                    what: "motion list",
                    detail: format!("`{field}` is not one word"),
                });
            }
            out.extend_from_slice(text);
            out.push(b' ');
        }
        out.extend_from_slice(entry.weight.to_string().as_bytes());
        out.extend_from_slice(b"\r\n");
    }
    Ok(out)
}

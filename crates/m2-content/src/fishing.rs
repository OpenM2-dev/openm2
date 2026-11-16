//! Fishing (`docs/specs/server/content.md`, "Fishing"):
//! `fishing.toml`, the operator's file, which the import never writes.
//! The poles and the bait are the item table's (`Item::rod`, `Item::bait`).

use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;

fn one_u8() -> u8 {
    1
}

fn default_bite() -> [u32; 2] {
    [10, 40]
}

fn default_react() -> f64 {
    6.0
}

fn default_reach() -> f64 {
    6.0
}

/// How casts go (`[rules]`), each with a default.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FishingRules {
    /// The shortest and the longest wait from a cast to the bite, in whole
    /// seconds; each wait is one of them or a whole second between.
    #[serde(default = "default_bite")]
    pub bite: [u32; 2],
    /// Seconds a bite lasts: a reel-in later finds the fish gone.
    #[serde(default = "default_react")]
    pub react: f64,
    /// Metres from the fisher to the float.
    #[serde(default = "default_reach")]
    pub reach: f64,
}

impl Default for FishingRules {
    fn default() -> Self {
        Self {
            bite: default_bite(),
            react: default_react(),
            reach: default_reach(),
        }
    }
}

impl FishingRules {
    /// The bite's time, in milliseconds.
    pub fn react_ms(&self) -> u64 {
        (self.react * 1000.0).round() as u64
    }

    /// The reach, in centimetres.
    pub fn reach_cm(&self) -> i32 {
        (self.reach * 100.0).round() as i32
    }
}

/// An NPC that betters full fishing poles (`[[fisher]]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fisher {
    /// The NPC's monster number.
    pub npc: u32,
}

/// How the chance of holding a fish goes with the time taken to reel in
/// after its bite (`[[timing]]`).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Timing {
    /// Its name, which fish give.
    pub name: String,
    /// Points of seconds after the bite and percent, in order of time.
    pub points: Vec<[f64; 2]>,
}

impl Timing {
    /// The percent chance a reel-in `seconds` after the bite holds the
    /// fish: on straight lines between the points, the first point's
    /// before them and the last's after.
    pub fn chance(&self, seconds: f64) -> f64 {
        let Some(&[first_time, first]) = self.points.first() else {
            return 0.0;
        };
        if seconds <= first_time {
            return first;
        }
        for pair in self.points.windows(2) {
            let [[t0, c0], [t1, c1]] = pair else {
                continue;
            };
            let ([t0, c0], [t1, c1]) = ([*t0, *c0], [*t1, *c1]);
            if seconds <= t1 {
                if t1 <= t0 {
                    return c1;
                }
                return c0 + (c1 - c0) * (seconds - t0) / (t1 - t0);
            }
        }
        self.points.last().map_or(0.0, |point| point[1])
    }
}

/// Something that may be caught (`[[fish]]`): a fish, or any item.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fish {
    /// The item caught.
    pub item: u32,
    /// How hard it is to hold: the pole's level plus the bait's strength
    /// over this is the chance, up to sure.
    pub difficulty: u32,
    /// The name of its `[[timing]]`.
    pub timing: String,
}

/// What bites on some maps (`[[water]]`).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Water {
    /// The maps, by name.
    pub maps: Vec<String>,
    /// The weight of an empty hook.
    #[serde(default)]
    pub nothing: u32,
    /// The fish and their weights.
    #[serde(default)]
    pub catch: Vec<Catch>,
}

impl Water {
    /// All the weights together, the empty hook's too.
    pub fn total(&self) -> u64 {
        u64::from(self.nothing)
            + self
                .catch
                .iter()
                .map(|catch| u64::from(catch.weight))
                .sum::<u64>()
    }

    /// What a roll from 0 to below [`Water::total`] picks: the fish of the
    /// weight the roll falls in, `None` for the empty hook (whose weight
    /// comes first).
    pub fn pick(&self, roll: u64) -> Option<u32> {
        let mut left = roll.checked_sub(u64::from(self.nothing))?;
        for catch in &self.catch {
            match left.checked_sub(u64::from(catch.weight)) {
                Some(rest) => left = rest,
                None => return Some(catch.item),
            }
        }
        None
    }
}

/// A fish with its weight among a water's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Catch {
    /// The item of `[[fish]]`.
    pub item: u32,
    /// Its weight.
    pub weight: u32,
}

/// An item that, used, gives one of others by chance (`[[open]]`): a live
/// fish, a clam.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Open {
    /// The item used.
    pub item: u32,
    /// What it may give; what the chances leave is nothing.
    #[serde(default)]
    pub gives: Vec<Gift>,
}

impl Open {
    /// What a roll from 0 to below 100 picks: the gift whose chance it
    /// falls in, `None` for nothing.
    pub fn pick(&self, roll: f64) -> Option<&Gift> {
        let mut left = roll;
        for gift in &self.gives {
            if left < gift.chance {
                return Some(gift);
            }
            left -= gift.chance;
        }
        None
    }
}

/// One of what an opened item may give.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Gift {
    /// The item.
    pub item: u32,
    /// How many.
    #[serde(default = "one_u8")]
    pub count: u8,
    /// The percent chance.
    pub chance: f64,
}

/// The fire an item lights (`[fire]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fire {
    /// The item that lights it.
    pub item: u32,
    /// The fire, an NPC.
    pub npc: u32,
    /// How many seconds it burns.
    pub seconds: u32,
}

/// An item a fire turns into another (`[[grill]]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Grill {
    /// The item given to the fire.
    pub item: u32,
    /// What it becomes, as many.
    pub into: u32,
}

/// What `fishing.toml` holds, before it is checked: empty without the file.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FishingParts {
    /// `[rules]`, if written.
    pub rules: Option<FishingRules>,
    /// The fishers.
    pub fishers: Vec<Fisher>,
    /// The timings.
    pub timings: Vec<Timing>,
    /// What may be caught.
    pub fish: Vec<Fish>,
    /// The waters.
    pub waters: Vec<Water>,
    /// The items that open.
    pub opens: Vec<Open>,
    /// The fire, if any.
    pub fire: Option<Fire>,
    /// What fires grill.
    pub grills: Vec<Grill>,
}

/// The checked fishing ([`crate::Content::fishing`]): empty without
/// `fishing.toml`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Fishing {
    pub(crate) rules: Option<FishingRules>,
    pub(crate) fishers: BTreeSet<u32>,
    pub(crate) timings: BTreeMap<String, Timing>,
    pub(crate) fish: BTreeMap<u32, Fish>,
    /// In the order written.
    pub(crate) waters: Vec<Water>,
    /// Each map's water, by map name: its index in `waters`.
    pub(crate) water_of: BTreeMap<String, usize>,
    pub(crate) opens: BTreeMap<u32, Open>,
    pub(crate) fire: Option<Fire>,
    pub(crate) grills: BTreeMap<u32, Grill>,
}

impl Fishing {
    /// Whether there is nothing: no rules, fisher, timing, fish, water,
    /// opening, fire or grilling.
    pub fn is_empty(&self) -> bool {
        self.rules.is_none()
            && self.fishers.is_empty()
            && self.timings.is_empty()
            && self.fish.is_empty()
            && self.waters.is_empty()
            && self.opens.is_empty()
            && self.fire.is_none()
            && self.grills.is_empty()
    }

    /// How casts go: `[rules]`, or the defaults.
    pub fn rules(&self) -> FishingRules {
        self.rules.unwrap_or_default()
    }

    /// `[rules]` as written, if it was.
    pub fn written_rules(&self) -> Option<&FishingRules> {
        self.rules.as_ref()
    }

    /// Whether NPCs numbered `npc` better fishing poles.
    pub fn is_fisher(&self, npc: u32) -> bool {
        self.fishers.contains(&npc)
    }

    /// The fishers' NPC numbers.
    pub fn fishers(&self) -> impl Iterator<Item = u32> + '_ {
        self.fishers.iter().copied()
    }

    /// The timing called `name`.
    pub fn timing(&self, name: &str) -> Option<&Timing> {
        self.timings.get(name)
    }

    /// The timings, by name.
    pub fn timings(&self) -> impl Iterator<Item = &Timing> {
        self.timings.values()
    }

    /// What catching item `item` takes, if it may be caught.
    pub fn fish(&self, item: u32) -> Option<&Fish> {
        self.fish.get(&item)
    }

    /// What may be caught, by item.
    pub fn all_fish(&self) -> impl Iterator<Item = &Fish> {
        self.fish.values()
    }

    /// What bites on the map called `map`.
    pub fn water(&self, map: &str) -> Option<&Water> {
        self.water_of
            .get(map)
            .and_then(|index| self.waters.get(*index))
    }

    /// The waters, in the order written.
    pub fn waters(&self) -> &[Water] {
        &self.waters
    }

    /// What using item `item` opens to, if it opens.
    pub fn open(&self, item: u32) -> Option<&Open> {
        self.opens.get(&item)
    }

    /// The items that open, by item.
    pub fn opens(&self) -> impl Iterator<Item = &Open> {
        self.opens.values()
    }

    /// The fire, if an item lights one.
    pub fn fire(&self) -> Option<&Fire> {
        self.fire.as_ref()
    }

    /// What a fire makes of item `item`, if it grills it.
    pub fn grill(&self, item: u32) -> Option<u32> {
        self.grills.get(&item).map(|grill| grill.into)
    }

    /// What fires grill, by item.
    pub fn grills(&self) -> impl Iterator<Item = &Grill> {
        self.grills.values()
    }
}

//! Trees grown from openm2's own recipes, not from the
//! client's SpeedTree files.
//!
//! A [`TreeBook`] holds recipes by name and the words that pick one from a
//! tree file's name (`palmetto` before `palm`) and its season (`winter`).
//! [`grow`] turns a recipe and a seed into a [`Part`]: bark first, then
//! leaves, unskinned, so it goes through the model pipeline like a
//! building. [`textures`](crate::tree_texture::textures) draws its two
//! textures by code. The same seed grows the same tree.
//!
//! - Broadleaf trees and conifers: a trunk, branches spreading by the golden
//!   angle and reaching as far as the crown's shape allows, twigs, and leaf
//!   cards at their ends (conifers: needle sprays along the branches).
//! - Palms: a curved trunk with drooping fronds at its top.
//! - Rosettes (ferns, aloes): leaves arching out of the ground.
//!
//! What is grown and why is in `docs/specs/render.md`, "Maps: trees".

use std::collections::BTreeMap;
use std::f32::consts::{PI, TAU};

use glam::{Quat, Vec3};
use m2_formats::RawStr;
use m2_formats::model::Texture;
use serde::Deserialize;

use crate::RenderError;
use crate::model::{Draw, Part, Vertex};

/// Most vertices one tree may have, bark and leaves together.
pub const MAX_TREE_VERTICES: usize = 65_536;
/// Most recipes, and most name rules, in a book.
pub const MAX_RECIPES: usize = 256;
/// Most branches off a trunk.
pub const MAX_BRANCHES: u32 = 64;
/// Most twigs on a branch.
pub const MAX_TWIGS: u32 = 16;
/// Most leaf cards on a twig, fronds on a palm or leaves of a rosette.
pub const MAX_LEAVES: u32 = 64;
/// Most leaf clusters a recipe may ask for: branches × twigs × leaves.
pub const MAX_CLUSTERS: u32 = 2048;
/// The tallest tree, in centimetres.
pub const MAX_HEIGHT: f32 = 10_000.0;

/// The length of a crown's normals: the model shader lights normals longer
/// than 4 as they are, without turning them towards the camera.
pub const CROWN_NORMAL: f32 = 8.0;

/// The angle between one branch and the next around the trunk.
const GOLDEN: f32 = 2.399_963;
/// Bark texture: centimetres around a trunk, and along it, per repeat.
const BARK_AROUND: f32 = 60.0;
const BARK_ALONG: f32 = 120.0;

/// Recipes by name and the words that pick them.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TreeBook {
    /// The recipe of a tree whose file name has none of the words.
    pub default: String,
    /// Words looked for in a tree file's name, in order: the first found
    /// picks the recipe.
    pub names: Vec<NameRule>,
    /// Words that pick the season; without one, summer.
    pub seasons: Vec<SeasonRule>,
    /// The recipes.
    pub recipes: BTreeMap<String, TreeRecipe>,
}

/// A word of a tree file's name and the recipe it picks.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NameRule {
    /// Looked for in the lowercase file name.
    pub word: String,
    /// The recipe's name.
    pub recipe: String,
}

/// A word of a tree file's name and the season it picks.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SeasonRule {
    /// Looked for in the lowercase file name.
    pub word: String,
    /// The season.
    pub season: Season,
}

/// The look a tree has.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Deserialize)]
pub enum Season {
    /// The recipe's own colours.
    #[default]
    Summer,
    /// [`TreeRecipe::fall`].
    Fall,
    /// [`TreeRecipe::winter`].
    Winter,
    /// [`TreeRecipe::flowers`].
    Flowers,
}

impl Season {
    /// Every season.
    pub const ALL: [Season; 4] = [Self::Summer, Self::Fall, Self::Winter, Self::Flowers];

    /// Its name in lowercase.
    pub fn name(self) -> &'static str {
        match self {
            Self::Summer => "summer",
            Self::Fall => "fall",
            Self::Winter => "winter",
            Self::Flowers => "flowers",
        }
    }
}

/// How a tree grows and looks.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TreeRecipe {
    /// Its build.
    pub form: Form,
    /// The lowest and the highest a tree grows, in centimetres; each tree's
    /// height is picked between them.
    pub height: (f32, f32),
    /// Its trunk.
    pub trunk: Trunk,
    /// Where its branches reach (broadleaf trees and conifers).
    #[serde(default)]
    pub crown: Crown,
    /// Its branches (broadleaf trees and conifers); `droop` also bends
    /// fronds and rosette leaves.
    #[serde(default)]
    pub branches: Branches,
    /// Its leaves.
    pub leaves: Leaves,
    /// Its look in the fall.
    #[serde(default)]
    pub fall: Option<Look>,
    /// Its look in winter.
    #[serde(default)]
    pub winter: Option<Look>,
    /// Its look in flower.
    #[serde(default)]
    pub flowers: Option<Look>,
}

/// A tree's build.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum Form {
    /// A trunk, branches, twigs and leaf clusters at their ends.
    Broadleaf,
    /// A trunk to the top, branches all along it, leaf sprays along them.
    Conifer,
    /// A curved trunk with fronds at its top.
    Palm,
    /// Leaves arching out of the ground.
    Rosette,
}

/// A trunk.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Trunk {
    /// Radius at the foot of the tallest tree, in centimetres; 0 for none.
    pub radius: f32,
    /// Radius at the top over the radius at the foot.
    pub taper: f32,
    /// Degrees it leans from upright.
    #[serde(default)]
    pub lean: f32,
    /// Degrees it curves over its length.
    #[serde(default)]
    pub bend: f32,
    /// The bark's colour.
    pub bark: [u8; 3],
    /// How deep the bark's furrows are, from 0 (smooth) to 1.
    #[serde(default)]
    pub grooves: f32,
}

/// Where branches reach.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Crown {
    /// Where the branches start, over the height.
    pub from: f32,
    /// The crown's widest radius over the height.
    pub width: f32,
    /// Its outline.
    pub shape: Shape,
}

impl Default for Crown {
    fn default() -> Self {
        Self {
            from: 0.5,
            width: 0.3,
            shape: Shape::Round,
        }
    }
}

/// A crown's outline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum Shape {
    /// Widest in the middle.
    Round,
    /// Widest at the top, like an umbrella.
    Flat,
    /// Widest at the bottom.
    Cone,
    /// About as wide all the way up.
    Column,
}

/// Branches.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Branches {
    /// Branches off the trunk.
    pub count: u32,
    /// Degrees between a branch and the trunk.
    pub angle: f32,
    /// How far they bend down along their length (negative: up), about −1
    /// to 1.
    #[serde(default)]
    pub droop: f32,
    /// Twigs on each branch.
    #[serde(default)]
    pub twigs: u32,
}

/// Leaves.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Leaves {
    /// What they look like.
    pub kind: LeafKind,
    /// Their colour.
    pub colour: [u8; 3],
    /// A card's side, or a frond's or leaf's length, in centimetres, on the
    /// tallest tree.
    pub size: f32,
    /// Cards on each twig (each branch without twigs); fronds of a palm;
    /// leaves of a rosette.
    pub count: u32,
    /// A frond's or a rosette leaf's width over its length.
    #[serde(default)]
    pub width: Option<f32>,
}

/// What leaves look like.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum LeafKind {
    /// A cluster of broad leaves.
    Broad,
    /// A spray of needles along a twig.
    Needles,
    /// A fan of narrow blades.
    Tuft,
    /// Leaflets along a rib, as long as the card (palms, ferns).
    Frond,
    /// One long leaf, as long as the card (aloes, bananas).
    Blade,
}

impl LeafKind {
    fn width(self) -> f32 {
        match self {
            Self::Frond => 0.32,
            Self::Blade => 0.2,
            Self::Broad | Self::Needles | Self::Tuft => 1.0,
        }
    }
}

/// How a season changes a tree.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Look {
    /// The leaves' colour instead of the recipe's.
    #[serde(default)]
    pub leaves: Option<[u8; 3]>,
    /// The bark's colour instead of the recipe's.
    #[serde(default)]
    pub bark: Option<[u8; 3]>,
    /// Blossoms of this colour over the leaves.
    #[serde(default)]
    pub flowers: Option<[u8; 3]>,
    /// No leaves.
    #[serde(default)]
    pub bare: bool,
    /// Snow on the leaves.
    #[serde(default)]
    pub snow: bool,
}

impl TreeBook {
    /// Reads a book from RON text and checks it: every rule names a recipe,
    /// every number is in its range.
    pub fn parse(text: &str) -> Result<Self, RenderError> {
        let book: Self = ron::Options::default()
            .with_default_extension(ron::extensions::Extensions::IMPLICIT_SOME)
            .from_str(text)
            .map_err(|e| RenderError::Trees(e.to_string()))?;
        book.check().map_err(RenderError::Trees)?;
        Ok(book)
    }

    fn check(&self) -> Result<(), String> {
        if self.recipes.len() > MAX_RECIPES {
            return Err(format!(
                "{} recipes (at most {MAX_RECIPES})",
                self.recipes.len()
            ));
        }
        if self.names.len() + self.seasons.len() > MAX_RECIPES {
            return Err(format!("more than {MAX_RECIPES} name and season rules"));
        }
        if !self.recipes.contains_key(&self.default) {
            return Err(format!("the default `{}` is not a recipe", self.default));
        }
        for rule in &self.names {
            if rule.word.is_empty() {
                return Err(format!("an empty word picks `{}`", rule.recipe));
            }
            if !self.recipes.contains_key(&rule.recipe) {
                return Err(format!(
                    "`{}` picks `{}`, which is not a recipe",
                    rule.word, rule.recipe
                ));
            }
        }
        if self.seasons.iter().any(|rule| rule.word.is_empty()) {
            return Err("an empty word picks a season".to_owned());
        }
        for (name, recipe) in &self.recipes {
            recipe
                .check()
                .map_err(|detail| format!("recipe `{name}`: {detail}"))?;
        }
        Ok(())
    }

    /// A recipe by name.
    pub fn recipe(&self, name: &str) -> Option<&TreeRecipe> {
        self.recipes.get(name)
    }

    /// The recipe's name and the season a tree file's name picks: the first
    /// rule whose word it holds (ignoring case and folders), or the default.
    pub fn pick(&self, file: &[u8]) -> (&str, Season) {
        let name = file
            .rsplit(|&b| b == b'/' || b == b'\\')
            .next()
            .unwrap_or(file)
            .to_ascii_lowercase();
        let holds = |word: &str| {
            let word = word.to_ascii_lowercase();
            name.windows(word.len()).any(|w| w == word.as_bytes())
        };
        let recipe = self
            .names
            .iter()
            .find(|rule| holds(&rule.word))
            .map_or(self.default.as_str(), |rule| rule.recipe.as_str());
        let season = self
            .seasons
            .iter()
            .find(|rule| holds(&rule.word))
            .map_or(Season::Summer, |rule| rule.season);
        (recipe, season)
    }
}

/// Fails unless `value` is a number from `min` to `max`.
fn within(what: &str, value: f32, min: f32, max: f32) -> Result<(), String> {
    if (min..=max).contains(&value) {
        Ok(())
    } else {
        Err(format!("{what} is {value}, not from {min} to {max}"))
    }
}

fn count_within(what: &str, value: u32, max: u32) -> Result<(), String> {
    if value <= max {
        Ok(())
    } else {
        Err(format!("{what} is {value}, at most {max}"))
    }
}

impl TreeRecipe {
    fn check(&self) -> Result<(), String> {
        let (low, high) = self.height;
        within("the lowest height", low, 10.0, MAX_HEIGHT)?;
        within("the highest height", high, low, MAX_HEIGHT)?;
        within("trunk.radius", self.trunk.radius, 0.0, 500.0)?;
        within("trunk.taper", self.trunk.taper, 0.0, 1.0)?;
        within("trunk.lean", self.trunk.lean, 0.0, 45.0)?;
        within("trunk.bend", self.trunk.bend, 0.0, 90.0)?;
        within("trunk.grooves", self.trunk.grooves, 0.0, 1.0)?;
        within("crown.from", self.crown.from, 0.0, 0.95)?;
        within("crown.width", self.crown.width, 0.0, 2.0)?;
        within("branches.angle", self.branches.angle, 0.0, 180.0)?;
        within("branches.droop", self.branches.droop, -2.0, 2.0)?;
        within("leaves.size", self.leaves.size, 1.0, 2000.0)?;
        if let Some(width) = self.leaves.width {
            within("leaves.width", width, 0.01, 2.0)?;
        }
        count_within("branches.count", self.branches.count, MAX_BRANCHES)?;
        count_within("branches.twigs", self.branches.twigs, MAX_TWIGS)?;
        count_within("leaves.count", self.leaves.count, MAX_LEAVES)?;
        let clusters = self.branches.count.max(1) * (self.branches.twigs + 1) * self.leaves.count;
        if clusters > MAX_CLUSTERS {
            return Err(format!(
                "branches × (twigs + 1) × leaves is {clusters}, at most {MAX_CLUSTERS}"
            ));
        }
        Ok(())
    }

    /// The tree's look in `season`: the recipe's own in summer, or a season
    /// it does not describe.
    pub fn look(&self, season: Season) -> Look {
        match season {
            Season::Summer => None,
            Season::Fall => self.fall.clone(),
            Season::Winter => self.winter.clone(),
            Season::Flowers => self.flowers.clone(),
        }
        .unwrap_or_default()
    }

    /// The bark's colour in `season`.
    pub fn bark(&self, season: Season) -> [u8; 3] {
        self.look(season).bark.unwrap_or(self.trunk.bark)
    }

    /// The leaves' colour in `season`.
    pub fn leaf_colour(&self, season: Season) -> [u8; 3] {
        self.look(season).leaves.unwrap_or(self.leaves.colour)
    }
}

/// SplitMix64: a small seeded generator, the same on every machine.
#[derive(Debug, Clone)]
pub struct Rng(u64);

impl Rng {
    /// A generator from a seed.
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    /// The next 64 bits.
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A number from 0 up to 1.
    pub fn unit(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }

    /// A number from `low` up to `high`.
    pub fn range(&mut self, low: f32, high: f32) -> f32 {
        low + (high - low) * self.unit()
    }

    /// `value`, more or less `by` of it.
    pub fn spread(&mut self, value: f32, by: f32) -> f32 {
        value * self.range(1.0 - by, 1.0 + by)
    }

    fn direction(&mut self) -> Vec3 {
        let z = self.range(-1.0, 1.0);
        let angle = self.range(0.0, TAU);
        let r = (1.0 - z * z).max(0.0).sqrt();
        Vec3::new(r * angle.cos(), r * angle.sin(), z)
    }
}

/// Vertices and triangles of one texture.
#[derive(Default)]
struct Mesh {
    vertices: Vec<Vertex>,
    indices: Vec<u32>,
}

impl Mesh {
    /// True if `count` more vertices fit (half the tree's vertices each for
    /// bark and leaves).
    fn room(&self, count: usize) -> bool {
        self.vertices.len() + count <= MAX_TREE_VERTICES / 2
    }

    fn push(&mut self, position: Vec3, normal: Vec3, uv: [f32; 2]) -> u32 {
        let index = self.vertices.len() as u32;
        self.vertices.push(Vertex {
            position: position.into(),
            normal: normal.into(),
            uv,
            joints: [0; 4],
            weights: [1.0, 0.0, 0.0, 0.0],
        });
        index
    }

    fn quad(&mut self, a: u32, b: u32, c: u32, d: u32) {
        self.indices.extend_from_slice(&[a, b, c, a, c, d]);
    }
}

/// A line of points with a radius at each.
type Spine = Vec<(Vec3, f32)>;

/// The point, radius and direction `t` (0 to 1) of the way along a spine.
fn along(spine: &[(Vec3, f32)], t: f32) -> (Vec3, f32, Vec3) {
    let last = spine.len().saturating_sub(1);
    let at = t.clamp(0.0, 1.0) * last as f32;
    let i = (at.floor() as usize).min(last.saturating_sub(1));
    let (Some(&(a, ra)), Some(&(b, rb))) = (spine.get(i), spine.get(i + 1)) else {
        let (p, r) = spine.first().copied().unwrap_or((Vec3::ZERO, 0.0));
        return (p, r, Vec3::Z);
    };
    let f = at - i as f32;
    (
        a.lerp(b, f),
        ra + (rb - ra) * f,
        (b - a).normalize_or(Vec3::Z),
    )
}

/// A tapered tube along a spine, `sides` around; the bark repeats `around`
/// times around it.
fn tube(mesh: &mut Mesh, spine: &[(Vec3, f32)], sides: u32, around: f32) {
    let sides = sides.max(3);
    let per_ring = sides + 1;
    let rings = spine.len() as u32;
    if rings < 2 || !mesh.room((rings * per_ring) as usize) {
        return;
    }
    let first = mesh.vertices.len() as u32;
    let mut side: Option<Vec3> = None;
    let mut length = 0.0;
    for (i, &(point, radius)) in spine.iter().enumerate() {
        let before = i.checked_sub(1).and_then(|j| spine.get(j));
        let prev = before.map_or(point, |p| p.0);
        let next = spine.get(i + 1).map_or(point, |p| p.0);
        let tangent = (next - prev).normalize_or(Vec3::Z);
        length += point.distance(prev);
        // Carried along from ring to ring, so the tube does not twist.
        let s = side.unwrap_or_else(|| tangent.any_orthonormal_vector());
        let s = (s - tangent * s.dot(tangent)).normalize_or(tangent.any_orthonormal_vector());
        side = Some(s);
        let other = tangent.cross(s);
        for k in 0..per_ring {
            let angle = TAU * k as f32 / sides as f32;
            let out = s * angle.cos() + other * angle.sin();
            let u = around * k as f32 / sides as f32;
            mesh.push(point + out * radius, out, [u, length / BARK_ALONG]);
        }
    }
    for i in 0..rings - 1 {
        for k in 0..sides {
            let a = first + i * per_ring + k;
            let b = a + per_ring;
            mesh.quad(a, a + 1, b + 1, b);
        }
    }
}

/// How many times the bark goes around a trunk of `radius`.
fn around(radius: f32) -> f32 {
    (TAU * radius / BARK_AROUND).round().max(1.0)
}

/// A branch's spine: from `start` along `direction`, bending down by
/// `droop` and wavering a little.
#[allow(clippy::too_many_arguments)]
fn bough(
    rng: &mut Rng,
    start: Vec3,
    direction: Vec3,
    length: f32,
    segments: u32,
    droop: f32,
    radii: (f32, f32),
    waver: f32,
) -> Spine {
    let segments = segments.max(1);
    let step = length / segments as f32;
    let mut point = start;
    let mut d = direction.normalize_or(Vec3::Z);
    let mut spine = Vec::with_capacity(segments as usize + 1);
    spine.push((point, radii.0));
    for i in 1..=segments {
        let t = i as f32 / segments as f32;
        d = (d - Vec3::Z * (droop * 1.6 / segments as f32) + rng.direction() * waver)
            .normalize_or(d);
        point += d * step;
        spine.push((point, radii.0 + (radii.1 - radii.0) * t));
    }
    spine
}

/// One card, the texture's top at `centre + v`: the corners are
/// `centre ± u ± v`.
fn card(mesh: &mut Mesh, centre: Vec3, u: Vec3, v: Vec3, flip: bool) {
    if !mesh.room(4) {
        return;
    }
    let normal = u.cross(v).normalize_or(Vec3::Z);
    let (u0, u1) = if flip { (1.0, 0.0) } else { (0.0, 1.0) };
    let a = mesh.push(centre - u - v, normal, [u0, 1.0]);
    let b = mesh.push(centre + u - v, normal, [u1, 1.0]);
    let c = mesh.push(centre + u + v, normal, [u1, 0.0]);
    let d = mesh.push(centre - u + v, normal, [u0, 0.0]);
    mesh.quad(a, b, c, d);
}

/// Three crossed cards around `centre`, about `size` across and turned at
/// random: a cluster of leaves seen from any side, from above too.
fn cluster(mesh: &mut Mesh, rng: &mut Rng, centre: Vec3, size: f32) {
    let size = rng.spread(size, 0.25);
    let turn = Quat::from_axis_angle(rng.direction(), rng.range(0.0, TAU));
    let half = size * 0.5;
    let (x, y, z) = (
        turn * Vec3::X * half,
        turn * Vec3::Y * half,
        turn * Vec3::Z * half,
    );
    for (u, v) in [(x, z), (y, z), (x, y)] {
        let flip = rng.unit() < 0.5;
        card(mesh, centre, u, v, flip);
    }
}

/// Two needle sprays from `base` along `direction`, about `size` long: one
/// lying flat, one tilted across it.
fn spray(mesh: &mut Mesh, rng: &mut Rng, base: Vec3, direction: Vec3, size: f32) {
    let size = rng.spread(size, 0.2);
    let along = direction.normalize_or(Vec3::X);
    let across = along
        .cross(Vec3::Z)
        .normalize_or(along.any_orthonormal_vector());
    let roll = rng.range(-0.4, 0.4);
    let half = size * 0.5;
    let centre = base + along * half;
    for turn in [roll, roll + 1.2] {
        let side = Quat::from_axis_angle(along, turn) * across;
        card(mesh, centre, side * size * 0.35, -along * half, false);
    }
}

/// A leaf strip from `base` along `direction`, `length` long and `width`
/// wide, sagging by `droop` of its length at its tip and raised along its
/// middle. The texture runs from its base (top) to its tip (bottom).
fn strip(mesh: &mut Mesh, base: Vec3, direction: Vec3, length: f32, width: f32, droop: f32) {
    const SEGMENTS: u32 = 6;
    let rows = SEGMENTS + 1;
    if !mesh.room((rows * 3) as usize) {
        return;
    }
    let d = direction.normalize_or(Vec3::X);
    let first = mesh.vertices.len() as u32;
    for r in 0..rows {
        let s = r as f32 / SEGMENTS as f32;
        let point = base + d * (s * length) - Vec3::Z * (droop * s * s * length);
        let tangent = (d * length - Vec3::Z * (2.0 * droop * s * length)).normalize_or(d);
        let across = tangent
            .cross(Vec3::Z)
            .normalize_or(tangent.any_orthonormal_vector());
        let mut normal = across.cross(tangent).normalize_or(Vec3::Z);
        if normal.z < 0.0 {
            normal = -normal;
        }
        let half = width * 0.5;
        let left = (normal - across * 0.35).normalize_or(normal);
        let right = (normal + across * 0.35).normalize_or(normal);
        mesh.push(point - across * half, left, [0.0, s]);
        mesh.push(point + normal * half * 0.25, normal, [0.5, s]);
        mesh.push(point + across * half, right, [1.0, s]);
    }
    for r in 0..SEGMENTS {
        let a = first + r * 3;
        let b = a + 3;
        mesh.quad(a, a + 1, b + 1, b);
        mesh.quad(a + 1, a + 2, b + 2, b + 1);
    }
}

/// How far a crown reaches at `t` of its height (0 at its bottom), over its
/// widest.
fn envelope(shape: Shape, t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    let middle = 2.0 * t - 1.0;
    match shape {
        Shape::Round => (1.0 - middle * middle).max(0.0).sqrt().max(0.3),
        Shape::Flat => 0.35 + 0.65 * t,
        Shape::Cone => (1.0 - t).max(0.08),
        Shape::Column => (1.0 - middle.powi(4)).max(0.0).sqrt().max(0.35),
    }
}

/// What a tree is grown into.
#[derive(Default)]
struct Grown {
    bark: Mesh,
    leaves: Mesh,
}

/// Grows a tree: the same recipe, season and seed give the same tree. The
/// part's draws are the bark (texture 0) and the leaves (texture 1), its
/// foot at the origin, Z up, in centimetres.
pub fn grow(recipe: &TreeRecipe, season: Season, seed: u64) -> Part {
    let mut rng = Rng::new(seed);
    let look = recipe.look(season);
    let mut grown = Grown::default();
    let (low, high) = recipe.height;
    let high = high.clamp(10.0, MAX_HEIGHT);
    let height = rng.range(low.clamp(10.0, high), high);
    let scale = height / high;
    match recipe.form {
        Form::Broadleaf | Form::Conifer => {
            crowned(&mut grown, &mut rng, recipe, &look, height, scale);
        }
        Form::Palm => palm(&mut grown, &mut rng, recipe, &look, height, scale),
        Form::Rosette => rosette(&mut grown, &mut rng, recipe, &look, scale),
    }
    grown.into_part()
}

/// A trunk's spine: leaning, curving, wider at its foot.
fn trunk(rng: &mut Rng, recipe: &TreeRecipe, length: f32, radius: f32, segments: u32) -> Spine {
    let heading = rng.range(0.0, TAU);
    let lean = rng
        .spread(recipe.trunk.lean.clamp(0.0, 45.0), 0.3)
        .to_radians();
    let level = Vec3::new(heading.cos(), heading.sin(), 0.0);
    let axis = Vec3::Z.cross(level).normalize_or(Vec3::X);
    let bend = recipe.trunk.bend.clamp(0.0, 90.0).to_radians() / segments.max(1) as f32;
    let taper = recipe.trunk.taper.clamp(0.0, 1.0);
    let mut d = Quat::from_axis_angle(axis, lean) * Vec3::Z;
    let step = length / segments.max(1) as f32;
    let mut point = Vec3::ZERO;
    let mut spine = Vec::with_capacity(segments as usize + 1);
    for i in 0..=segments {
        let t = i as f32 / segments.max(1) as f32;
        let flare = 1.0 + 0.4 * (1.0 - t * 8.0).max(0.0);
        spine.push((point, radius * (1.0 - (1.0 - taper) * t) * flare));
        d = (Quat::from_axis_angle(axis, bend) * d + rng.direction() * 0.02).normalize_or(d);
        point += d * step;
    }
    spine
}

/// A broadleaf tree or a conifer.
fn crowned(
    grown: &mut Grown,
    rng: &mut Rng,
    recipe: &TreeRecipe,
    look: &Look,
    height: f32,
    scale: f32,
) {
    let conifer = recipe.form == Form::Conifer;
    let radius = recipe.trunk.radius.clamp(0.0, 500.0) * scale;
    let top = if conifer { height } else { height * 0.85 };
    let spine = trunk(rng, recipe, top, radius, 8);
    if radius > 0.0 {
        tube(&mut grown.bark, &spine, 10, around(radius));
    }
    let leafy = !look.bare;
    let size = recipe.leaves.size.clamp(1.0, 2000.0) * scale;
    let per_twig = recipe.leaves.count.min(MAX_LEAVES);
    let crown = &recipe.crown;
    let from = crown.from.clamp(0.0, 0.95);
    let reach = crown.width.clamp(0.0, 2.0) * height;
    let count = recipe.branches.count.min(MAX_BRANCHES);
    let twigs = recipe.branches.twigs.min(MAX_TWIGS);
    let droop = recipe.branches.droop.clamp(-2.0, 2.0);
    let highest = if conifer { 0.97 } else { 0.95 };
    for i in 0..count {
        let f = ((i as f32 + 0.5 + rng.range(-0.35, 0.35)) / count as f32).clamp(0.0, 1.0);
        let (base, trunk_radius, _) = along(&spine, from + (highest - from) * f);
        let heading = i as f32 * GOLDEN + rng.range(-0.3, 0.3);
        let mut angle = rng.spread(recipe.branches.angle.clamp(0.0, 180.0), 0.15);
        if !conifer && crown.shape == Shape::Round {
            angle *= 1.0 - 0.35 * f;
        }
        let angle = angle.to_radians().clamp(0.05, PI - 0.05);
        let direction = Vec3::new(
            heading.cos() * angle.sin(),
            heading.sin() * angle.sin(),
            angle.cos(),
        );
        let length = (reach * envelope(crown.shape, f) / angle.sin().max(0.35))
            .clamp(reach * 0.15, reach * 1.6 + 1.0)
            * rng.range(0.85, 1.15);
        let r0 = if conifer {
            (trunk_radius * 0.3).min(radius * 0.25)
        } else {
            (trunk_radius * 0.6).min(radius * 0.5)
        }
        .max(0.8 * scale);
        let branch = bough(
            rng,
            base,
            direction,
            length,
            5,
            droop,
            (r0, r0 * 0.25),
            0.06,
        );
        tube(&mut grown.bark, &branch, 6, 1.0);
        if leafy && (conifer || twigs == 0) {
            for k in 0..per_twig {
                let s = 0.2 + 0.8 * (k as f32 + 1.0) / per_twig as f32;
                let (at, _, tangent) = along(&branch, s);
                if conifer {
                    let size = size * (1.1 - 0.4 * s);
                    spray(&mut grown.leaves, rng, at, tangent, size);
                } else {
                    cluster(&mut grown.leaves, rng, at, size);
                }
            }
        }
        for j in 0..twigs {
            let s = 0.35 + 0.6 * (j as f32 + 0.5) / twigs as f32;
            let (at, branch_radius, tangent) = along(&branch, s);
            let side = Quat::from_axis_angle(tangent, j as f32 * GOLDEN)
                * tangent.any_orthonormal_vector();
            let direction = (tangent * 0.77 + side * 0.64 + Vec3::Z * 0.2).normalize_or(tangent);
            let twig_length = length * 0.45 * (1.1 - s * 0.5);
            let twig = bough(
                rng,
                at,
                direction,
                twig_length,
                2,
                droop * 0.5,
                (branch_radius * 0.5, branch_radius * 0.15),
                0.1,
            );
            tube(&mut grown.bark, &twig, 4, 1.0);
            if !leafy {
                continue;
            }
            for k in 0..per_twig {
                let s = 0.5 + 0.5 * (k as f32 + 1.0) / per_twig as f32;
                let (at, _, tangent) = along(&twig, s);
                if conifer {
                    spray(&mut grown.leaves, rng, at, tangent, size * 0.8);
                } else {
                    cluster(&mut grown.leaves, rng, at, size);
                }
            }
        }
    }
    if leafy {
        let (crown_top, _, _) = along(&spine, 1.0);
        if conifer {
            for _ in 0..3 {
                let direction = (Vec3::Z * 2.0 + rng.direction()).normalize_or(Vec3::Z);
                spray(&mut grown.leaves, rng, crown_top, direction, size * 0.8);
            }
        } else {
            for _ in 0..per_twig * 2 {
                let at = crown_top + rng.direction() * reach * 0.3 + Vec3::Z * reach * 0.1;
                cluster(&mut grown.leaves, rng, at, size);
            }
        }
    }
    crown_normals(&mut grown.leaves, conifer);
}

/// Points the leaves' normals out of the crown, so it is lit as one volume
/// rather than card by card: away from its middle (conifers: from the
/// trunk), tipped up a little, and [`CROWN_NORMAL`] long so the back of the
/// crown is not turned towards the camera.
fn crown_normals(mesh: &mut Mesh, conifer: bool) {
    let count = mesh.vertices.len();
    if count == 0 {
        return;
    }
    let sum: Vec3 = mesh.vertices.iter().map(|v| Vec3::from(v.position)).sum();
    let middle = sum / count as f32;
    for vertex in &mut mesh.vertices {
        let mut out = Vec3::from(vertex.position) - middle;
        if conifer {
            out.z = 0.0;
        }
        let normal = (out.normalize_or(Vec3::Z) + Vec3::Z * 0.4).normalize_or(Vec3::Z);
        vertex.normal = (normal * CROWN_NORMAL).into();
    }
}

/// A palm: a curved trunk with fronds at its top.
fn palm(
    grown: &mut Grown,
    rng: &mut Rng,
    recipe: &TreeRecipe,
    look: &Look,
    height: f32,
    scale: f32,
) {
    let radius = recipe.trunk.radius.clamp(0.0, 500.0) * scale;
    let spine = trunk(rng, recipe, height, radius, 10);
    if radius > 0.0 {
        tube(&mut grown.bark, &spine, 8, around(radius));
    }
    if look.bare {
        return;
    }
    let (top, _, _) = along(&spine, 1.0);
    let count = recipe.leaves.count.min(MAX_LEAVES);
    let width = recipe.leaves.width.unwrap_or(recipe.leaves.kind.width());
    let droop = recipe.branches.droop.clamp(-2.0, 2.0);
    for i in 0..count {
        let heading = i as f32 * GOLDEN + rng.range(-0.2, 0.2);
        let rise = (45.0 - 75.0 * rng.unit().powf(0.7)).to_radians();
        let direction = Vec3::new(
            heading.cos() * rise.cos(),
            heading.sin() * rise.cos(),
            rise.sin(),
        );
        let length = rng.spread(recipe.leaves.size.clamp(1.0, 2000.0) * scale, 0.15);
        let sag = droop * rng.range(0.7, 1.3);
        strip(
            &mut grown.leaves,
            top + direction * radius,
            direction,
            length,
            length * width,
            sag,
        );
    }
}

/// A rosette: leaves arching out of the ground, the inner ones upright.
fn rosette(grown: &mut Grown, rng: &mut Rng, recipe: &TreeRecipe, look: &Look, scale: f32) {
    if look.bare {
        return;
    }
    let count = recipe.leaves.count.min(MAX_LEAVES);
    let width = recipe.leaves.width.unwrap_or(recipe.leaves.kind.width());
    let droop = recipe.branches.droop.clamp(-2.0, 2.0);
    for i in 0..count {
        let t = (i as f32 + 0.5) / count as f32;
        let heading = i as f32 * GOLDEN + rng.range(-0.2, 0.2);
        let rise = (80.0 - 55.0 * t + rng.range(-8.0, 8.0)).to_radians();
        let direction = Vec3::new(
            heading.cos() * rise.cos(),
            heading.sin() * rise.cos(),
            rise.sin(),
        );
        let length =
            rng.spread(recipe.leaves.size.clamp(1.0, 2000.0) * scale, 0.2) * (0.6 + 0.4 * t);
        // Sagging no more than it rises, so its tip stays above the ground.
        let sag = (droop * rng.range(0.7, 1.3)).min(rise.sin() * 0.95);
        strip(
            &mut grown.leaves,
            direction * length * 0.03,
            direction,
            length,
            length * width,
            sag,
        );
    }
}

impl Grown {
    fn into_part(self) -> Part {
        let offset = self.bark.vertices.len() as u32;
        let bark = self.bark.indices.len() as u32;
        let leaves = self.leaves.indices.len() as u32;
        let mut vertices = self.bark.vertices;
        vertices.extend(self.leaves.vertices);
        let mut indices = self.bark.indices;
        indices.extend(self.leaves.indices.iter().map(|i| i + offset));
        let mut draws = Vec::new();
        if bark > 0 {
            draws.push(Draw {
                first: 0,
                count: bark,
                texture: Some(0),
            });
        }
        if leaves > 0 {
            draws.push(Draw {
                first: bark,
                count: leaves,
                texture: Some(1),
            });
        }
        Part {
            name: RawStr::from("tree"),
            vertices,
            indices,
            draws,
            textures: vec![Texture::file("bark"), Texture::file("leaves")],
            ..Part::default()
        }
    }
}

//! The textures of grown trees, drawn by code: bark that
//! tiles, and leaves cut out by their alpha.
//!
//! Leaf textures keep their coverage down the mip chain
//! ([`coverage_mips`]): plain averaging would let far crowns thin out under
//! the alpha test. Their transparent texels take the leaves' mean colour, so
//! smaller levels do not darken the leaves' edges.

use std::f32::consts::{PI, TAU};

use m2_formats::RgbaImage;

use crate::RenderError;
use crate::model::TextureData;
use crate::terrain::mip_chain;
use crate::tree::{Form, LeafKind, Rng, Season, TreeRecipe};

/// Texels with less alpha are not drawn: the scenes' cutoff of 0.5.
pub const CUTOFF: u8 = 128;
/// Bark: width (around a trunk) and height (along it), in pixels.
pub const BARK_SIZE: (u32, u32) = (128, 256);
/// Clusters, needle sprays and tufts are square, this many pixels a side.
pub const LEAF_SIDE: u32 = 256;
/// Fronds: width and height (from base to tip), in pixels.
pub const FROND_SIZE: (u32, u32) = (128, 512);
/// Long leaves: width and height (from base to tip), in pixels.
pub const BLADE_SIZE: (u32, u32) = (64, 256);

/// Twigs, stems and ribs: the leaves' colour mixed with this brown.
const BROWN: [u8; 3] = [96, 74, 52];
const SNOW: [u8; 3] = [236, 240, 246];

/// A tree's textures in `season`: its bark, then its leaves (textures 0 and
/// 1 of a part from [`grow`](crate::tree::grow)).
pub fn textures(
    recipe: &TreeRecipe,
    season: Season,
    seed: u64,
) -> Result<Vec<Option<TextureData>>, RenderError> {
    let look = recipe.look(season);
    let bark = bark(
        recipe.bark(season),
        recipe.trunk.grooves,
        recipe.form == Form::Palm,
        seed,
    )?;
    let leaves = leaves(
        recipe.leaves.kind,
        recipe.leaf_colour(season),
        look.flowers,
        look.snow,
        seed,
    )?;
    Ok(vec![Some(bark), Some(leaves)])
}

/// Bark that tiles both ways: furrows `grooves` deep (0 to 1) along it,
/// and rings around it for palms.
pub fn bark(
    colour: [u8; 3],
    grooves: f32,
    ringed: bool,
    seed: u64,
) -> Result<TextureData, RenderError> {
    let (width, height) = BARK_SIZE;
    let mut image = RgbaImage::new(width, height)?;
    let seed = fold(seed);
    let grooves = grooves.clamp(0.0, 1.0);
    for y in 0..height {
        for x in 0..width {
            let u = x as f32 / width as f32;
            let v = y as f32 / height as f32;
            let furrows = fbm(u * 6.0, v * 2.0, (6, 2), 3, seed);
            let detail = fbm(u * 24.0, v * 48.0, (24, 48), 2, seed ^ 0x5555);
            let mut shade = 0.8 + 0.5 * (detail - 0.5);
            shade *= 1.0 - grooves * 0.6 * (1.0 - smoothstep(0.32, 0.55, furrows));
            if ringed {
                shade *= 0.7 + 0.3 * smoothstep(0.0, 0.35, (v * 8.0).fract());
            }
            image.set_pixel(x, y, tint(colour, shade));
        }
    }
    Ok(TextureData::new(mip_chain(image)?)?.with_coverage(false))
}

/// Leaves of `kind`, cut out by alpha, with blossoms or snow over them.
pub fn leaves(
    kind: LeafKind,
    colour: [u8; 3],
    flowers: Option<[u8; 3]>,
    snow: bool,
    seed: u64,
) -> Result<TextureData, RenderError> {
    let mut rng = Rng::new(seed ^ 0x6c65_6166);
    let mut image = match kind {
        LeafKind::Broad => broad(&mut rng, colour)?,
        LeafKind::Needles => needles(&mut rng, colour)?,
        LeafKind::Tuft => tuft(&mut rng, colour)?,
        LeafKind::Frond => frond(&mut rng, colour)?,
        LeafKind::Blade => blade(colour, fold(seed))?,
    };
    if let Some(flowers) = flowers {
        blossoms(&mut image, &mut rng, flowers);
    }
    if snow {
        snow_on(&mut image, fold(seed));
    }
    fill_background(&mut image);
    TextureData::new(coverage_mips(image, CUTOFF)?)
}

/// A mip chain whose levels let as much through the alpha test as the
/// first: each level's alpha is scaled so the same share of its texels
/// reaches `cutoff`.
pub fn coverage_mips(image: RgbaImage, cutoff: u8) -> Result<Vec<RgbaImage>, RenderError> {
    let share = coverage(&image, cutoff);
    let mut levels = mip_chain(image)?;
    if share <= 0.0 {
        return Ok(levels);
    }
    for level in levels.iter_mut().skip(1) {
        let mut histogram = [0u32; 256];
        for &alpha in level.pixels().iter().skip(3).step_by(4) {
            if let Some(count) = histogram.get_mut(usize::from(alpha)) {
                *count += 1;
            }
        }
        let wanted = share * (level.width() * level.height()) as f32;
        // The highest alpha that at least `wanted` texels reach.
        let mut reached = 0u32;
        let mut threshold = 0u32;
        for (value, count) in (0u32..256).zip(histogram).skip(1).rev() {
            reached += count;
            if reached as f32 >= wanted {
                threshold = value;
                break;
            }
        }
        if threshold == 0 {
            continue;
        }
        let scale = f32::from(cutoff) / threshold as f32;
        for y in 0..level.height() {
            for x in 0..level.width() {
                if let Some([r, g, b, a]) = level.pixel(x, y) {
                    let a = (f32::from(a) * scale).round().min(255.0) as u8;
                    level.set_pixel(x, y, [r, g, b, a]);
                }
            }
        }
    }
    Ok(levels)
}

/// The share of an image's texels whose alpha reaches `cutoff`.
pub fn coverage(image: &RgbaImage, cutoff: u8) -> f32 {
    let total = (image.width() * image.height()).max(1);
    let covered = image
        .pixels()
        .iter()
        .skip(3)
        .step_by(4)
        .filter(|&&a| a >= cutoff)
        .count();
    covered as f32 / total as f32
}

/// A cluster of pointed leaves on stems around the middle.
fn broad(rng: &mut Rng, colour: [u8; 3]) -> Result<RgbaImage, RenderError> {
    let side = LEAF_SIDE as f32;
    let mut image = RgbaImage::new(LEAF_SIDE, LEAF_SIDE)?;
    let middle = (side * 0.5, side * 0.5);
    let stem = mix(colour, BROWN, 0.6);
    let mut leaves = Vec::new();
    for _ in 0..56 {
        let heading = rng.range(0.0, TAU);
        let distance = side * 0.36 * rng.unit().sqrt();
        let at = (
            middle.0 + heading.cos() * distance,
            middle.1 + heading.sin() * distance,
        );
        let turn = heading + rng.range(-0.7, 0.7);
        let direction = (turn.cos(), turn.sin());
        let length = side * rng.range(0.1, 0.16);
        let width = length * rng.range(0.38, 0.52);
        let base = (
            at.0 - direction.0 * length * 0.5,
            at.1 - direction.1 * length * 0.5,
        );
        let from = (
            middle.0 + (base.0 - middle.0) * 0.35,
            middle.1 + (base.1 - middle.1) * 0.35,
        );
        line(&mut image, from, base, 2.0, stem);
        leaves.push((base, direction, length, width, vary(rng, colour)));
    }
    for (base, direction, length, width, colour) in leaves {
        leaf(
            &mut image,
            base,
            direction,
            length,
            width,
            colour,
            Outline::Leaf,
        );
    }
    Ok(image)
}

/// A twig with side shoots, needles along them all, from its base at the
/// top to its tip at the bottom.
fn needles(rng: &mut Rng, colour: [u8; 3]) -> Result<RgbaImage, RenderError> {
    let side = LEAF_SIDE as f32;
    let mut image = RgbaImage::new(LEAF_SIDE, LEAF_SIDE)?;
    let twig = mix(colour, BROWN, 0.6);
    let length = side - 6.0;
    let mut shoots = vec![((side * 0.5, 2.0), (0.0, 1.0), length)];
    for k in 0..6 {
        let s = 0.12 + 0.13 * k as f32;
        let sign = if k % 2 == 0 { 1.0 } else { -1.0 };
        let angle: f32 = 0.55;
        shoots.push((
            (side * 0.5, 2.0 + s * length),
            (sign * angle.sin(), angle.cos()),
            side * rng.range(0.35, 0.45) * (1.0 - 0.4 * s),
        ));
    }
    for &(start, direction, length) in &shoots {
        let end = (
            start.0 + direction.0 * length,
            start.1 + direction.1 * length,
        );
        line(&mut image, start, end, 2.5, twig);
    }
    for &(start, direction, length) in &shoots {
        let count = (length / 2.5) as u32;
        for n in 0..count {
            let a = n as f32 / count.max(1) as f32;
            let at = (
                start.0 + direction.0 * a * length,
                start.1 + direction.1 * a * length,
            );
            for sign in [-1.0, 1.0] {
                let angle = rng.range(0.6, 1.0);
                let (c, s) = (angle.cos(), angle.sin() * sign);
                let needle = (
                    direction.0 * c - direction.1 * s,
                    direction.0 * s + direction.1 * c,
                );
                let reach = side * rng.range(0.08, 0.13) * (1.15 - 0.5 * a);
                leaf(
                    &mut image,
                    at,
                    needle,
                    reach,
                    3.0,
                    vary(rng, colour),
                    Outline::Blade,
                );
            }
        }
    }
    Ok(image)
}

/// A fan of narrow blades rising from the bottom middle.
fn tuft(rng: &mut Rng, colour: [u8; 3]) -> Result<RgbaImage, RenderError> {
    let side = LEAF_SIDE as f32;
    let mut image = RgbaImage::new(LEAF_SIDE, LEAF_SIDE)?;
    let base = (side * 0.5, side * 0.94);
    for _ in 0..18 {
        let angle = rng.range(-1.2, 1.2);
        let direction = (angle.sin(), -angle.cos());
        let length = side * rng.range(0.45, 0.85) * (1.0 - 0.3 * angle.abs());
        let width = side * rng.range(0.035, 0.05);
        leaf(
            &mut image,
            base,
            direction,
            length,
            width,
            vary(rng, colour),
            Outline::Blade,
        );
    }
    Ok(image)
}

/// Leaflets in pairs along a rib, from its base at the top to its tip at
/// the bottom, the longest a quarter of the way down.
fn frond(rng: &mut Rng, colour: [u8; 3]) -> Result<RgbaImage, RenderError> {
    let (width, height) = FROND_SIZE;
    let mut image = RgbaImage::new(width, height)?;
    let middle = width as f32 * 0.5;
    let pairs = 46;
    for n in 0..pairs {
        let s = 0.04 + 0.94 * n as f32 / pairs as f32;
        let y = s * height as f32;
        let reach = middle * 0.98 * (s * 5.0).min(1.0) * (1.0 - s).powf(0.45).max(0.15);
        for sign in [-1.0, 1.0] {
            let angle = rng.range(0.85, 1.05);
            let direction = (sign * angle.sin(), angle.cos());
            leaf(
                &mut image,
                (middle, y),
                direction,
                reach / angle.sin(),
                7.0,
                vary(rng, colour),
                Outline::Leaf,
            );
        }
    }
    line(
        &mut image,
        (middle, 0.0),
        (middle, height as f32),
        3.0,
        mix(colour, BROWN, 0.5),
    );
    Ok(image)
}

/// One long leaf from its base at the top to its tip at the bottom,
/// streaked along its length.
fn blade(colour: [u8; 3], seed: u32) -> Result<RgbaImage, RenderError> {
    let (width, height) = BLADE_SIZE;
    let mut image = RgbaImage::new(width, height)?;
    let middle = width as f32 * 0.5;
    for y in 0..height {
        let a = (y as f32 + 0.5) / height as f32;
        let half = middle * 0.92 * (a * 5.0).clamp(0.35, 1.0) * (1.0 - a).powf(0.5);
        for x in 0..width {
            let b = ((x as f32 + 0.5 - middle) / half.max(0.01)).abs();
            if b > 1.0 {
                continue;
            }
            let streak = 0.92 + 0.12 * lattice(x as i32, 0, seed);
            let mut shade = (0.85 + 0.2 * a) * streak;
            if b < 0.06 {
                shade *= 0.82;
            } else if b > 0.85 {
                shade *= 0.85;
            }
            image.set_pixel(x, y, tint(colour, shade));
        }
    }
    Ok(image)
}

/// Blossoms of `colour`, with light hearts, over the leaves.
fn blossoms(image: &mut RgbaImage, rng: &mut Rng, colour: [u8; 3]) {
    let (width, height) = (image.width(), image.height());
    let heart = mix(colour, [255, 236, 140], 0.6);
    for _ in 0..40 {
        let mut spot = None;
        for _ in 0..12 {
            let x = rng.range(0.0, width as f32);
            let y = rng.range(0.0, height as f32);
            if image.pixel(x as u32, y as u32).is_some_and(|p| p[3] > 0) {
                spot = Some((x, y));
                break;
            }
        }
        let Some((cx, cy)) = spot else { continue };
        let radius = rng.range(5.0, 9.0);
        let petals = vary(rng, colour);
        let x0 = (cx - radius).floor().max(0.0) as u32;
        let y0 = (cy - radius).floor().max(0.0) as u32;
        let x1 = ((cx + radius).ceil() as u32).min(width.saturating_sub(1));
        let y1 = ((cy + radius).ceil() as u32).min(height.saturating_sub(1));
        for y in y0..=y1 {
            for x in x0..=x1 {
                let d = (x as f32 + 0.5 - cx).hypot(y as f32 + 0.5 - cy) / radius;
                if d > 1.0 {
                    continue;
                }
                let rgb = if d < 0.35 { heart } else { petals };
                image.set_pixel(x, y, tint(rgb, 1.0 - 0.15 * d));
            }
        }
    }
}

/// Patches of snow over the leaves.
fn snow_on(image: &mut RgbaImage, seed: u32) {
    for y in 0..image.height() {
        for x in 0..image.width() {
            let Some([r, g, b, a]) = image.pixel(x, y) else {
                continue;
            };
            if a == 0 {
                continue;
            }
            let n = fbm(x as f32 / 20.0, y as f32 / 20.0, (1024, 1024), 2, seed);
            let amount = smoothstep(0.45, 0.6, n) * 0.9;
            let [r, g, b] = mix([r, g, b], SNOW, amount);
            image.set_pixel(x, y, [r, g, b, a]);
        }
    }
}

/// Gives transparent texels the mean colour of the drawn ones.
fn fill_background(image: &mut RgbaImage) {
    let mut sum = [0u64; 3];
    let mut count = 0u64;
    for &[r, g, b, a] in image.pixels().as_chunks::<4>().0 {
        if a >= CUTOFF {
            sum[0] += u64::from(r);
            sum[1] += u64::from(g);
            sum[2] += u64::from(b);
            count += 1;
        }
    }
    if count == 0 {
        return;
    }
    let [r, g, b] = sum.map(|s| (s / count) as u8);
    for y in 0..image.height() {
        for x in 0..image.width() {
            if image.pixel(x, y).is_some_and(|p| p[3] == 0) {
                image.set_pixel(x, y, [r, g, b, 0]);
            }
        }
    }
}

/// A leaf's or a needle's outline.
#[derive(Clone, Copy)]
enum Outline {
    /// Pointed at both ends, widest in the middle.
    Leaf,
    /// Narrow at the base, widest just above it, thinning to the tip.
    Blade,
}

/// Paints a leaf from `base` along `direction` (a unit vector), `length`
/// long and `width` wide, in pixels: lighter towards its tip, a darker
/// midrib and edge.
fn leaf(
    image: &mut RgbaImage,
    base: (f32, f32),
    direction: (f32, f32),
    length: f32,
    width: f32,
    colour: [u8; 3],
    outline: Outline,
) {
    band(image, base, direction, length, width * 0.5, |a, b| {
        let profile = match outline {
            Outline::Leaf => (PI * a).sin().powf(0.75),
            Outline::Blade => (1.0 - a).powf(0.8) * (a * 8.0).clamp(0.4, 1.0),
        };
        let b = b.abs();
        if b > profile {
            return None;
        }
        let shade = 0.82 + 0.28 * a;
        Some(if b < 0.08 {
            tint(colour, shade * 0.8)
        } else if b > profile * 0.8 {
            tint(colour, shade * 0.88)
        } else {
            tint(colour, shade)
        })
    });
}

/// Paints a line `thickness` wide.
fn line(image: &mut RgbaImage, from: (f32, f32), to: (f32, f32), thickness: f32, colour: [u8; 3]) {
    let length = (to.0 - from.0).hypot(to.1 - from.1).max(0.001);
    let direction = ((to.0 - from.0) / length, (to.1 - from.1) / length);
    band(image, from, direction, length, thickness * 0.5, |_, b| {
        (b.abs() <= 1.0).then(|| tint(colour, 1.0 - 0.15 * b.abs()))
    });
}

/// Paints the texels a band from `base` along `direction` covers, `half`
/// either side of it: `paint` gets how far along (0 to 1) and how far across
/// (−1 to 1) each is and gives its colour, or none to leave it.
fn band(
    image: &mut RgbaImage,
    base: (f32, f32),
    direction: (f32, f32),
    length: f32,
    half: f32,
    paint: impl Fn(f32, f32) -> Option<[u8; 4]>,
) {
    let (dx, dy) = direction;
    let half = half.max(0.5);
    let length = length.max(0.5);
    let tip = (base.0 + dx * length, base.1 + dy * length);
    let last_x = image.width().saturating_sub(1) as f32;
    let last_y = image.height().saturating_sub(1) as f32;
    let x0 = (base.0.min(tip.0) - half - 1.0).floor().clamp(0.0, last_x) as u32;
    let x1 = (base.0.max(tip.0) + half + 1.0).ceil().clamp(0.0, last_x) as u32;
    let y0 = (base.1.min(tip.1) - half - 1.0).floor().clamp(0.0, last_y) as u32;
    let y1 = (base.1.max(tip.1) + half + 1.0).ceil().clamp(0.0, last_y) as u32;
    for y in y0..=y1 {
        for x in x0..=x1 {
            let rx = x as f32 + 0.5 - base.0;
            let ry = y as f32 + 0.5 - base.1;
            let a = (rx * dx + ry * dy) / length;
            if !(0.0..=1.0).contains(&a) {
                continue;
            }
            let b = (ry * dx - rx * dy) / half;
            if let Some(rgba) = paint(a, b) {
                image.set_pixel(x, y, rgba);
            }
        }
    }
}

/// `colour` lighter or darker by `shade`, opaque.
fn tint(colour: [u8; 3], shade: f32) -> [u8; 4] {
    let [r, g, b] = colour.map(|c| (f32::from(c) * shade).round().clamp(0.0, 255.0) as u8);
    [r, g, b, 255]
}

/// `colour`, a little lighter or darker, greener or redder.
fn vary(rng: &mut Rng, colour: [u8; 3]) -> [u8; 3] {
    let overall = rng.range(0.78, 1.12);
    colour.map(|c| {
        (f32::from(c) * overall * rng.range(0.93, 1.07))
            .round()
            .clamp(0.0, 255.0) as u8
    })
}

/// `a` mixed with `amount` (0 to 1) of `b`.
fn mix(a: [u8; 3], b: [u8; 3], amount: f32) -> [u8; 3] {
    let mut out = a;
    for (o, (x, y)) in out.iter_mut().zip(a.into_iter().zip(b)) {
        *o = (f32::from(x) + (f32::from(y) - f32::from(x)) * amount)
            .round()
            .clamp(0.0, 255.0) as u8;
    }
    out
}

fn smoothstep(low: f32, high: f32, x: f32) -> f32 {
    let t = ((x - low) / (high - low)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn fold(seed: u64) -> u32 {
    (seed ^ (seed >> 32)) as u32
}

/// A number from 0 to 1 for a lattice point.
fn lattice(x: i32, y: i32, seed: u32) -> f32 {
    let mut h = (x as u32).wrapping_mul(0x8DA6_B343)
        ^ (y as u32).wrapping_mul(0xD816_3841)
        ^ seed.wrapping_mul(0xCB1A_B31F);
    h ^= h >> 13;
    h = h.wrapping_mul(0x5BD1_E995);
    h ^= h >> 15;
    (h & 0xFFFF) as f32 / 65535.0
}

/// Smooth noise from 0 to 1 that repeats every `period` lattice cells.
fn noise(x: f32, y: f32, period: (i32, i32), seed: u32) -> f32 {
    let (px, py) = (period.0.max(1), period.1.max(1));
    let (xf, yf) = (x.floor(), y.floor());
    let (fx, fy) = (x - xf, y - yf);
    let (sx, sy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
    let x0 = (xf as i32).rem_euclid(px);
    let y0 = (yf as i32).rem_euclid(py);
    let (x1, y1) = ((x0 + 1) % px, (y0 + 1) % py);
    let top = lattice(x0, y0, seed) + (lattice(x1, y0, seed) - lattice(x0, y0, seed)) * sx;
    let bottom = lattice(x0, y1, seed) + (lattice(x1, y1, seed) - lattice(x0, y1, seed)) * sx;
    top + (bottom - top) * sy
}

/// Octaves of [`noise`], each twice as fine; repeats like the first.
fn fbm(x: f32, y: f32, period: (i32, i32), octaves: u32, seed: u32) -> f32 {
    let (mut sum, mut total, mut amplitude, mut frequency) = (0.0, 0.0, 0.5, 1);
    for octave in 0..octaves {
        let f = frequency as f32;
        sum += amplitude
            * noise(
                x * f,
                y * f,
                (period.0 * frequency, period.1 * frequency),
                seed.wrapping_add(octave),
            );
        total += amplitude;
        amplitude *= 0.5;
        frequency *= 2;
    }
    if total > 0.0 { sum / total } else { 0.0 }
}

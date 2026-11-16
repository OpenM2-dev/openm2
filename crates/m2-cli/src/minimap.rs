//! The minimap and the atlas of `play` (`docs/specs/ui.md`, "The minimap"
//! and "The atlas"): a round picture of the map around our character and a
//! picture of the whole map, drawn on the CPU from the asset folder's
//! pictures (the areas' `minimap.png`, the map's atlas picture) with marks for our
//! character, the entities near it and the atlas's points, and handed to
//! the interface as `@minimap` and `@atlas`.
//!
//! Pictures are north up: x grows east (right) and y south (down), in
//! map centimetres as the map's files count them.

use std::collections::BTreeMap;
use std::f32::consts::PI;

use m2_core::EntityKind;
use m2_formats::RgbaImage;
use m2_formats::map::points::{MapPoint, PointKind};
use m2_ui::{Skin, Skins};

use crate::interface::read_art;
use crate::map_folder::{area_minimap, atlas_of};
use crate::source::Source;
use crate::view::Files;

/// Picture pixels per metre at each zoom step of the minimap, at interface
/// scale 1 (the areas' pictures have a texel a metre).
pub(crate) const ZOOMS: [f32; 4] = [0.5, 1.0, 2.0, 4.0];
/// The zoom step the minimap starts at: a texel a pixel.
pub(crate) const START_ZOOM: usize = 1;
/// The minimap's picture is this many layout pixels across (`game.ron`):
/// marks are drawn at their own size in it at interface scale 1.
pub(crate) const MINIMAP_SIZE: f32 = 128.0;
/// An area's side in map centimetres.
const AREA_CM: f64 = 25_600.0;
/// Where the map has no picture (outside it, or an area without one).
const BACKGROUND: [f32; 4] = [0.0, 0.0, 0.0, 1.0];

/// The looks of the skin whose images the minimap draws with
/// (`docs/specs/ui.md`, "The minimap").
pub(crate) const CAMERA_LOOK: &str = "minimap_camera";
pub(crate) const PLAYER_LOOK: &str = "minimap_player";
pub(crate) const DOT_LOOK: &str = "minimap_dot";
pub(crate) const TARGET_LOOKS: [&str; 2] = ["minimap_target_1", "minimap_target_2"];

/// What a mark stands for: its colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MarkKind {
    Monster,
    Npc,
    Stone,
    Player,
    Warp,
}

impl MarkKind {
    /// The mark of an entity of `kind`; none for doors, buildings, horses
    /// and the like.
    pub fn of(kind: EntityKind) -> Option<Self> {
        match kind {
            EntityKind::Monster => Some(MarkKind::Monster),
            EntityKind::Npc => Some(MarkKind::Npc),
            EntityKind::MetinStone => Some(MarkKind::Stone),
            EntityKind::Player | EntityKind::TransformedPlayer => Some(MarkKind::Player),
            EntityKind::Warp => Some(MarkKind::Warp),
            EntityKind::Door
            | EntityKind::Building
            | EntityKind::Horse
            | EntityKind::Goto
            | EntityKind::Other(_) => None,
        }
    }

    /// The mark of an atlas point; none for kinds no file of the client
    /// uses.
    pub fn of_point(kind: &PointKind) -> Option<Self> {
        match kind {
            PointKind::Npc => Some(MarkKind::Npc),
            PointKind::Warp => Some(MarkKind::Warp),
            PointKind::Other(_) => None,
        }
    }

    /// Its colour (openm2's choice).
    fn colour(self) -> [f32; 4] {
        let [r, g, b] = match self {
            MarkKind::Monster => [235, 55, 40],
            MarkKind::Npc => [255, 210, 50],
            MarkKind::Stone => [190, 110, 240],
            MarkKind::Player => [80, 160, 255],
            MarkKind::Warp => [90, 230, 120],
        };
        [unit(r), unit(g), unit(b), 1.0]
    }
}

fn unit(value: u8) -> f32 {
    f32::from(value) / 255.0
}

/// The minimap's art: the images of its looks in the skin. What is
/// missing is drawn without: the map alone, or plain squares for marks.
#[derive(Debug, Default)]
pub(crate) struct MinimapArt {
    /// The camera's view: a dark disc with the view cut out of it, pointing
    /// up.
    camera: Option<RgbaImage>,
    /// Our character: an arrow pointing up.
    player: Option<RgbaImage>,
    /// Everyone else: a white dot, tinted.
    dot: Option<RgbaImage>,
    /// Our target, blinking between the two (white, red).
    target: [Option<RgbaImage>; 2],
}

impl MinimapArt {
    /// Reads the images of the skin's looks from `files`; notes say what
    /// could not be read.
    pub fn load(files: &Files<'_>, skins: &Skins) -> (Self, Vec<String>) {
        let mut notes = Vec::new();
        let mut read = |look: &str| {
            let Some(Skin::Image { image, .. }) = skins.skins.get(look) else {
                notes.push(format!("the skin has no image look `{look}`"));
                return None;
            };
            if image.is_empty() {
                return None;
            }
            match read_art(files, image) {
                Ok(image) => Some(image),
                Err(error) => {
                    notes.push(format!("{image}: {error:#}"));
                    None
                }
            }
        };
        let art = MinimapArt {
            camera: read(CAMERA_LOOK),
            player: read(PLAYER_LOOK),
            dot: read(DOT_LOOK),
            target: TARGET_LOOKS.map(&mut read),
        };
        (art, notes)
    }
}

/// The pictures of one map's areas, each read the first time it shows.
#[derive(Debug)]
pub(crate) struct AreaPictures {
    map: Vec<u8>,
    /// The map's size in areas.
    areas: [u32; 2],
    pictures: BTreeMap<(u32, u32), Option<RgbaImage>>,
}

impl AreaPictures {
    /// No pictures yet of map `map` (an asset folder's map file), `areas`
    /// areas wide and tall.
    pub fn new(map: &[u8], areas: [u32; 2]) -> Self {
        Self {
            map: map.to_vec(),
            areas,
            pictures: BTreeMap::new(),
        }
    }

    /// The map folder.
    pub fn map(&self) -> &[u8] {
        &self.map
    }

    /// Reads the pictures of the areas that a square `half` centimetres
    /// out from `centre` covers, those not read yet. Notes say which could
    /// not be read.
    fn load_around(&mut self, files: &Files<'_>, centre: (f64, f64), half: f64) -> Vec<String> {
        let mut notes = Vec::new();
        let range = |c: f64, areas: u32| {
            let first = ((c - half) / AREA_CM).floor().max(0.0);
            let last = ((c + half) / AREA_CM).floor().min(f64::from(areas) - 1.0);
            (first as u32, last as u32, first <= last)
        };
        let (x0, x1, xs) = range(centre.0, self.areas[0]);
        let (y0, y1, ys) = range(centre.1, self.areas[1]);
        if !(xs && ys) {
            return notes;
        }
        for y in y0..=y1 {
            for x in x0..=x1 {
                self.pictures.entry((x, y)).or_insert_with(|| {
                    match area_minimap(files, &self.map, x, y) {
                        Ok(picture) => picture,
                        Err(error) => {
                            notes.push(format!("area ({x}, {y}): {error:#}"));
                            None
                        }
                    }
                });
            }
        }
        notes
    }

    /// The map's colour at map position (`x`, `y`) centimetres, blended
    /// between texels inside its area; `None` outside the map or where an
    /// area has no picture.
    fn colour_at(&self, x: f64, y: f64) -> Option<[f32; 4]> {
        if x < 0.0 || y < 0.0 {
            return None;
        }
        let (ax, ay) = ((x / AREA_CM).floor(), (y / AREA_CM).floor());
        if ax >= f64::from(self.areas[0]) || ay >= f64::from(self.areas[1]) {
            return None;
        }
        let picture = self.pictures.get(&(ax as u32, ay as u32))?.as_ref()?;
        let u = (x / AREA_CM - ax) * f64::from(picture.width());
        let v = (y / AREA_CM - ay) * f64::from(picture.height());
        Some(sample(picture, u as f32, v as f32, true))
    }
}

/// What the minimap shows.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct MinimapView {
    /// Our character's map position, in centimetres.
    pub centre: (f64, f64),
    /// Which way it faces: degrees, 0 south, 90 east (`movement.md`).
    pub facing: u16,
    /// The camera's yaw, as `m2_render::Camera` has it (radians; 0 looks
    /// north, more turns counter-clockwise).
    pub camera_yaw: f32,
    /// The others near it: map positions and what they are.
    pub marks: Vec<((f64, f64), MarkKind)>,
    /// Our target's map position.
    pub target: Option<(f64, f64)>,
    /// Which of the target's two marks shows: it blinks.
    pub blink: bool,
}

impl MinimapView {
    /// The view with positions on whole picture pixels and the camera's
    /// yaw in whole degrees, at `cm_per_pixel`: what is drawn, so a view
    /// that did not change by a pixel is not drawn again.
    fn snapped(&self, cm_per_pixel: f64) -> Self {
        let snap = |(x, y): (f64, f64)| {
            (
                (x / cm_per_pixel).round() * cm_per_pixel,
                (y / cm_per_pixel).round() * cm_per_pixel,
            )
        };
        Self {
            centre: snap(self.centre),
            facing: self.facing,
            camera_yaw: self.camera_yaw.to_degrees().round().to_radians(),
            marks: self
                .marks
                .iter()
                .map(|&(at, kind)| (snap(at), kind))
                .collect(),
            target: self.target.map(snap),
            blink: self.blink,
        }
    }
}

/// Draws the minimap: `size` pixels across, `pixels_per_metre` of the map
/// (zoom and interface scale together), marks `scale` times their size.
/// The map's pictures around the centre, darker outside the camera's view,
/// the others' marks, our target's, our arrow in the middle; cut to a
/// circle.
pub(crate) fn draw_minimap(
    view: &MinimapView,
    pixels_per_metre: f32,
    size: u32,
    scale: f32,
    areas: &AreaPictures,
    art: &MinimapArt,
) -> Option<RgbaImage> {
    let mut canvas = Canvas::new(size, size)?;
    let cm_per_pixel = 100.0 / f64::from(pixels_per_metre.max(0.01));
    let half = f64::from(size) / 2.0;
    for (row, line) in canvas.pixels.chunks_mut(size as usize).enumerate() {
        let y = view.centre.1 + (row as f64 + 0.5 - half) * cm_per_pixel;
        for (column, pixel) in line.iter_mut().enumerate() {
            let x = view.centre.0 + (column as f64 + 0.5 - half) * cm_per_pixel;
            *pixel = areas.colour_at(x, y).unwrap_or(BACKGROUND);
        }
    }
    let middle = (half as f32, half as f32);
    if let Some(camera) = &art.camera {
        // The art fills a picture of its own width.
        let fit = size as f32 / camera.width().max(1) as f32;
        canvas.stamp(camera, middle, fit, -view.camera_yaw, [1.0; 4]);
    }
    let place = |(x, y): (f64, f64)| {
        (
            (half + (x - view.centre.0) / cm_per_pixel) as f32,
            (half + (y - view.centre.1) / cm_per_pixel) as f32,
        )
    };
    let inside = half as f32 - 2.0 * scale;
    let shows = |(x, y): (f32, f32)| (x - middle.0).hypot(y - middle.1) < inside;
    for &(at, kind) in &view.marks {
        let at = place(at);
        if shows(at) {
            canvas.mark(art.dot.as_ref(), at, scale, kind.colour());
        }
    }
    if let Some(target) = view.target.map(place).filter(|at| shows(*at)) {
        let [white, red] = &art.target;
        let (mark, fallback) = if view.blink {
            (red, [1.0, 0.0, 0.0, 1.0])
        } else {
            (white, [1.0; 4])
        };
        match mark {
            Some(mark) => canvas.stamp(mark, target, scale, 0.0, [1.0; 4]),
            None => canvas.mark(None, target, scale, fallback),
        }
    }
    canvas.arrow(art.player.as_ref(), middle, scale, view.facing);
    canvas.cut_circle();
    canvas.into_image()
}

/// Draws the atlas: the map's `picture` at `scale` times its size (smaller
/// if it does not fit), centred in a `width` × `height` picture, with the
/// atlas's points and our arrow (our map position and facing). `map_cm` is
/// the map's size in centimetres.
pub(crate) fn draw_atlas(
    picture: &RgbaImage,
    map_cm: (f64, f64),
    points: &[((f64, f64), MarkKind)],
    me: Option<((f64, f64), u16)>,
    (width, height): (u32, u32),
    scale: f32,
    art: &MinimapArt,
) -> Option<RgbaImage> {
    let mut canvas = Canvas::new(width, height)?;
    let (dw, dh) = atlas_size(picture, (width, height), scale);
    let fit = dw / picture.width().max(1) as f32;
    let left = ((width as f32 - dw) / 2.0).floor();
    let top = ((height as f32 - dh) / 2.0).floor();
    canvas.stamp(
        picture,
        (left + dw / 2.0, top + dh / 2.0),
        fit,
        0.0,
        [1.0; 4],
    );
    let place = |(x, y): (f64, f64)| {
        (
            left + (x / map_cm.0.max(1.0)) as f32 * dw,
            top + (y / map_cm.1.max(1.0)) as f32 * dh,
        )
    };
    for &(at, kind) in points {
        canvas.mark(art.dot.as_ref(), place(at), scale, kind.colour());
    }
    if let Some((at, facing)) = me {
        canvas.arrow(art.player.as_ref(), place(at), scale, facing);
    }
    canvas.into_image()
}

/// The size the atlas `picture` is drawn at in a `width` × `height` box:
/// `scale` times its own, smaller if it does not fit.
fn atlas_size(picture: &RgbaImage, (width, height): (u32, u32), scale: f32) -> (f32, f32) {
    let (pw, ph) = (
        picture.width().max(1) as f32,
        picture.height().max(1) as f32,
    );
    let fit = scale.min(width as f32 / pw).min(height as f32 / ph);
    (pw * fit, ph * fit)
}

/// The atlas's marks of a map's points: the NPCs and warps.
pub(crate) fn point_marks(points: &[MapPoint]) -> Vec<((f64, f64), MarkKind)> {
    points
        .iter()
        .filter_map(|point| {
            let kind = MarkKind::of_point(&point.kind)?;
            let [x, y] = point.position;
            Some(((f64::from(x), f64::from(y)), kind))
        })
        .collect()
}

/// The atlas of one map, as read from the asset folder.
#[derive(Debug)]
pub(crate) struct Atlas {
    map: Vec<u8>,
    /// The map's picture; `None` if the map file names none.
    pub picture: Option<RgbaImage>,
    /// Its points' marks (none without marks in the map file).
    pub marks: Vec<((f64, f64), MarkKind)>,
}

impl Atlas {
    /// Reads map `map`'s atlas picture and its points; notes say what
    /// could not be read.
    pub fn load(source: &Source, map: &[u8]) -> (Self, Vec<String>) {
        let (picture, points, notes) = atlas_of(source, map);
        let atlas = Atlas {
            map: map.to_vec(),
            picture,
            marks: point_marks(&points),
        };
        (atlas, notes)
    }

    /// The map folder.
    pub fn map(&self) -> &[u8] {
        &self.map
    }
}

/// The colour of `image` at (`u`, `v`) texels (a texel's centre is at its
/// half), blended between the nearest four, with alpha multiplied in;
/// outside the image transparent, or with `clamp` its edge's texels.
fn sample(image: &RgbaImage, u: f32, v: f32, clamp: bool) -> [f32; 4] {
    let (x, y) = (u - 0.5, v - 0.5);
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = (x - x0, y - y0);
    let (w, h) = (i64::from(image.width()), i64::from(image.height()));
    let texel = |tx: i64, ty: i64| -> [f32; 4] {
        let (tx, ty) = if clamp {
            (tx.clamp(0, w - 1), ty.clamp(0, h - 1))
        } else {
            (tx, ty)
        };
        let (Ok(tx), Ok(ty)) = (u32::try_from(tx), u32::try_from(ty)) else {
            return [0.0; 4];
        };
        let Some([r, g, b, a]) = image.pixel(tx, ty) else {
            return [0.0; 4];
        };
        let a = unit(a);
        [unit(r) * a, unit(g) * a, unit(b) * a, a]
    };
    let (x0, y0) = (x0 as i64, y0 as i64);
    let corners = [
        (texel(x0, y0), (1.0 - fx) * (1.0 - fy)),
        (texel(x0 + 1, y0), fx * (1.0 - fy)),
        (texel(x0, y0 + 1), (1.0 - fx) * fy),
        (texel(x0 + 1, y0 + 1), fx * fy),
    ];
    let mut out = [0.0; 4];
    for (colour, weight) in corners {
        for (o, c) in out.iter_mut().zip(colour) {
            *o += c * weight;
        }
    }
    out
}

/// A picture being drawn: colours from 0 to 1 with alpha multiplied in,
/// row by row from the top left.
struct Canvas {
    width: u32,
    height: u32,
    pixels: Vec<[f32; 4]>,
}

impl Canvas {
    /// A transparent picture; `None` if it is empty or too large.
    fn new(width: u32, height: u32) -> Option<Self> {
        let count = u64::from(width) * u64::from(height);
        if count == 0 || count > m2_formats::image::MAX_PIXELS {
            return None;
        }
        Some(Self {
            width,
            height,
            pixels: vec![[0.0; 4]; count as usize],
        })
    }

    /// Lays `colour` (alpha multiplied in) over the pixel at (`x`, `y`).
    fn blend(&mut self, x: u32, y: u32, colour: [f32; 4]) {
        if x >= self.width || y >= self.height {
            return;
        }
        let at = y as usize * self.width as usize + x as usize;
        if let Some(pixel) = self.pixels.get_mut(at) {
            let keep = 1.0 - colour[3];
            for (p, c) in pixel.iter_mut().zip(colour) {
                *p = c + *p * keep;
            }
        }
    }

    /// Lays `sprite` over the picture, `scale` times its size, turned
    /// `turn` radians clockwise, its centre at `centre` (pixels), its
    /// colours multiplied by `tint`.
    fn stamp(
        &mut self,
        sprite: &RgbaImage,
        centre: (f32, f32),
        scale: f32,
        turn: f32,
        tint: [f32; 4],
    ) {
        let (w, h) = (sprite.width() as f32, sprite.height() as f32);
        let scale = scale.max(0.01);
        let reach = w.hypot(h) * 0.5 * scale + 1.0;
        let (sin, cos) = turn.sin_cos();
        let rows = (centre.1 - reach).floor().max(0.0) as u32
            ..((centre.1 + reach).ceil().max(0.0) as u32).min(self.height);
        for y in rows {
            let columns = (centre.0 - reach).floor().max(0.0) as u32
                ..((centre.0 + reach).ceil().max(0.0) as u32).min(self.width);
            for x in columns {
                let dx = x as f32 + 0.5 - centre.0;
                let dy = y as f32 + 0.5 - centre.1;
                // Turned back, into the sprite's texels.
                let u = (dx * cos + dy * sin) / scale + w / 2.0;
                let v = (dy * cos - dx * sin) / scale + h / 2.0;
                if u < -1.0 || v < -1.0 || u > w + 1.0 || v > h + 1.0 {
                    continue;
                }
                let colour = sample(sprite, u, v, false);
                if colour[3] > 0.0 {
                    let mut tinted = colour;
                    for (t, k) in tinted.iter_mut().zip(tint) {
                        *t *= k;
                    }
                    self.blend(x, y, tinted);
                }
            }
        }
    }

    /// A mark at `at`: the dot art tinted `colour`, or without it a square
    /// of that colour, 4 pixels at scale 1.
    fn mark(&mut self, dot: Option<&RgbaImage>, at: (f32, f32), scale: f32, colour: [f32; 4]) {
        match dot {
            Some(dot) => self.stamp(dot, at, scale, 0.0, colour),
            None => {
                let half = 2.0 * scale;
                let x0 = (at.0 - half).round().max(0.0) as u32;
                let y0 = (at.1 - half).round().max(0.0) as u32;
                let side = (2.0 * half).round().max(1.0) as u32;
                for y in y0..y0 + side {
                    for x in x0..x0 + side {
                        self.blend(x, y, colour);
                    }
                }
            }
        }
    }

    /// Our arrow at `at`, pointing where `facing` (degrees, 0 south, 90
    /// east) looks; without the art, a white mark.
    fn arrow(&mut self, art: Option<&RgbaImage>, at: (f32, f32), scale: f32, facing: u16) {
        match art {
            // The art points up (north): turned clockwise by 180° less the
            // facing.
            Some(arrow) => {
                let turn = PI - f32::from(facing).to_radians();
                self.stamp(arrow, at, scale, turn, [1.0; 4]);
            }
            None => self.mark(None, at, scale * 1.5, [1.0; 4]),
        }
    }

    /// Cuts the picture to the circle that fills it, with a smooth edge a
    /// pixel wide.
    fn cut_circle(&mut self) {
        let (cx, cy) = (self.width as f32 / 2.0, self.height as f32 / 2.0);
        let radius = cx.min(cy);
        let width = self.width as usize;
        for (at, pixel) in self.pixels.iter_mut().enumerate() {
            let x = (at % width) as f32 + 0.5 - cx;
            let y = (at / width) as f32 + 0.5 - cy;
            let cover = (radius - x.hypot(y) + 0.5).clamp(0.0, 1.0);
            for p in pixel.iter_mut() {
                *p *= cover;
            }
        }
    }

    /// The picture as RGBA8 with straight alpha.
    fn into_image(self) -> Option<RgbaImage> {
        let mut bytes = Vec::with_capacity(self.pixels.len() * 4);
        for [r, g, b, a] in self.pixels {
            let straight = |c: f32| {
                let c = if a > 0.0 { c / a } else { 0.0 };
                (c.clamp(0.0, 1.0) * 255.0).round() as u8
            };
            bytes.extend_from_slice(&[
                straight(r),
                straight(g),
                straight(b),
                (a.clamp(0.0, 1.0) * 255.0).round() as u8,
            ]);
        }
        RgbaImage::from_pixels(self.width, self.height, bytes)
    }
}

/// What an atlas picture shows: the map, our place (on a pixel) and
/// facing, the picture's size and the interface's scale (its bits).
#[derive(Debug, Clone, PartialEq)]
struct AtlasKey {
    map: Vec<u8>,
    me: Option<((f64, f64), u16)>,
    size: (u32, u32),
    scale: u32,
}

/// What `play` keeps of the minimap and the atlas between frames: the art,
/// the pictures read, and what was drawn last.
#[derive(Debug, Default)]
pub(crate) struct MapPictures {
    art: Option<MinimapArt>,
    areas: Option<AreaPictures>,
    atlas: Option<Atlas>,
    /// The minimap drawn last: its view, size and pixels per metre.
    minimap_drawn: Option<(MinimapView, u32, u32)>,
    /// The atlas drawn last.
    atlas_drawn: Option<AtlasKey>,
}

impl MapPictures {
    /// Forgets the map and what was drawn (a logout); the art stays.
    pub fn clear(&mut self) {
        self.areas = None;
        self.atlas = None;
        self.minimap_drawn = None;
        self.atlas_drawn = None;
    }

    fn art(&mut self, files: &Files<'_>, skins: &Skins) -> &MinimapArt {
        self.art.get_or_insert_with(|| {
            let (art, notes) = MinimapArt::load(files, skins);
            for note in notes {
                eprintln!("note: minimap: {note}");
            }
            art
        })
    }

    /// The minimap of `view` on map `map` (`areas` areas), `size` pixels
    /// across at zoom step `zoom` and interface scale `scale`, with the art
    /// of `skins` from `files`; `None` if it would look as the last one
    /// drawn.
    #[allow(clippy::too_many_arguments)]
    pub fn minimap(
        &mut self,
        files: &Files<'_>,
        skins: &Skins,
        map: &[u8],
        areas: [u32; 2],
        view: &MinimapView,
        size: u32,
        zoom: usize,
        scale: f32,
    ) -> Option<RgbaImage> {
        let zoom = ZOOMS.get(zoom).copied().unwrap_or(1.0);
        let pixels_per_metre = zoom * size as f32 / MINIMAP_SIZE;
        let cm_per_pixel = 100.0 / f64::from(pixels_per_metre);
        let view = view.snapped(cm_per_pixel);
        let key = (view.clone(), size, pixels_per_metre.to_bits());
        if self.minimap_drawn.as_ref() == Some(&key) {
            return None;
        }
        if self.areas.as_ref().is_none_or(|a| a.map() != map) {
            self.areas = Some(AreaPictures::new(map, areas));
        }
        self.art(files, skins);
        let (Some(pictures), Some(art)) = (&mut self.areas, &self.art) else {
            return None;
        };
        let half = f64::from(size) / 2.0 * cm_per_pixel;
        for note in pictures.load_around(files, view.centre, half) {
            eprintln!("note: minimap: {note}");
        }
        let image = draw_minimap(&view, pixels_per_metre, size, scale, pictures, art)?;
        self.minimap_drawn = Some(key);
        Some(image)
    }

    /// The atlas of map `map` (`map_cm` centimetres across) with our
    /// place, in a `size` picture at interface scale `scale`; `None` if it
    /// would look as the last one drawn or there is no picture of the map.
    #[allow(clippy::too_many_arguments)]
    pub fn atlas(
        &mut self,
        source: &Source,
        skins: &Skins,
        map: &[u8],
        map_cm: (f64, f64),
        me: Option<((f64, f64), u16)>,
        size: (u32, u32),
        scale: f32,
    ) -> Option<RgbaImage> {
        if self.atlas.as_ref().is_none_or(|a| a.map() != map) {
            let (atlas, notes) = Atlas::load(source, map);
            for note in notes {
                eprintln!("note: {}: {note}", map.escape_ascii());
            }
            self.atlas = Some(atlas);
        }
        self.art(&source.files(), skins);
        let (Some(atlas), Some(art)) = (&self.atlas, &self.art) else {
            return None;
        };
        let picture = atlas.picture.as_ref()?;
        // Our place on a whole pixel of the map as drawn.
        let (across, down) = atlas_size(picture, size, scale);
        let me = me.map(|((x, y), facing)| {
            let snap = |c: f64, cm: f64, pixels: f32| {
                let per_pixel = cm / f64::from(pixels.max(1.0));
                (c / per_pixel).round() * per_pixel
            };
            ((snap(x, map_cm.0, across), snap(y, map_cm.1, down)), facing)
        });
        let key = AtlasKey {
            map: map.to_vec(),
            me,
            size,
            scale: scale.to_bits(),
        };
        if self.atlas_drawn.as_ref() == Some(&key) {
            return None;
        }
        let image = draw_atlas(picture, map_cm, &atlas.marks, me, size, scale, art)?;
        self.atlas_drawn = Some(key);
        Some(image)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A map of 2 × 1 areas whose pictures are 4 × 4 texels: area (0, 0)
    /// red, (1, 0) blue.
    fn areas() -> AreaPictures {
        let mut areas = AreaPictures::new(b"metin2_map_test", [2, 1]);
        let red = RgbaImage::filled(4, 4, [255, 0, 0, 255]).unwrap();
        let blue = RgbaImage::filled(4, 4, [0, 0, 255, 255]).unwrap();
        areas.pictures.insert((0, 0), Some(red));
        areas.pictures.insert((1, 0), Some(blue));
        areas
    }

    fn view() -> MinimapView {
        MinimapView {
            // Near the border between the two areas.
            centre: (25_000.0, 12_800.0),
            facing: 0,
            camera_yaw: 0.0,
            marks: Vec::new(),
            target: None,
            blink: false,
        }
    }

    fn rgba(image: &RgbaImage, x: u32, y: u32) -> [u8; 4] {
        image.pixel(x, y).unwrap()
    }

    #[test]
    fn the_map_is_drawn_around_us_north_up_and_cut_round() {
        let art = MinimapArt::default();
        // 64 pixels at a pixel a metre: 32 m each way.
        let image = draw_minimap(&view(), 1.0, 64, 1.0, &areas(), &art).unwrap();
        // West of us area (0, 0), east of us (1, 0); 6 m east is the border.
        assert_eq!(rgba(&image, 10, 32), [255, 0, 0, 255]);
        assert_eq!(rgba(&image, 50, 32), [0, 0, 255, 255]);
        // Corners are outside the circle.
        assert_eq!(rgba(&image, 0, 0)[3], 0);
        assert_eq!(rgba(&image, 63, 63)[3], 0);
        // Without the art, our mark is a white square in the middle.
        assert_eq!(rgba(&image, 32, 32), [255, 255, 255, 255]);

        // Off the map's south edge (256 m) it is black.
        let south = MinimapView {
            centre: (12_800.0, 25_000.0),
            ..view()
        };
        let image = draw_minimap(&south, 1.0, 64, 1.0, &areas(), &art).unwrap();
        assert_eq!(rgba(&image, 32, 10), [255, 0, 0, 255]);
        assert_eq!(rgba(&image, 32, 50), [0, 0, 0, 255]);
    }

    #[test]
    fn marks_sit_where_the_others_are() {
        let art = MinimapArt::default();
        let view = MinimapView {
            marks: vec![
                // 10 m west and 5 m north of us.
                ((24_000.0, 12_300.0), MarkKind::Npc),
                // Far outside the picture.
                ((0.0, 0.0), MarkKind::Monster),
            ],
            target: Some((25_000.0, 13_800.0)),
            ..view()
        };
        let image = draw_minimap(&view, 1.0, 64, 1.0, &areas(), &art).unwrap();
        assert_eq!(rgba(&image, 22, 27), [255, 210, 50, 255]);
        // The target 10 m south, white while it does not blink.
        assert_eq!(rgba(&image, 32, 42), [255, 255, 255, 255]);
        let blinking = MinimapView {
            blink: true,
            ..view
        };
        let image = draw_minimap(&blinking, 1.0, 64, 1.0, &areas(), &art).unwrap();
        assert_eq!(rgba(&image, 32, 42), [255, 0, 0, 255]);
    }

    #[test]
    fn the_arrow_turns_with_our_facing_and_the_view_with_the_camera() {
        // An arrow 4 × 9 texels: white tip (the top three rows), the rest
        // clear.
        let mut arrow = RgbaImage::new(4, 9).unwrap();
        for y in 0..3 {
            for x in 0..4 {
                arrow.set_pixel(x, y, [255, 255, 255, 255]);
            }
        }
        // The camera's art: dark everywhere but the top half (its view).
        let mut camera = RgbaImage::filled(8, 8, [0, 0, 0, 255]).unwrap();
        for y in 0..4 {
            for x in 0..8 {
                camera.set_pixel(x, y, [0, 0, 0, 0]);
            }
        }
        let art = MinimapArt {
            camera: Some(camera),
            player: Some(arrow),
            ..MinimapArt::default()
        };
        let centre = (12_800.0, 12_800.0);
        let draw = |facing: u16, camera_yaw: f32| {
            let view = MinimapView {
                centre,
                facing,
                camera_yaw,
                ..view()
            };
            draw_minimap(&view, 1.0, 64, 2.0, &areas(), &art).unwrap()
        };
        // Facing north (180), the tip is above the middle; facing east (90),
        // right of it; south (0), below.
        let north = draw(180, 0.0);
        assert_eq!(rgba(&north, 32, 25), [255, 255, 255, 255]);
        // No tip below it: there the camera's art darkens the map.
        assert_eq!(rgba(&north, 32, 38), [0, 0, 0, 255]);
        let east = draw(90, 0.0);
        assert_eq!(rgba(&east, 38, 32), [255, 255, 255, 255]);
        assert_eq!(rgba(&east, 32, 25), [255, 0, 0, 255]);
        let south = draw(0, 0.0);
        assert_eq!(rgba(&south, 32, 38), [255, 255, 255, 255]);
        // The camera looks north at yaw 0: the north is lit, the south dark.
        assert_eq!(rgba(&north, 32, 10), [255, 0, 0, 255]);
        assert_eq!(rgba(&north, 32, 54), [0, 0, 0, 255]);
        // Turned a quarter counter-clockwise it looks west.
        let west = draw(180, PI / 2.0);
        assert_eq!(rgba(&west, 10, 32), [255, 0, 0, 255]);
        assert_eq!(rgba(&west, 54, 32), [0, 0, 0, 255]);
    }

    #[test]
    fn the_atlas_fits_its_box_and_marks_points_and_us() {
        let picture = RgbaImage::filled(20, 10, [0, 128, 0, 255]).unwrap();
        let art = MinimapArt::default();
        let points = [((0.0, 0.0), MarkKind::Warp)];
        // A 2 × 1 area map drawn twice its size into 60 × 30.
        let map = (51_200.0, 25_600.0);
        let me = Some(((25_600.0, 12_800.0), 0));
        let image = draw_atlas(&picture, map, &points, me, (60, 30), 2.0, &art).unwrap();
        assert_eq!((image.width(), image.height()), (60, 30));
        // 40 × 20 in the middle: 10 clear pixels each side.
        assert_eq!(rgba(&image, 5, 15)[3], 0);
        assert_eq!(rgba(&image, 14, 15), [0, 128, 0, 255]);
        // The warp at the map's top left corner, us in the middle.
        assert_eq!(rgba(&image, 10, 5), [90, 230, 120, 255]);
        assert_eq!(rgba(&image, 30, 15), [255, 255, 255, 255]);
        // Too large for its box: shrunk to fit.
        let image = draw_atlas(&picture, map, &[], None, (10, 10), 1.0, &art).unwrap();
        assert_eq!(rgba(&image, 5, 1)[3], 0);
        assert_eq!(rgba(&image, 5, 5), [0, 128, 0, 255]);
    }

    #[test]
    fn marks_and_points_by_kind() {
        assert_eq!(MarkKind::of(EntityKind::Monster), Some(MarkKind::Monster));
        assert_eq!(
            MarkKind::of(EntityKind::TransformedPlayer),
            Some(MarkKind::Player)
        );
        assert_eq!(MarkKind::of(EntityKind::Door), None);
        let points = [
            MapPoint {
                number: 0,
                kind: PointKind::Npc,
                position: [100, 200],
                name: "a".into(),
            },
            MapPoint {
                number: 1,
                kind: PointKind::Other("WAYPOINT".into()),
                position: [1, 1],
                name: "b".into(),
            },
        ];
        assert_eq!(point_marks(&points), [((100.0, 200.0), MarkKind::Npc)]);
    }

    #[test]
    fn a_view_that_moved_less_than_a_pixel_is_the_same() {
        let a = view().snapped(100.0);
        let b = MinimapView {
            centre: (25_030.0, 12_780.0),
            camera_yaw: 0.004,
            ..view()
        }
        .snapped(100.0);
        assert_eq!(a, b);
        let c = MinimapView {
            centre: (25_060.0, 12_800.0),
            ..view()
        }
        .snapped(100.0);
        assert_ne!(a, c);
    }
}

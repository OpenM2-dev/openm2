//! Text: shaping and line breaking (cosmic-text), glyphs rasterized into
//! atlas pages that the renderer uploads, and quads for the draw list.
//!
//! Fonts come from the system; openm2 ships none. [`Fonts::system`] picks the
//! first family it finds from a preference list (Tahoma first, the face the
//! original client's look is closest to). Without any font, text takes no
//! space and draws nothing, which is what tests get from [`Fonts::empty`].

use std::collections::HashMap;

use cosmic_text::{
    Attrs, Buffer, CacheKey, Family, FontSystem, Metrics, Shaping, SwashCache, SwashContent, Wrap,
};

use crate::draw::{Colour, DrawList, TextureRef};
use crate::geometry::Rect;

/// Side of a glyph atlas page, in pixels.
pub const PAGE_SIZE: u32 = 1024;
/// Pages the atlas grows to before it starts over.
const MAX_PAGES: usize = 4;
/// Shaped texts kept for reuse.
const CACHE_LIMIT: usize = 2048;

/// A page of rasterized glyphs: RGBA8, white with the glyph's coverage in
/// alpha (colour glyphs keep their colours).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlyphPage {
    /// RGBA8 pixels, [`PAGE_SIZE`] × [`PAGE_SIZE`], row by row.
    pub pixels: Vec<u8>,
    /// Goes up whenever the pixels change; upload the page again then.
    pub version: u64,
}

/// Where a glyph is in the atlas and how it sits on the pen position.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AtlasGlyph {
    page: u32,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    left: i32,
    top: i32,
}

/// Pages filled shelf by shelf, left to right.
#[derive(Debug, Default)]
struct Atlas {
    pages: Vec<GlyphPage>,
    glyphs: HashMap<CacheKey, Option<AtlasGlyph>>,
    /// Pen on the last page: x, the current shelf's top and height.
    pen: (u32, u32, u32),
}

impl Atlas {
    /// Room for a `w` × `h` glyph (plus a pixel of padding), on a new shelf or
    /// page if needed; `None` if the atlas is full or the glyph too large.
    fn allocate(&mut self, w: u32, h: u32) -> Option<(u32, u32, u32)> {
        let (pw, ph) = (w + 1, h + 1);
        if pw > PAGE_SIZE || ph > PAGE_SIZE {
            return None;
        }
        let (mut x, mut shelf, mut height) = self.pen;
        if self.pages.is_empty() || x + pw > PAGE_SIZE {
            shelf += height;
            x = 0;
            height = 0;
        }
        if self.pages.is_empty() || shelf + ph > PAGE_SIZE {
            if self.pages.len() >= MAX_PAGES {
                return None;
            }
            self.pages.push(GlyphPage {
                pixels: vec![0; (PAGE_SIZE * PAGE_SIZE * 4) as usize],
                version: 1,
            });
            (x, shelf, height) = (0, 0, 0);
        }
        self.pen = (x + pw, shelf, height.max(ph));
        Some(((self.pages.len() - 1) as u32, x, shelf))
    }

    fn reset(&mut self) {
        self.glyphs.clear();
        self.pen = (0, 0, 0);
        // Keep the first page's memory, cleared; drop the rest.
        self.pages.truncate(1);
        for page in &mut self.pages {
            page.pixels.fill(0);
            page.version += 1;
        }
    }

    fn is_nearly_full(&self) -> bool {
        self.pages.len() >= MAX_PAGES && self.pen.1 > PAGE_SIZE * 3 / 4
    }
}

/// A glyph of a shaped text, in whole pixels from the text's top left.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ShapedGlyph {
    key: CacheKey,
    x: i32,
    y: i32,
}

/// A text laid out once and drawn many times.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Shaped {
    glyphs: Vec<ShapedGlyph>,
    /// Width of the widest line, in pixels.
    pub width: f32,
    /// Height of all lines.
    pub height: f32,
    /// Pen position after each character (byte offset, x) on the first line,
    /// for placing a caret.
    carets: Vec<(usize, f32)>,
}

impl Shaped {
    /// The x position of a caret before byte `index` of the text (first line).
    pub fn caret_x(&self, index: usize) -> f32 {
        self.carets
            .iter()
            .take_while(|(at, _)| *at <= index)
            .last()
            .map_or(0.0, |(_, x)| *x)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ShapeKey {
    text: String,
    size: u32,
    wrap: Option<u32>,
}

/// The fonts, the glyph atlas and shaped texts.
pub struct Fonts {
    system: FontSystem,
    swash: SwashCache,
    family: Option<String>,
    atlas: Atlas,
    shaped: HashMap<ShapeKey, (Shaped, u64)>,
    frame: u64,
}

impl std::fmt::Debug for Fonts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Fonts")
            .field("family", &self.family)
            .field("pages", &self.atlas.pages.len())
            .field("shaped", &self.shaped.len())
            .finish_non_exhaustive()
    }
}

impl Fonts {
    /// The system's fonts, using the first of `families` that is installed
    /// (else the system's sans-serif face). Loading them takes a moment.
    pub fn system(families: &[String]) -> Self {
        let system = FontSystem::new();
        let family = families
            .iter()
            .find(|wanted| {
                system.db().faces().any(|face| {
                    face.families
                        .iter()
                        .any(|(name, _)| name.eq_ignore_ascii_case(wanted))
                })
            })
            .cloned();
        Self::with(system, family)
    }

    /// No fonts at all: text is empty. For tests.
    pub fn empty() -> Self {
        let system = FontSystem::new_with_locale_and_db(
            "en-US".to_owned(),
            cosmic_text::fontdb::Database::new(),
        );
        Self::with(system, None)
    }

    fn with(system: FontSystem, family: Option<String>) -> Self {
        Self {
            system,
            swash: SwashCache::new(),
            family,
            atlas: Atlas::default(),
            shaped: HashMap::new(),
            frame: 0,
        }
    }

    /// The family in use, if one of the wanted ones was found.
    pub fn family(&self) -> Option<&str> {
        self.family.as_deref()
    }

    /// Number of font faces loaded.
    pub fn faces(&self) -> usize {
        self.system.db().len()
    }

    /// The atlas pages to upload.
    pub fn pages(&self) -> &[GlyphPage] {
        &self.atlas.pages
    }

    /// Call once per frame before drawing: forgets texts not drawn for a
    /// while and starts the atlas over when it is nearly full.
    pub fn begin_frame(&mut self) {
        self.frame += 1;
        if self.shaped.len() > CACHE_LIMIT {
            let frame = self.frame;
            self.shaped.retain(|_, (_, used)| frame - *used < 60);
        }
        if self.atlas.is_nearly_full() {
            self.atlas.reset();
        }
    }

    /// Width and height of `text` at `size` pixels, wrapped at `wrap` pixels
    /// if given.
    pub fn measure(&mut self, text: &str, size: f32, wrap: Option<f32>) -> (f32, f32) {
        let shaped = self.cached(text, size, wrap);
        (shaped.width, shaped.height)
    }

    /// The x position of a caret before byte `index` of `text` (one line).
    pub fn caret_x(&mut self, text: &str, size: f32, index: usize) -> f32 {
        self.cached(text, size, None).caret_x(index)
    }

    /// Draws `text` at `size` pixels with its top left at (`x`, `y`)
    /// (rounded to whole pixels), wrapped at `wrap` pixels if given. Returns
    /// its width and height.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_text(
        &mut self,
        list: &mut DrawList,
        text: &str,
        size: f32,
        wrap: Option<f32>,
        (x, y): (f32, f32),
        colour: Colour,
        clip: Option<Rect>,
    ) -> (f32, f32) {
        let key = self.ensure(text, size, wrap);
        let Self {
            system,
            swash,
            atlas,
            shaped,
            ..
        } = self;
        let Some((shaped, _)) = shaped.get(&key) else {
            return (0.0, 0.0);
        };
        let (ox, oy) = (x.round() as i32, y.round() as i32);
        for glyph in &shaped.glyphs {
            let Some(placed) = glyph_in_atlas(system, swash, atlas, glyph.key) else {
                continue;
            };
            let rect = Rect::new(
                (ox + glyph.x + placed.left) as f32,
                (oy + glyph.y - placed.top) as f32,
                placed.w as f32,
                placed.h as f32,
            );
            let page = PAGE_SIZE as f32;
            let uv = Rect::new(
                placed.x as f32 / page,
                placed.y as f32 / page,
                placed.w as f32 / page,
                placed.h as f32 / page,
            );
            list.quad(rect, uv, TextureRef::Glyphs(placed.page), colour, clip);
        }
        (shaped.width, shaped.height)
    }

    /// The text laid out, from the cache or shaped now.
    fn cached(&mut self, text: &str, size: f32, wrap: Option<f32>) -> &Shaped {
        let key = self.ensure(text, size, wrap);
        self.shaped.get(&key).map_or(&EMPTY, |(shaped, _)| shaped)
    }

    /// Shapes the text if it is not in the cache and marks it used.
    fn ensure(&mut self, text: &str, size: f32, wrap: Option<f32>) -> ShapeKey {
        let key = ShapeKey {
            text: text.to_owned(),
            size: size.to_bits(),
            wrap: wrap.map(f32::to_bits),
        };
        let frame = self.frame;
        match self.shaped.get_mut(&key) {
            Some(entry) => entry.1 = frame,
            None => {
                let shaped = self.layout(text, size, wrap);
                self.shaped.insert(key.clone(), (shaped, frame));
            }
        }
        key
    }

    fn layout(&mut self, text: &str, size: f32, wrap: Option<f32>) -> Shaped {
        // cosmic-text needs at least one font to shape with.
        if self.system.db().is_empty() {
            return Shaped::default();
        }
        let line_height = line_height(size);
        let mut buffer = Buffer::new(&mut self.system, Metrics::new(size, line_height));
        buffer.set_wrap(if wrap.is_some() {
            Wrap::WordOrGlyph
        } else {
            Wrap::None
        });
        buffer.set_size(wrap, None);
        let attrs = match &self.family {
            Some(name) => Attrs::new().family(Family::Name(name)),
            None => Attrs::new().family(Family::SansSerif),
        };
        buffer.set_text(text, &attrs, Shaping::Advanced, None);
        buffer.shape_until_scroll(&mut self.system, false);
        let mut shaped = Shaped::default();
        let mut lines = 0usize;
        for run in buffer.layout_runs() {
            lines += 1;
            shaped.width = shaped.width.max(run.line_w);
            for glyph in run.glyphs {
                let physical = glyph.physical((0.0, run.line_y), 1.0);
                shaped.glyphs.push(ShapedGlyph {
                    key: physical.cache_key,
                    x: physical.x,
                    y: physical.y,
                });
                if run.line_i == 0 {
                    shaped.carets.push((glyph.end, glyph.x + glyph.w));
                }
            }
        }
        shaped.height = lines as f32 * line_height;
        shaped
    }
}

/// The glyph in the atlas, rasterized the first time it is drawn; `None`
/// for glyphs without pixels (spaces) or when the atlas is full.
fn glyph_in_atlas(
    system: &mut FontSystem,
    swash: &mut SwashCache,
    atlas: &mut Atlas,
    key: CacheKey,
) -> Option<AtlasGlyph> {
    if let Some(known) = atlas.glyphs.get(&key) {
        return *known;
    }
    let placed = rasterize(system, swash, atlas, key);
    atlas.glyphs.insert(key, placed);
    placed
}

fn rasterize(
    system: &mut FontSystem,
    swash: &mut SwashCache,
    atlas: &mut Atlas,
    key: CacheKey,
) -> Option<AtlasGlyph> {
    let image = swash.get_image(system, key).as_ref()?;
    let (w, h) = (image.placement.width, image.placement.height);
    if w == 0 || h == 0 {
        return None;
    }
    let texel = |n: usize| -> [u8; 4] {
        match image.content {
            SwashContent::Mask => [255, 255, 255, image.data.get(n).copied().unwrap_or(0)],
            SwashContent::Color => {
                let at = n * 4;
                match image.data.get(at..at + 4) {
                    Some([r, g, b, a]) => [*r, *g, *b, *a],
                    _ => [0; 4],
                }
            }
            SwashContent::SubpixelMask => {
                let at = n * 4;
                let coverage = image
                    .data
                    .get(at..at + 3)
                    .map_or(0, |c| c.iter().map(|v| u32::from(*v)).sum::<u32>() / 3);
                [255, 255, 255, coverage as u8]
            }
        }
    };
    let pixels: Vec<[u8; 4]> = (0..(w * h) as usize).map(texel).collect();
    let (left, top) = (image.placement.left, image.placement.top);
    let (page, x, y) = atlas.allocate(w, h)?;
    let target = atlas.pages.get_mut(page as usize)?;
    for row in 0..h {
        let from = (row * w) as usize;
        let to = (((y + row) * PAGE_SIZE + x) * 4) as usize;
        let line: Vec<u8> = pixels.get(from..from + w as usize)?.concat();
        target
            .pixels
            .get_mut(to..to + line.len())?
            .copy_from_slice(&line);
    }
    target.version += 1;
    Some(AtlasGlyph {
        page,
        x,
        y,
        w,
        h,
        left,
        top,
    })
}

static EMPTY: Shaped = Shaped {
    glyphs: Vec::new(),
    width: 0.0,
    height: 0.0,
    carets: Vec::new(),
};

/// Pixels from one line to the next for a font of `size` pixels.
pub fn line_height(size: f32) -> f32 {
    (size * 1.25).round()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_atlas_fills_shelves_then_pages() {
        let mut atlas = Atlas::default();
        assert_eq!(atlas.allocate(10, 20), Some((0, 0, 0)));
        assert_eq!(atlas.allocate(10, 5), Some((0, 11, 0)));
        // does not fit on the shelf: a new shelf under the tallest glyph
        assert_eq!(atlas.allocate(PAGE_SIZE - 5, 5), Some((0, 0, 21)));
        // does not fit on the page: a new page
        assert_eq!(atlas.allocate(10, PAGE_SIZE - 10), Some((1, 0, 0)));
        assert_eq!(atlas.allocate(PAGE_SIZE + 1, 1), None);
        assert_eq!(atlas.pages.len(), 2);
        atlas.reset();
        assert_eq!(atlas.pages.len(), 1);
        assert_eq!(atlas.allocate(10, 10), Some((0, 0, 0)));
    }

    #[test]
    fn without_fonts_text_is_empty() {
        let mut fonts = Fonts::empty();
        fonts.begin_frame();
        assert_eq!(fonts.measure("hello", 12.0, None), (0.0, 0.0));
        let mut list = DrawList::default();
        fonts.draw_text(&mut list, "hello", 12.0, None, (0.0, 0.0), [255; 4], None);
        assert_eq!(list.quads(), 0);
    }
}

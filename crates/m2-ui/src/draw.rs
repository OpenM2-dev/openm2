//! What the interface hands the renderer: textured, coloured quads in
//! batches that share a texture and a clip rectangle.

use std::ops::Range;

use bytemuck::{Pod, Zeroable};

use crate::geometry::Rect;

/// An RGBA colour, 0–255 per channel, in gamma space like the client's art.
pub type Colour = [u8; 4];

/// White, the colour that leaves a texture as it is.
pub const WHITE: Colour = [255, 255, 255, 255];

/// An image the application uploaded, by a number it chose.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ImageId(pub u32);

/// The texture a batch samples.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TextureRef {
    /// A white texel: flat colour.
    White,
    /// An image the application uploaded.
    Image(ImageId),
    /// A page of the glyph atlas ([`crate::text::GlyphPage`]).
    Glyphs(u32),
}

/// A part of an image: what a layout's image path stands for once the
/// application loaded it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sprite {
    /// The image it is cut from.
    pub image: ImageId,
    /// The part, in texture coordinates (0 to 1).
    pub uv: Rect,
    /// Its size in pixels.
    pub size: (f32, f32),
}

/// One corner of a triangle: position in pixels from the target's top left,
/// texture coordinates, colour (multiplied with the texel), and the part of
/// the texture its quad samples.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
pub struct Vertex {
    /// Pixels from the top left corner.
    pub pos: [f32; 2],
    /// Texture coordinates.
    pub uv: [f32; 2],
    /// RGBA.
    pub colour: Colour,
    /// The quad's texture coordinates as a rectangle: left, top, right,
    /// bottom. A scaled image is filtered inside it, so a part of an image
    /// does not take in its neighbours (`docs/specs/render.md`).
    pub bounds: [f32; 4],
}

/// Vertices that share a texture and a clip rectangle.
#[derive(Debug, Clone, PartialEq)]
pub struct Batch {
    /// What they sample.
    pub texture: TextureRef,
    /// Pixels outside this rectangle are not drawn; `None` for the whole
    /// target.
    pub clip: Option<Rect>,
    /// Which vertices of [`DrawList::vertices`], six per quad.
    pub vertices: Range<u32>,
}

/// Quads in drawing order (later ones on top), grouped into batches.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DrawList {
    /// Two triangles per quad, no index buffer.
    pub vertices: Vec<Vertex>,
    /// Runs of vertices with one texture and clip rectangle.
    pub batches: Vec<Batch>,
}

impl DrawList {
    /// Empties the list, keeping its memory.
    pub fn clear(&mut self) {
        self.vertices.clear();
        self.batches.clear();
    }

    /// Number of quads.
    pub fn quads(&self) -> usize {
        self.vertices.len() / 6
    }

    /// Adds a quad covering `rect`, sampling `uv` of `texture`. Quads wholly
    /// outside `clip` are left out.
    pub fn quad(
        &mut self,
        rect: Rect,
        uv: Rect,
        texture: TextureRef,
        colour: Colour,
        clip: Option<Rect>,
    ) {
        if rect.is_empty() || clip.is_some_and(|c| c.intersect(&rect).is_empty()) {
            return;
        }
        let start = self.vertices.len() as u32;
        let (x0, y0, x1, y1) = (rect.x, rect.y, rect.right(), rect.bottom());
        let (u0, v0, u1, v1) = (uv.x, uv.y, uv.right(), uv.bottom());
        let bounds = [u0.min(u1), v0.min(v1), u0.max(u1), v0.max(v1)];
        let corner = |x, y, u, v| Vertex {
            pos: [x, y],
            uv: [u, v],
            colour,
            bounds,
        };
        self.vertices.extend([
            corner(x0, y0, u0, v0),
            corner(x1, y0, u1, v0),
            corner(x0, y1, u0, v1),
            corner(x0, y1, u0, v1),
            corner(x1, y0, u1, v0),
            corner(x1, y1, u1, v1),
        ]);
        let end = self.vertices.len() as u32;
        match self.batches.last_mut() {
            Some(last)
                if last.texture == texture && last.clip == clip && last.vertices.end == start =>
            {
                last.vertices.end = end;
            }
            _ => self.batches.push(Batch {
                texture,
                clip,
                vertices: start..end,
            }),
        }
    }

    /// A rectangle of flat colour.
    pub fn fill(&mut self, rect: Rect, colour: Colour, clip: Option<Rect>) {
        self.quad(
            rect,
            Rect::new(0.0, 0.0, 1.0, 1.0),
            TextureRef::White,
            colour,
            clip,
        );
    }

    /// A sprite stretched over `rect`.
    pub fn sprite(&mut self, rect: Rect, sprite: &Sprite, colour: Colour, clip: Option<Rect>) {
        self.quad(
            rect,
            sprite.uv,
            TextureRef::Image(sprite.image),
            colour,
            clip,
        );
    }

    /// A sprite repeated over `rect` at its own size, from the top left; the
    /// last row and column are cut to fit.
    pub fn tile(&mut self, rect: Rect, sprite: &Sprite, colour: Colour, clip: Option<Rect>) {
        let (w, h) = sprite.size;
        if w < 1.0 || h < 1.0 {
            return;
        }
        let mut y = rect.y;
        while y < rect.bottom() {
            let th = h.min(rect.bottom() - y);
            let mut x = rect.x;
            while x < rect.right() {
                let tw = w.min(rect.right() - x);
                let uv = Rect::new(
                    sprite.uv.x,
                    sprite.uv.y,
                    sprite.uv.w * tw / w,
                    sprite.uv.h * th / h,
                );
                self.quad(
                    Rect::new(x, y, tw, th),
                    uv,
                    TextureRef::Image(sprite.image),
                    colour,
                    clip,
                );
                x += w;
            }
            y += h;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sprite(w: f32, h: f32) -> Sprite {
        Sprite {
            image: ImageId(7),
            uv: Rect::new(0.5, 0.0, 0.25, 0.5),
            size: (w, h),
        }
    }

    #[test]
    fn quads_with_the_same_texture_and_clip_share_a_batch() {
        let mut list = DrawList::default();
        let clip = Some(Rect::new(0.0, 0.0, 100.0, 100.0));
        list.fill(Rect::new(0.0, 0.0, 10.0, 10.0), WHITE, None);
        list.fill(Rect::new(10.0, 0.0, 10.0, 10.0), WHITE, None);
        list.sprite(
            Rect::new(0.0, 0.0, 10.0, 10.0),
            &sprite(8.0, 8.0),
            WHITE,
            None,
        );
        list.fill(Rect::new(0.0, 0.0, 10.0, 10.0), WHITE, clip);
        list.fill(Rect::new(500.0, 0.0, 10.0, 10.0), WHITE, clip); // outside the clip
        let batches: Vec<_> = list
            .batches
            .iter()
            .map(|b| (b.texture, b.vertices.clone()))
            .collect();
        assert_eq!(
            batches,
            [
                (TextureRef::White, 0..12),
                (TextureRef::Image(ImageId(7)), 12..18),
                (TextureRef::White, 18..24)
            ]
        );
        assert_eq!(list.quads(), 4);
    }

    #[test]
    fn a_quad_is_two_triangles_with_its_corners() {
        let mut list = DrawList::default();
        list.sprite(
            Rect::new(1.0, 2.0, 3.0, 4.0),
            &sprite(8.0, 8.0),
            [1, 2, 3, 4],
            None,
        );
        let corners: Vec<_> = list.vertices.iter().map(|v| (v.pos, v.uv)).collect();
        assert_eq!(
            corners,
            [
                ([1.0, 2.0], [0.5, 0.0]),
                ([4.0, 2.0], [0.75, 0.0]),
                ([1.0, 6.0], [0.5, 0.5]),
                ([1.0, 6.0], [0.5, 0.5]),
                ([4.0, 2.0], [0.75, 0.0]),
                ([4.0, 6.0], [0.75, 0.5]),
            ]
        );
        assert!(list.vertices.iter().all(|v| v.colour == [1, 2, 3, 4]));
        assert!(
            list.vertices
                .iter()
                .all(|v| v.bounds == [0.5, 0.0, 0.75, 0.5])
        );
    }

    #[test]
    fn tiles_repeat_and_the_last_ones_are_cut() {
        let mut list = DrawList::default();
        list.tile(
            Rect::new(0.0, 0.0, 20.0, 12.0),
            &sprite(8.0, 8.0),
            WHITE,
            None,
        );
        // 3 columns (8, 8, 4) × 2 rows (8, 4)
        assert_eq!(list.quads(), 6);
        let last = &list.vertices[30..36];
        assert_eq!(last[5].pos, [20.0, 12.0]);
        assert_eq!(last[5].uv, [0.5 + 0.25 * 0.5, 0.5 * 0.5]);
    }
}

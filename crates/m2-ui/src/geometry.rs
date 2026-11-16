//! Rectangles and how a node is placed in its parent.

use serde::Deserialize;

/// A rectangle in pixels: `x` to the right, `y` down, from the screen's top
/// left corner.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Rect {
    /// Left edge.
    pub x: f32,
    /// Top edge.
    pub y: f32,
    /// Width.
    pub w: f32,
    /// Height.
    pub h: f32,
}

impl Rect {
    /// A rectangle from its top left corner and size.
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    /// Right edge.
    pub fn right(&self) -> f32 {
        self.x + self.w
    }

    /// Bottom edge.
    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }

    /// True if the point is inside (left and top edges included).
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && y >= self.y && x < self.right() && y < self.bottom()
    }

    /// The part of both rectangles; empty (zero size) if they do not meet.
    pub fn intersect(&self, other: &Rect) -> Rect {
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        let right = self.right().min(other.right());
        let bottom = self.bottom().min(other.bottom());
        Rect::new(x, y, (right - x).max(0.0), (bottom - y).max(0.0))
    }

    /// True if it has no area.
    pub fn is_empty(&self) -> bool {
        self.w <= 0.0 || self.h <= 0.0
    }
}

/// Which point of the parent a node is placed from. The offset (`pos`) goes
/// inwards from the edges named: `BottomRight` with `pos: (8, 4)` puts the
/// node's bottom right corner 8 pixels left of and 4 pixels above the
/// parent's. Along a centred axis the offset moves right or down from the
/// centre.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
pub enum Anchor {
    /// The top left corner (the default).
    #[default]
    TopLeft,
    /// The middle of the top edge.
    Top,
    /// The top right corner.
    TopRight,
    /// The middle of the left edge.
    Left,
    /// The centre.
    Center,
    /// The middle of the right edge.
    Right,
    /// The bottom left corner.
    BottomLeft,
    /// The middle of the bottom edge.
    Bottom,
    /// The bottom right corner.
    BottomRight,
}

/// Where along one axis a node is anchored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Side {
    Start,
    Middle,
    End,
}

impl Anchor {
    fn sides(self) -> (Side, Side) {
        use Side::{End, Middle, Start};
        match self {
            Anchor::TopLeft => (Start, Start),
            Anchor::Top => (Middle, Start),
            Anchor::TopRight => (End, Start),
            Anchor::Left => (Start, Middle),
            Anchor::Center => (Middle, Middle),
            Anchor::Right => (End, Middle),
            Anchor::BottomLeft => (Start, End),
            Anchor::Bottom => (Middle, End),
            Anchor::BottomRight => (End, End),
        }
    }
}

/// A width or height: pixels, or what is left of the parent.
///
/// In RON a plain number is pixels; `Fill(m)` stretches the node to `m`
/// pixels short of the parent's far edge (for a centred node, `m` pixels
/// short of both edges).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Length {
    /// Pixels.
    Px(f32),
    /// The parent's size less the offset and this margin.
    Fill(f32),
}

impl<'de> Deserialize<'de> for Length {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(LengthVisitor)
    }
}

/// Reads a number as pixels and `Fill(m)` as a fill. RON hands a named
/// tuple to `deserialize_any` without its name, so a one-number tuple is
/// taken as `Fill`.
struct LengthVisitor;

impl<'de> serde::de::Visitor<'de> for LengthVisitor {
    type Value = Length;

    fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("pixels (a number) or Fill(margin)")
    }

    fn visit_f64<E: serde::de::Error>(self, v: f64) -> Result<Length, E> {
        Ok(Length::Px(v as f32))
    }

    fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<Length, E> {
        Ok(Length::Px(v as f32))
    }

    fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<Length, E> {
        Ok(Length::Px(v as f32))
    }

    fn visit_newtype_struct<D: serde::Deserializer<'de>>(self, d: D) -> Result<Length, D::Error> {
        f32::deserialize(d).map(Length::Fill)
    }

    fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut seq: A) -> Result<Length, A::Error> {
        let margin = seq
            .next_element::<f32>()?
            .ok_or_else(|| serde::de::Error::invalid_length(0, &self))?;
        if seq.next_element::<serde::de::IgnoredAny>()?.is_some() {
            return Err(serde::de::Error::invalid_length(2, &self));
        }
        Ok(Length::Fill(margin))
    }
}

impl Length {
    /// The length `scale` times as long (a fill's margin too).
    pub fn scaled(self, scale: f32) -> Self {
        match self {
            Length::Px(px) => Length::Px(px * scale),
            Length::Fill(margin) => Length::Fill(margin * scale),
        }
    }

    fn resolve(self, parent: f32, offset: f32, side: Side) -> f32 {
        match self {
            Length::Px(px) => px,
            Length::Fill(margin) => match side {
                Side::Middle => parent - 2.0 * margin,
                Side::Start | Side::End => parent - offset - margin,
            }
            .max(0.0),
        }
    }
}

/// Places a node of `size` in `parent`, `offset` inwards from the `anchor`.
/// Positions are rounded to whole pixels, so images and text stay sharp.
pub fn place(parent: &Rect, anchor: Anchor, offset: (f32, f32), size: (Length, Length)) -> Rect {
    let (sx, sy) = anchor.sides();
    let w = size.0.resolve(parent.w, offset.0, sx);
    let h = size.1.resolve(parent.h, offset.1, sy);
    let along = |start: f32, extent: f32, length: f32, offset: f32, side: Side| match side {
        Side::Start => start + offset,
        Side::Middle => start + (extent - length) / 2.0 + offset,
        Side::End => start + extent - length - offset,
    };
    Rect::new(
        along(parent.x, parent.w, w, offset.0, sx).round(),
        along(parent.y, parent.h, h, offset.1, sy).round(),
        w.round(),
        h.round(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const PARENT: Rect = Rect::new(100.0, 50.0, 400.0, 300.0);

    #[test]
    fn anchors_measure_inwards_from_their_edges() {
        let size = (Length::Px(40.0), Length::Px(20.0));
        let at = |anchor| place(&PARENT, anchor, (8.0, 4.0), size);
        assert_eq!(at(Anchor::TopLeft), Rect::new(108.0, 54.0, 40.0, 20.0));
        assert_eq!(at(Anchor::TopRight), Rect::new(452.0, 54.0, 40.0, 20.0));
        assert_eq!(at(Anchor::BottomLeft), Rect::new(108.0, 326.0, 40.0, 20.0));
        assert_eq!(at(Anchor::BottomRight), Rect::new(452.0, 326.0, 40.0, 20.0));
        // centred axes: the offset moves right and down from the centre
        assert_eq!(at(Anchor::Center), Rect::new(288.0, 194.0, 40.0, 20.0));
        assert_eq!(at(Anchor::Bottom), Rect::new(288.0, 326.0, 40.0, 20.0));
    }

    #[test]
    fn fill_takes_what_is_left() {
        let fill = Length::Fill(10.0);
        let r = place(
            &PARENT,
            Anchor::TopLeft,
            (8.0, 4.0),
            (fill, Length::Px(20.0)),
        );
        assert_eq!(r, Rect::new(108.0, 54.0, 382.0, 20.0));
        let r = place(&PARENT, Anchor::Center, (0.0, 0.0), (fill, fill));
        assert_eq!(r, Rect::new(110.0, 60.0, 380.0, 280.0));
        let tiny = Length::Fill(1000.0);
        assert_eq!(
            place(&PARENT, Anchor::TopLeft, (0.0, 0.0), (tiny, tiny)).w,
            0.0
        );
    }

    #[test]
    fn rectangles_meet_or_not() {
        let a = Rect::new(0.0, 0.0, 10.0, 10.0);
        assert_eq!(
            a.intersect(&Rect::new(5.0, 5.0, 10.0, 10.0)),
            Rect::new(5.0, 5.0, 5.0, 5.0)
        );
        assert!(a.intersect(&Rect::new(20.0, 0.0, 5.0, 5.0)).is_empty());
        assert!(a.contains(0.0, 9.9) && !a.contains(10.0, 5.0));
    }
}

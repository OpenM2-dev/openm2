//! Where one may walk on a map (`docs/specs/server/content.md`,
//! "Walk masks"): squares a metre wide, open or blocked; whether a straight
//! walk crosses a blocked square ([`WalkMap::clear`]); and paths around the
//! blocked ones ([`WalkMap::find_path`]). The server checks its players'
//! walks and moves its monsters with these, and `play` finds its paths with
//! them, so a path the client walks is a walk the server accepts.
//!
//! Points are in centimetres from the map's top-left corner (east, south);
//! square (x, y) covers x × 100 to x × 100 + 99 cm east, and the same south.
//! Lines are followed in whole numbers, so every machine gets the same
//! answer.

use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::fmt;
use std::io::Cursor;

/// A square's side, in centimetres.
pub const SQUARE: i32 = 100;
/// The widest or tallest walk mask, in squares (8 km).
pub const MAX_WALK_SIDE: u32 = 8_192;
/// The widest or tallest box a path is looked for in, in squares.
pub const MAX_PATH_SIDE: u32 = 512;
/// How many squares a path may stray outside the box around its start and
/// its end.
const PATH_MARGIN: i64 = 32;
/// How far the legs of a path keep from blocked squares, in centimetres,
/// where they can: a walker a little off its leg still misses the wall.
pub const PATH_CLEARANCE: i64 = 30;
/// A step's cost straight and diagonally (10 × √2, rounded).
const STRAIGHT: u32 = 10;
const DIAGONAL: u32 = 14;
/// The grey under which a mask's pixel is blocked.
const BLOCKED_BELOW: u16 = 128;

/// Why a walk mask cannot be made or read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalkError(String);

impl fmt::Display for WalkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for WalkError {}

fn error(message: impl Into<String>) -> WalkError {
    WalkError(message.into())
}

/// A map's squares, open or blocked.
#[derive(Clone, PartialEq, Eq)]
pub struct WalkMap {
    width: u32,
    height: u32,
    /// A bit a square, row by row from the north-west corner; set means
    /// blocked.
    blocked: Vec<u64>,
}

impl fmt::Debug for WalkMap {
    // Sizes only: a mask holds millions of squares.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WalkMap")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("blocked", &self.blocked_squares())
            .finish()
    }
}

/// The square a coordinate (centimetres) falls in.
pub fn square_of(cm: i32) -> i64 {
    i64::from(cm).div_euclid(i64::from(SQUARE))
}

/// The centre of square `index`, in centimetres.
fn centre(index: i64) -> i32 {
    i32::try_from(index * i64::from(SQUARE) + i64::from(SQUARE / 2)).unwrap_or(i32::MAX)
}

impl WalkMap {
    /// A mask of `width` × `height` squares, all open. Refused for a side
    /// of 0 or past [`MAX_WALK_SIDE`].
    pub fn open(width: u32, height: u32) -> Result<Self, WalkError> {
        if width == 0 || height == 0 || width > MAX_WALK_SIDE || height > MAX_WALK_SIDE {
            return Err(error(format!(
                "{width} × {height} squares: a walk mask is 1 to {MAX_WALK_SIDE} squares on a side"
            )));
        }
        let words = (u64::from(width) * u64::from(height)).div_ceil(64);
        let words = usize::try_from(words).map_err(|_| error("too large"))?;
        Ok(Self {
            width,
            height,
            blocked: vec![0; words],
        })
    }

    /// Squares from west to east.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Squares from north to south.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// The bit of square (x, y), if it is on the map.
    fn bit(&self, x: i64, y: i64) -> Option<(usize, u64)> {
        let inside =
            (0..i64::from(self.width)).contains(&x) && (0..i64::from(self.height)).contains(&y);
        if !inside {
            return None;
        }
        let index = u64::try_from(y * i64::from(self.width) + x).ok()?;
        let word = usize::try_from(index / 64).ok()?;
        Some((word, 1 << (index % 64)))
    }

    /// Whether square (x, y) is blocked; squares off the map are.
    pub fn is_blocked(&self, x: i64, y: i64) -> bool {
        match self.bit(x, y) {
            Some((word, bit)) => self.blocked.get(word).is_none_or(|w| w & bit != 0),
            None => true,
        }
    }

    /// Blocks or opens square (x, y); squares off the map are left alone.
    pub fn set_blocked(&mut self, x: u32, y: u32, blocked: bool) {
        let Some((word, bit)) = self.bit(i64::from(x), i64::from(y)) else {
            return;
        };
        if let Some(w) = self.blocked.get_mut(word) {
            if blocked {
                *w |= bit;
            } else {
                *w &= !bit;
            }
        }
    }

    /// How many squares are blocked.
    pub fn blocked_squares(&self) -> u64 {
        self.blocked.iter().map(|w| u64::from(w.count_ones())).sum()
    }

    /// Whether the square under point `at` is blocked.
    pub fn blocked_at(&self, at: (i32, i32)) -> bool {
        self.is_blocked(square_of(at.0), square_of(at.1))
    }

    /// Whether a straight walk from `from` to `to` crosses no blocked
    /// square: every square the line passes through is open, and where it
    /// passes exactly through a corner, the two squares beside it are open
    /// too, so no walk slips between two blocked squares that touch at a
    /// corner. Blocked squares at the start are walked out of (a character
    /// standing in one is not stuck), but the square it ends in must be
    /// open.
    pub fn clear(&self, from: (i32, i32), to: (i32, i32)) -> bool {
        let mut leaving = true;
        let mut end_open = false;
        let finished = self.along(from, to, |map, x, y, beside| {
            if let Some((ax, ay, bx, by)) = beside {
                return leaving || !(map.is_blocked(ax, ay) || map.is_blocked(bx, by));
            }
            let blocked = map.is_blocked(x, y);
            if !blocked {
                leaving = false;
            }
            end_open = !blocked;
            leaving || !blocked
        });
        finished && end_open
    }

    /// Whether every square from `from` to `to` is open, with no walking
    /// out at the start.
    fn open_between(&self, from: (i32, i32), to: (i32, i32)) -> bool {
        self.along(from, to, |map, x, y, beside| match beside {
            Some((ax, ay, bx, by)) => !(map.is_blocked(ax, ay) || map.is_blocked(bx, by)),
            None => !map.is_blocked(x, y),
        })
    }

    /// Follows the line from `from` to `to` square by square, in order,
    /// calling `visit` with each square it passes through (`beside` `None`)
    /// and, where it goes exactly through a corner, with the two squares
    /// beside the corner (`beside` `Some`). Stops when `visit` says no; true
    /// if it reached the end.
    fn along(
        &self,
        from: (i32, i32),
        to: (i32, i32),
        mut visit: impl FnMut(&Self, i64, i64, Option<(i64, i64, i64, i64)>) -> bool,
    ) -> bool {
        let side = i64::from(SQUARE);
        let (fx, fy) = (i64::from(from.0), i64::from(from.1));
        let (dx, dy) = (i64::from(to.0) - fx, i64::from(to.1) - fy);
        let (mut x, mut y) = (square_of(from.0), square_of(from.1));
        let (end_x, end_y) = (square_of(to.0), square_of(to.1));
        let (step_x, step_y) = (dx.signum(), dy.signum());
        // The next grid line the walk crosses each way, in centimetres.
        let mut line_x = if dx > 0 { (x + 1) * side } else { x * side };
        let mut line_y = if dy > 0 { (y + 1) * side } else { y * side };
        let steps = (end_x - x).abs() + (end_y - y).abs();
        for _ in 0..=steps {
            if !visit(self, x, y, None) {
                return false;
            }
            if (x, y) == (end_x, end_y) {
                return true;
            }
            // Which line comes first, of those crossed before the end:
            // |line_x − fx| / |dx| against |line_y − fy| / |dy|,
            // multiplied out.
            let before_x = dx != 0 && (line_x - fx).abs() < dx.abs();
            let before_y = dy != 0 && (line_y - fy).abs() < dy.abs();
            let cross_x = before_x.then(|| i128::from((line_x - fx).abs()) * i128::from(dy.abs()));
            let cross_y = before_y.then(|| i128::from((line_y - fy).abs()) * i128::from(dx.abs()));
            let (go_x, go_y) = match (cross_x, cross_y) {
                (Some(a), Some(b)) => (a <= b, b <= a),
                (Some(_), None) => (true, false),
                (None, Some(_)) => (false, true),
                // The end lies on the far side of a grid line or corner:
                // into its square, beside this one.
                (None, None) => (x != end_x, y != end_y),
            };
            if go_x && go_y && !visit(self, x, y, Some((x + step_x, y, x, y + step_y))) {
                return false;
            }
            if go_x {
                x += step_x;
                line_x += step_x * side;
            }
            if go_y {
                y += step_y;
                line_y += step_y * side;
            }
        }
        false
    }

    /// Whether a leg from `from` to `to` is [`clear`](Self::clear) and keeps
    /// [`PATH_CLEARANCE`] from blocked squares on both sides, but for the
    /// clearance at its two ends.
    fn clear_wide(&self, from: (i32, i32), to: (i32, i32)) -> bool {
        if !self.clear(from, to) {
            return false;
        }
        let (dx, dy) = (
            f64::from(to.0) - f64::from(from.0),
            f64::from(to.1) - f64::from(from.1),
        );
        let length = dx.hypot(dy);
        let keep = PATH_CLEARANCE as f64;
        if length <= 2.0 * keep {
            return true;
        }
        let (ux, uy) = (dx / length, dy / length);
        let point = |base: (i32, i32), along: f64, across: f64| {
            (
                (f64::from(base.0) + ux * along - uy * across).round() as i32,
                (f64::from(base.1) + uy * along + ux * across).round() as i32,
            )
        };
        [keep, -keep]
            .iter()
            .all(|&across| self.open_between(point(from, keep, across), point(to, -keep, across)))
    }

    /// A path from `from` to `to` around blocked squares: the points to walk
    /// to in turn, `to` last, each leg [`clear`](Self::clear). `None` if
    /// `to` is in a blocked square or off the map, or no path is found
    /// within [`MAX_PATH_SIDE`] squares of box (the start and the end, and
    /// 32 squares around them).
    ///
    /// The search is A* over squares, eight ways, never cutting a blocked
    /// square's corner; its squares are then joined into the fewest legs
    /// that keep [`PATH_CLEARANCE`] from walls.
    pub fn find_path(&self, from: (i32, i32), to: (i32, i32)) -> Option<Vec<(i32, i32)>> {
        let start = (square_of(from.0), square_of(from.1));
        let goal = (square_of(to.0), square_of(to.1));
        if self.is_blocked(goal.0, goal.1) {
            return None;
        }
        if start == goal || self.clear_wide(from, to) {
            return Some(vec![to]);
        }
        let squares = self.search(start, goal);
        let Some(squares) = squares else {
            return self.clear(from, to).then(|| vec![to]);
        };
        let mut points: Vec<(i32, i32)> = squares
            .iter()
            .map(|&(x, y)| (centre(x), centre(y)))
            .collect();
        if let Some(last) = points.last_mut() {
            *last = to;
        }
        Some(self.straighten(from, &points))
    }

    /// A* from square `start` to square `goal`: the squares after `start`,
    /// `goal` last.
    fn search(&self, start: (i64, i64), goal: (i64, i64)) -> Option<Vec<(i64, i64)>> {
        let (width, height) = (i64::from(self.width), i64::from(self.height));
        let left = (start.0.min(goal.0) - PATH_MARGIN).max(0);
        let top = (start.1.min(goal.1) - PATH_MARGIN).max(0);
        let right = (start.0.max(goal.0) + PATH_MARGIN).min(width - 1);
        let bottom = (start.1.max(goal.1) + PATH_MARGIN).min(height - 1);
        let (box_w, box_h) = (right - left + 1, bottom - top + 1);
        let most = i64::from(MAX_PATH_SIDE);
        if box_w <= 0 || box_h <= 0 || box_w > most || box_h > most {
            return None;
        }
        let index = |x: i64, y: i64| -> Option<usize> {
            let inside = (left..=right).contains(&x) && (top..=bottom).contains(&y);
            inside
                .then(|| usize::try_from((y - top) * box_w + (x - left)).ok())
                .flatten()
        };
        let square = |i: usize| -> (i64, i64) {
            let i = i64::try_from(i).unwrap_or(0);
            (left + i % box_w, top + i / box_w)
        };
        let guess = |(x, y): (i64, i64)| -> u32 {
            let (a, b) = ((x - goal.0).abs(), (y - goal.1).abs());
            let (long, short) = (a.max(b), a.min(b));
            u32::try_from(long * i64::from(STRAIGHT) + short * i64::from(DIAGONAL - STRAIGHT))
                .unwrap_or(u32::MAX)
        };
        let cells = usize::try_from(box_w * box_h).ok()?;
        let (first, last) = (index(start.0, start.1)?, index(goal.0, goal.1)?);
        let mut cost = vec![u32::MAX; cells];
        let mut came = vec![usize::MAX; cells];
        let mut done = vec![false; cells];
        let mut open = BinaryHeap::new();
        *cost.get_mut(first)? = 0;
        open.push(Reverse((guess(start), guess(start), first)));
        while let Some(Reverse((_, _, at))) = open.pop() {
            if done.get(at).copied().unwrap_or(true) {
                continue;
            }
            if let Some(d) = done.get_mut(at) {
                *d = true;
            }
            if at == last {
                break;
            }
            let here = cost.get(at).copied().unwrap_or(u32::MAX);
            let (x, y) = square(at);
            for (sx, sy) in [
                (1, 0),
                (-1, 0),
                (0, 1),
                (0, -1),
                (1, 1),
                (1, -1),
                (-1, 1),
                (-1, -1),
            ] {
                let (nx, ny) = (x + sx, y + sy);
                let Some(next) = index(nx, ny) else { continue };
                let diagonal = sx != 0 && sy != 0;
                if self.is_blocked(nx, ny)
                    || (diagonal && (self.is_blocked(x + sx, y) || self.is_blocked(x, y + sy)))
                {
                    continue;
                }
                let step = if diagonal { DIAGONAL } else { STRAIGHT };
                let through = here.saturating_add(step);
                if through < cost.get(next).copied().unwrap_or(0) {
                    if let (Some(c), Some(f)) = (cost.get_mut(next), came.get_mut(next)) {
                        *c = through;
                        *f = at;
                    }
                    let left_over = guess((nx, ny));
                    open.push(Reverse((
                        through.saturating_add(left_over),
                        left_over,
                        next,
                    )));
                }
            }
        }
        if !done.get(last).copied().unwrap_or(false) {
            return None;
        }
        let mut squares = Vec::new();
        let mut at = last;
        while at != first {
            squares.push(square(at));
            at = came.get(at).copied()?;
        }
        squares.reverse();
        Some(squares)
    }

    /// Joins a path's points into legs: from each point reached, the
    /// farthest of the following ones in a [`clear_wide`](Self::clear_wide)
    /// line, else the next one (next to it, so in a clear line).
    fn straighten(&self, from: (i32, i32), points: &[(i32, i32)]) -> Vec<(i32, i32)> {
        let mut legs = Vec::new();
        let mut at = from;
        let mut i = 0;
        while let Some(&next) = points.get(i) {
            let mut reach = i;
            for (j, &point) in points.iter().enumerate().skip(i + 1) {
                if !self.clear_wide(at, point) {
                    break;
                }
                reach = j;
            }
            let point = points.get(reach).copied().unwrap_or(next);
            legs.push(point);
            at = point;
            i = reach + 1;
        }
        legs
    }

    /// Reads a walk mask from a PNG: a pixel a square, dark (grey under 128,
    /// or the mean of red, green and blue) blocked, light open; alpha is not
    /// read. Any bit depth and colour type; not interlaced.
    pub fn from_png(bytes: &[u8]) -> Result<Self, WalkError> {
        let failed =
            |e: png::DecodingError| error(format!("not a PNG image the server reads: {e}"));
        let mut decoder = png::Decoder::new(Cursor::new(bytes));
        decoder.set_transformations(png::Transformations::normalize_to_color8());
        let mut reader = decoder.read_info().map_err(failed)?;
        let (width, height, interlaced) = {
            let info = reader.info();
            (info.width, info.height, info.interlaced)
        };
        if interlaced {
            return Err(error(
                "an interlaced PNG: save the mask without interlacing",
            ));
        }
        let mut map = Self::open(width, height)?;
        let channels: usize = match reader.output_color_type() {
            (png::ColorType::Grayscale, png::BitDepth::Eight) => 1,
            (png::ColorType::GrayscaleAlpha, png::BitDepth::Eight) => 2,
            (png::ColorType::Rgb, png::BitDepth::Eight) => 3,
            (png::ColorType::Rgba, png::BitDepth::Eight) => 4,
            (color, depth) => {
                return Err(error(format!(
                    "pixels of {color:?} at {depth:?} after expanding them"
                )));
            }
        };
        let read = channels.min(3);
        for y in 0..height {
            let row = reader
                .next_row()
                .map_err(failed)?
                .ok_or_else(|| error("the image ends early"))?;
            let pixels = row.data().chunks_exact(channels);
            if pixels.len() < width as usize {
                return Err(error("a row of the image is too short"));
            }
            for (x, pixel) in (0..width).zip(pixels) {
                let sum: u16 = pixel.iter().take(read).map(|&c| u16::from(c)).sum();
                let grey = sum / read as u16;
                if grey < BLOCKED_BELOW {
                    map.set_blocked(x, y, true);
                }
            }
        }
        Ok(map)
    }

    /// Writes the mask as a black and white PNG (a bit a pixel, white
    /// open), with `comment` as its text when there is one.
    pub fn to_png(&self, comment: Option<&str>) -> Result<Vec<u8>, WalkError> {
        let failed = |e: png::EncodingError| error(format!("writing the PNG: {e}"));
        let row_bytes = self.width.div_ceil(8) as usize;
        let mut data = vec![0u8; row_bytes * self.height as usize];
        for (y, row) in (0..self.height).zip(data.chunks_exact_mut(row_bytes)) {
            for x in 0..self.width {
                if !self.is_blocked(i64::from(x), i64::from(y))
                    && let Some(byte) = row.get_mut((x / 8) as usize)
                {
                    *byte |= 0x80 >> (x % 8);
                }
            }
        }
        let mut out = Vec::new();
        let mut encoder = png::Encoder::new(&mut out, self.width, self.height);
        encoder.set_color(png::ColorType::Grayscale);
        encoder.set_depth(png::BitDepth::One);
        if let Some(comment) = comment {
            encoder
                .add_text_chunk("Comment".to_owned(), comment.to_owned())
                .map_err(failed)?;
        }
        let mut writer = encoder.write_header().map_err(failed)?;
        writer.write_image_data(&data).map_err(failed)?;
        writer.finish().map_err(failed)?;
        Ok(out)
    }
}

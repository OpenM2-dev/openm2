//! The maps' walls (`docs/specs/server/world.md`, "Walls"): which map a
//! point lies on, whether a straight walk stays on it and crosses no
//! blocked square of its walk mask, and paths around those squares for
//! monsters. A point on no map (a world without maps, a start off them) is
//! open, and so is a map without a mask.

use m2_content::{Map, WalkMap, square_of};

use super::Server;

/// Why a straight walk may not be walked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Blocked {
    /// It leaves the map it starts on.
    OffMap,
    /// It crosses a blocked square.
    Wall,
}

impl Blocked {
    /// For the log.
    pub(super) fn why(self) -> &'static str {
        match self {
            Blocked::OffMap => "off its map",
            Blocked::Wall => "through a wall",
        }
    }
}

/// `at` in the centimetres of the map whose corner is `origin`.
fn local(at: (i32, i32), origin: (i32, i32)) -> (i32, i32) {
    (at.0.saturating_sub(origin.0), at.1.saturating_sub(origin.1))
}

/// Whether two points fall in the same square: no line between them
/// crosses another.
fn same_square(a: (i32, i32), b: (i32, i32)) -> bool {
    (square_of(a.0), square_of(a.1)) == (square_of(b.0), square_of(b.1))
}

impl Server {
    /// The map under world point `at` and its walk mask, if it has one.
    fn ground_at(&self, at: (i32, i32)) -> Option<(&Map, Option<&WalkMap>)> {
        let map = self.content.map_at(at)?;
        Some((map, self.content.walk(&map.name)))
    }

    /// Whether a straight walk from `from` to `to` may be walked: it stays
    /// on the map `from` lies on and crosses none of its blocked squares
    /// ([`WalkMap::clear`]: a walker standing in one walks out of it). A
    /// move within one square crosses nothing.
    pub(super) fn passable(&self, from: (i32, i32), to: (i32, i32)) -> Result<(), Blocked> {
        let Some((map, walk)) = self.ground_at(from) else {
            return Ok(());
        };
        if !map.contains_cm(to) {
            return Err(Blocked::OffMap);
        }
        let Some(walk) = walk else {
            return Ok(());
        };
        let origin = map.origin_cm();
        let (a, b) = (local(from, origin), local(to, origin));
        if same_square(a, b) || walk.clear(a, b) {
            Ok(())
        } else {
            Err(Blocked::Wall)
        }
    }

    /// Whether the square under world point `at` is open.
    pub(super) fn open_at(&self, at: (i32, i32)) -> bool {
        match self.ground_at(at) {
            Some((map, Some(walk))) => !walk.blocked_at(local(at, map.origin_cm())),
            Some((_, None)) | None => true,
        }
    }

    /// A path from `from` to `to` around blocked squares
    /// ([`WalkMap::find_path`]): the points to walk to in turn, `to` last.
    /// `None` if `to` is off the map `from` lies on, blocked, or out of
    /// the search's reach.
    pub(super) fn path(&self, from: (i32, i32), to: (i32, i32)) -> Option<Vec<(i32, i32)>> {
        let Some((map, walk)) = self.ground_at(from) else {
            return Some(vec![to]);
        };
        if !map.contains_cm(to) {
            return None;
        }
        let Some(walk) = walk else {
            return Some(vec![to]);
        };
        let origin = map.origin_cm();
        let path = walk.find_path(local(from, origin), local(to, origin))?;
        let world = |(x, y): (i32, i32)| (x.saturating_add(origin.0), y.saturating_add(origin.1));
        Some(path.into_iter().map(world).collect())
    }
}

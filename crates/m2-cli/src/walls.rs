//! Where one may walk on a map of the client: its areas'
//! `attr.atr` flags made into a walk mask, a square a metre. The import
//! writes these masks into the content folder, so the server blocks the
//! same squares as the client's files do; `play` walks paths around them
//! ([`Walls`]).

use anyhow::Result;
use m2_assets::Vfs;
use m2_assets::map::{read_attributes, read_setting};
use m2_content::{WalkMap, square_of};
use m2_core::Point;
use m2_formats::map::AREA_CELLS;
use m2_formats::map::grid::AttrMap;

use crate::map_folder::AssetMap;

/// The attribute flag that blocks a square (`map.md`, "Attributes").
pub(crate) const BLOCKED: u8 = 0x01;
/// The attribute flag of water.
pub(crate) const WATER: u8 = 0x02;

/// How finely a straight line is followed to find where it stops being
/// clear, in centimetres.
const STEP: f64 = 25.0;

/// A path whose end is nearer than this to where we stand, in centimetres,
/// gets us nowhere: we are as near as we can get.
pub(crate) const NOWHERE: f64 = 10.0;

/// The walk mask of the client's map in `folder`, a square a metre: blocked
/// where the attributes under the square's centre have [`BLOCKED`], or where
/// no area or attribute file covers it; and its water mask: set where they
/// have [`WATER`], `None` for a map without any. `size` is
/// the map's width and height in metres (as `maps.toml` gives it), or `None`
/// for its own areas. `None` when the map has no settings or no attribute
/// files at all, or is too large for a mask.
pub(crate) fn walk_and_water(
    vfs: &Vfs,
    folder: &[u8],
    size: Option<[f64; 2]>,
) -> Result<Option<(WalkMap, Option<WalkMap>)>> {
    let Ok(setting) = read_setting(vfs, folder) else {
        return Ok(None);
    };
    let [areas_w, areas_h] = setting.size;
    let mut attributes = Vec::new();
    for y in 0..areas_h {
        for x in 0..areas_w {
            attributes.push(read_attributes(vfs, folder, x, y)?);
        }
    }
    let area = f64::from(setting.area_size());
    let Some(walk) = mask(setting.size, area, size, &attributes, BLOCKED, true) else {
        return Ok(None);
    };
    let water = mask(setting.size, area, size, &attributes, WATER, false)
        .filter(|water| water.blocked_squares() > 0);
    Ok(Some((walk, water)))
}

/// The walk mask of an asset folder's map, as [`walk_and_water`] makes the
/// client's: from its areas' `attributes.png`.
pub(crate) fn asset_walk_map(map: &AssetMap<'_>) -> Result<Option<WalkMap>> {
    let [areas_w, areas_h] = map.file.size;
    let mut attributes = Vec::new();
    for y in 0..areas_h {
        for x in 0..areas_w {
            attributes.push(map.attributes(x, y)?);
        }
    }
    Ok(mask(
        map.file.size,
        f64::from(map.file.cell_cm()) * f64::from(AREA_CELLS),
        None,
        &attributes,
        BLOCKED,
        true,
    ))
}

/// The water mask of an asset folder's map: its set squares are water, by
/// its areas' `attributes.png` ([`WATER`]); `None` without water.
pub(crate) fn asset_water_map(map: &AssetMap<'_>) -> Result<Option<WalkMap>> {
    let [areas_w, areas_h] = map.file.size;
    let mut attributes = Vec::new();
    for y in 0..areas_h {
        for x in 0..areas_w {
            attributes.push(map.attributes(x, y)?);
        }
    }
    Ok(mask(
        map.file.size,
        f64::from(map.file.cell_cm()) * f64::from(AREA_CELLS),
        None,
        &attributes,
        WATER,
        false,
    )
    .filter(|water| water.blocked_squares() > 0))
}

/// The mask of a map of `areas` areas `area` centimetres a side, with
/// their attributes (row by row), `size` metres wide and high (else its
/// areas'): a square is set where the attributes under its centre have
/// `flag`, and where none cover it if `unknown` says so. `None` without
/// attributes, or for a map too large.
fn mask(
    [areas_w, areas_h]: [u32; 2],
    area: f64,
    size: Option<[f64; 2]>,
    attributes: &[Option<AttrMap>],
    flag: u8,
    unknown: bool,
) -> Option<WalkMap> {
    let size = size.unwrap_or([
        f64::from(areas_w) * area / 100.0,
        f64::from(areas_h) * area / 100.0,
    ]);
    if attributes.iter().all(Option::is_none) {
        return None;
    }
    let Ok(mut walk) = WalkMap::open(size[0].ceil() as u32, size[1].ceil() as u32) else {
        return None;
    };
    for my in 0..walk.height() {
        for mx in 0..walk.width() {
            // The square's centre, in centimetres, and the area under it.
            let (cx, cy) = (f64::from(mx) * 100.0 + 50.0, f64::from(my) * 100.0 + 50.0);
            let (ax, ay) = ((cx / area) as u32, (cy / area) as u32);
            let flags = (ax < areas_w && ay < areas_h)
                .then(|| attributes.get((ay * areas_w + ax) as usize))
                .flatten()
                .and_then(Option::as_ref)
                .and_then(|attributes| {
                    let grid = &attributes.grid;
                    let across = |c: f64, a: u32, side: u32| {
                        ((c - f64::from(a) * area) / area * f64::from(side)) as u32
                    };
                    grid.get(across(cx, ax, grid.width()), across(cy, ay, grid.height()))
                });
            if flags.map_or(unknown, |flags| flags & flag != 0) {
                walk.set_blocked(mx, my, true);
            }
        }
    }
    Some(walk)
}

/// Where our character may walk on the map it is on: the map's walk mask
/// and where the map lies in the world.
pub(crate) struct Walls {
    walk: WalkMap,
    /// The map's top-left corner in the world, in centimetres.
    base: Point,
    /// Its water, if it has any.
    water: Option<WalkMap>,
}

impl Walls {
    /// The walls of a map of mask `walk`, whose top-left corner is at
    /// `base`.
    pub(crate) fn new(walk: WalkMap, base: Point) -> Self {
        Self {
            walk,
            base,
            water: None,
        }
    }

    /// The same walls, with the map's water mask.
    pub(crate) fn with_water(self, water: Option<WalkMap>) -> Self {
        Self { water, ..self }
    }

    /// Whether world point `at` is on water: a square of the map's water
    /// mask.
    pub(crate) fn is_water(&self, at: Point) -> bool {
        let Some(water) = &self.water else {
            return false;
        };
        let (x, y) = self.local(at);
        let (x, y) = (square_of(x), square_of(y));
        (0..i64::from(water.width())).contains(&x)
            && (0..i64::from(water.height())).contains(&y)
            && water.is_blocked(x, y)
    }

    fn local(&self, point: Point) -> (i32, i32) {
        (
            point.x.saturating_sub(self.base.x),
            point.y.saturating_sub(self.base.y),
        )
    }

    fn world(&self, (x, y): (i32, i32)) -> Point {
        Point::new(x.saturating_add(self.base.x), y.saturating_add(self.base.y))
    }

    /// The points to walk to in turn from `from` to `to` around the walls
    /// (`m2_content::WalkMap::find_path`). A `to` in a blocked square gives
    /// way to the first open spot on the line back from it to `from`; where
    /// no path is found, the walk goes as far along the straight line as it
    /// is clear. Empty when that is nowhere.
    pub(crate) fn path(&self, from: Point, to: Point) -> Vec<Point> {
        let start = self.local(from);
        let mut end = self.local(to);
        if self.walk.blocked_at(end) {
            match along(end, start).find(|&spot| !self.walk.blocked_at(spot)) {
                Some(open) if open != start => end = open,
                _ => return Vec::new(),
            }
        }
        if let Some(points) = self.walk.find_path(start, end) {
            return points.into_iter().map(|p| self.world(p)).collect();
        }
        self.reach_local(start, end)
            .map(|spot| vec![self.world(spot)])
            .unwrap_or_default()
    }

    /// The farthest spot along the straight line from `from` to `to` that
    /// we may walk to, every [`STEP`] up to the first wall: `from` itself
    /// when that is nowhere.
    pub(crate) fn reach(&self, from: Point, to: Point) -> Point {
        self.reach_local(self.local(from), self.local(to))
            .map_or(from, |spot| self.world(spot))
    }

    /// [`reach`](Self::reach) on the map, `None` for nowhere.
    fn reach_local(&self, start: (i32, i32), end: (i32, i32)) -> Option<(i32, i32)> {
        let mut reach = None;
        for spot in along(start, end) {
            if self.walk.clear(start, spot) {
                reach = Some(spot);
            } else if !self.walk.blocked_at(spot) {
                // Open itself, so a wall before it stops the line.
                break;
            }
        }
        reach
    }

    /// True when the straight walk from `from` to `to` is clear
    /// (`m2_content::WalkMap::clear`).
    #[cfg(test)]
    pub(crate) fn clear(&self, from: Point, to: Point) -> bool {
        self.walk.clear(self.local(from), self.local(to))
    }
}

/// Spots every [`STEP`] along the line from `from` to `to`, `to` last
/// (`from` left out).
fn along(from: (i32, i32), to: (i32, i32)) -> impl Iterator<Item = (i32, i32)> {
    let (dx, dy) = (
        f64::from(to.0) - f64::from(from.0),
        f64::from(to.1) - f64::from(from.1),
    );
    let steps = (dx.hypot(dy) / STEP).ceil().max(1.0) as u32;
    (1..=steps).map(move |i| {
        let t = f64::from(i) / f64::from(steps);
        (
            (f64::from(from.0) + dx * t).round() as i32,
            (f64::from(from.1) + dy * t).round() as i32,
        )
    })
}

/// The path from `from` to `to`: around `walls` if the map has them, else
/// straight.
pub(crate) fn path(walls: Option<&Walls>, from: Point, to: Point) -> Vec<Point> {
    match walls {
        Some(walls) => walls.path(from, to),
        None => vec![to],
    }
}

/// The farthest spot along the straight line from `from` to `to` that we
/// may walk to: up to the first of `walls` if the map has them, else `to`.
pub(crate) fn reach(walls: Option<&Walls>, from: Point, to: Point) -> Point {
    match walls {
        Some(walls) => walls.reach(from, to),
        None => to,
    }
}

/// The path from `from` to `stand` centimetres short of `at`, coming along
/// its last leg: to stand before an item, an NPC or a foe. Where `at` cannot
/// be reached, as near as the walls let us.
pub(crate) fn path_short_of(
    walls: Option<&Walls>,
    from: Point,
    at: Point,
    stand: f64,
) -> Vec<Point> {
    let mut points = path(walls, from, at);
    if points.last() != Some(&at) {
        return points;
    }
    points.pop();
    let before = points.last().copied().unwrap_or(from);
    let distance = before.distance(at);
    if distance > stand {
        let t = (distance - stand) / distance;
        points.push(Point::new(
            (f64::from(before.x) + f64::from(at.x - before.x) * t).round() as i32,
            (f64::from(before.y) + f64::from(at.y - before.y) * t).round() as i32,
        ));
    }
    points
}

/// True when `path` takes us somewhere from `here`: it has an end, not
/// [`NOWHERE`] near.
pub(crate) fn goes_somewhere(path: &[Point], here: Point) -> bool {
    path.last().is_some_and(|end| end.distance(here) >= NOWHERE)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Where the test map lies in the world.
    const BASE: Point = Point {
        x: 100_000,
        y: 200_000,
    };

    /// A map 30 × 20 m with the squares of `blocked` walled.
    fn walls(blocked: impl IntoIterator<Item = (u32, u32)>) -> Walls {
        let mut walk = WalkMap::open(30, 20).unwrap();
        for (x, y) in blocked {
            walk.set_blocked(x, y, true);
        }
        Walls::new(walk, BASE)
    }

    /// The point `x`, `y` metres into the test map.
    fn at(x: f64, y: f64) -> Point {
        Point::new(BASE.x + (x * 100.0) as i32, BASE.y + (y * 100.0) as i32)
    }

    /// Every leg of `path` from `from` is a clear walk.
    fn walkable(walls: &Walls, from: Point, path: &[Point]) -> bool {
        let mut here = from;
        path.iter().all(|&next| {
            let clear = walls.walk.clear(walls.local(here), walls.local(next));
            here = next;
            clear
        })
    }

    #[test]
    fn without_walls_a_path_is_straight() {
        assert_eq!(path(None, at(1.0, 1.0), at(9.0, 9.0)), [at(9.0, 9.0)]);
        let open = walls([]);
        assert_eq!(open.path(at(1.0, 1.0), at(9.0, 9.0)), [at(9.0, 9.0)]);
    }

    #[test]
    fn a_path_goes_around_a_wall() {
        // A wall from the north edge 15 m down, 10 m in.
        let walls = walls((0..15).map(|y| (10, y)));
        let (from, to) = (at(5.5, 5.5), at(15.5, 5.5));
        let path = walls.path(from, to);
        assert!(path.len() > 1, "{path:?}");
        assert_eq!(path.last(), Some(&to));
        assert!(walkable(&walls, from, &path), "{path:?}");
        // Around its south end.
        assert!(path.iter().any(|p| p.y >= at(0.0, 15.0).y), "{path:?}");
    }

    #[test]
    fn a_blocked_end_gives_way_to_the_open_spot_before_it() {
        // A house 4 × 4 m, 10 m in; a click on its middle from the west.
        let walls = walls((10..14).flat_map(|x| (5..9).map(move |y| (x, y))));
        let from = at(2.5, 7.0);
        let path = walls.path(from, at(12.0, 7.0));
        let end = *path.last().unwrap();
        assert!(!walls.walk.blocked_at(walls.local(end)));
        // Just before its west wall, on the line clicked along.
        assert!(
            (at(9.5, 7.0).x..at(10.0, 7.0).x).contains(&end.x),
            "{end:?}"
        );
        assert_eq!(end.y, at(0.0, 7.0).y);
        assert!(walkable(&walls, from, &path));
    }

    #[test]
    fn with_no_way_round_the_walk_stops_at_the_wall() {
        // A wall across the whole map, 10 m in.
        let walls = walls((0..20).map(|y| (10, y)));
        let from = at(2.5, 7.0);
        let path = walls.path(from, at(20.5, 7.0));
        assert_eq!(path.len(), 1, "{path:?}");
        let end = path[0];
        assert!(
            (at(9.5, 7.0).x..at(10.0, 7.0).x).contains(&end.x),
            "{end:?}"
        );
        assert!(walkable(&walls, from, &path));
        // From against the wall, nowhere.
        let stuck = walls.path(end, at(20.5, 7.0));
        assert!(!goes_somewhere(&stuck, end), "{stuck:?}");
    }

    #[test]
    fn a_straight_line_reaches_up_to_the_first_wall() {
        assert_eq!(reach(None, at(1.0, 1.0), at(9.0, 1.0)), at(9.0, 1.0));
        // A wall across the whole map, 10 m in.
        let walls = walls((0..20).map(|y| (10, y)));
        let from = at(2.5, 7.0);
        let end = walls.reach(from, at(20.5, 7.0));
        assert!(
            (at(9.75, 7.0).x..at(10.0, 7.0).x).contains(&end.x),
            "{end:?}"
        );
        assert!(walls.clear(from, end));
        // Against it: no further, so where we stand.
        assert_eq!(walls.reach(end, at(20.5, 7.0)), end);
        // Away from it, all the way.
        assert_eq!(walls.reach(end, from), from);
    }

    #[test]
    fn a_path_short_of_a_spot_stands_before_it_on_the_last_leg() {
        assert_eq!(
            path_short_of(None, at(1.0, 1.0), at(1.0, 6.0), 100.0),
            [at(1.0, 5.0)]
        );
        // Already near enough: no walk.
        assert!(path_short_of(None, at(1.0, 1.0), at(1.0, 1.5), 100.0).is_empty());
        // Around the wall, the last leg comes from its south end.
        let walls = walls((0..15).map(|y| (10, y)));
        let (from, item) = (at(5.5, 5.5), at(15.5, 5.5));
        let path = path_short_of(Some(&walls), from, item, 50.0);
        let end = *path.last().unwrap();
        assert!((end.distance(item) - 50.0).abs() < 1.0, "{path:?}");
        assert!(walkable(&walls, from, &path));
        assert!(goes_somewhere(&path, from));
        assert!(!goes_somewhere(&[], from));
    }
}

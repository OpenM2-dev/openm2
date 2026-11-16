//! Who is where, in square cells as wide as the view radius, so finding
//! what is near a point looks at a few cells instead of the whole world
//! (`docs/specs/server/world.md`, "View").
//!
//! Players are kept where they last looked around, monsters at the end
//! point of their walk: the positions the view rules use. Groups are kept
//! at their centre (a pack's moves each time it comes back), ground items
//! where they lie.

use std::collections::{BTreeMap, BTreeSet};

use crate::time::ConnId;

/// A group of monsters, by its place in the server's list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct GroupId(pub u32);

/// What stands in one cell.
#[derive(Debug, Default)]
struct Cell {
    players: BTreeSet<ConnId>,
    mobs: BTreeSet<u32>,
    groups: BTreeSet<GroupId>,
    ground: BTreeSet<u32>,
}

impl Cell {
    fn is_empty(&self) -> bool {
        self.players.is_empty()
            && self.mobs.is_empty()
            && self.groups.is_empty()
            && self.ground.is_empty()
    }
}

/// The cells, by (east, south) index.
#[derive(Debug)]
pub(super) struct Grid {
    side: i32,
    cells: BTreeMap<(i32, i32), Cell>,
}

impl Grid {
    /// An empty grid with cells `view_radius` wide (at least 10 m).
    pub fn new(view_radius: i32) -> Self {
        Self {
            side: view_radius.max(1_000),
            cells: BTreeMap::new(),
        }
    }

    fn key(&self, at: (i32, i32)) -> (i32, i32) {
        (at.0.div_euclid(self.side), at.1.div_euclid(self.side))
    }

    fn cell(&mut self, at: (i32, i32)) -> &mut Cell {
        let key = self.key(at);
        self.cells.entry(key).or_default()
    }

    fn tidy(&mut self, at: (i32, i32)) {
        let key = self.key(at);
        if self.cells.get(&key).is_some_and(Cell::is_empty) {
            self.cells.remove(&key);
        }
    }

    pub fn insert_player(&mut self, conn: ConnId, at: (i32, i32)) {
        self.cell(at).players.insert(conn);
    }

    pub fn remove_player(&mut self, conn: ConnId, at: (i32, i32)) {
        self.cell(at).players.remove(&conn);
        self.tidy(at);
    }

    pub fn move_player(&mut self, conn: ConnId, from: (i32, i32), to: (i32, i32)) {
        if self.key(from) != self.key(to) {
            self.remove_player(conn, from);
            self.insert_player(conn, to);
        }
    }

    pub fn insert_mob(&mut self, vid: u32, at: (i32, i32)) {
        self.cell(at).mobs.insert(vid);
    }

    pub fn remove_mob(&mut self, vid: u32, at: (i32, i32)) {
        self.cell(at).mobs.remove(&vid);
        self.tidy(at);
    }

    pub fn move_mob(&mut self, vid: u32, from: (i32, i32), to: (i32, i32)) {
        if self.key(from) != self.key(to) {
            self.remove_mob(vid, from);
            self.insert_mob(vid, to);
        }
    }

    pub fn insert_group(&mut self, group: GroupId, at: (i32, i32)) {
        self.cell(at).groups.insert(group);
    }

    pub fn remove_group(&mut self, group: GroupId, at: (i32, i32)) {
        self.cell(at).groups.remove(&group);
        self.tidy(at);
    }

    /// A pack's group comes back somewhere else.
    pub fn move_group(&mut self, group: GroupId, from: (i32, i32), to: (i32, i32)) {
        if self.key(from) != self.key(to) {
            self.cell(from).groups.remove(&group);
            self.tidy(from);
            self.insert_group(group, to);
        }
    }

    pub fn insert_ground(&mut self, vid: u32, at: (i32, i32)) {
        self.cell(at).ground.insert(vid);
    }

    pub fn remove_ground(&mut self, vid: u32, at: (i32, i32)) {
        self.cell(at).ground.remove(&vid);
        self.tidy(at);
    }

    /// The cells that hold every point within `distance` of `at` (and some
    /// beyond it: callers check the distance themselves).
    fn around(&self, at: (i32, i32), distance: i32) -> impl Iterator<Item = &Cell> {
        let low = self.key((at.0.saturating_sub(distance), at.1.saturating_sub(distance)));
        let high = self.key((at.0.saturating_add(distance), at.1.saturating_add(distance)));
        (low.0..=high.0).flat_map(move |x| {
            self.cells
                .range((x, low.1)..=(x, high.1))
                .map(|(_, cell)| cell)
        })
    }

    /// The players in the cells around `at`.
    pub fn players_around(&self, at: (i32, i32), distance: i32) -> BTreeSet<ConnId> {
        self.around(at, distance)
            .flat_map(|cell| cell.players.iter().copied())
            .collect()
    }

    /// The monsters in the cells around `at`.
    pub fn mobs_around(&self, at: (i32, i32), distance: i32) -> BTreeSet<u32> {
        self.around(at, distance)
            .flat_map(|cell| cell.mobs.iter().copied())
            .collect()
    }

    /// The ground items in the cells around `at`.
    pub fn ground_around(&self, at: (i32, i32), distance: i32) -> BTreeSet<u32> {
        self.around(at, distance)
            .flat_map(|cell| cell.ground.iter().copied())
            .collect()
    }

    /// The groups whose centres are in the cells around `at`.
    pub fn groups_around(&self, at: (i32, i32), distance: i32) -> BTreeSet<GroupId> {
        self.around(at, distance)
            .flat_map(|cell| cell.groups.iter().copied())
            .collect()
    }
}

/// Whether `a` and `b` are at most `distance` apart.
pub(super) fn within(a: (i32, i32), b: (i32, i32), distance: i32) -> bool {
    // Clients send any i32: squares of their differences need 128 bits.
    let dx = i128::from(a.0) - i128::from(b.0);
    let dy = i128::from(a.1) - i128::from(b.1);
    let distance = i128::from(distance);
    dx * dx + dy * dy <= distance * distance
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nearby_cells_are_found_and_far_ones_not() {
        let mut grid = Grid::new(10_000);
        grid.insert_player(ConnId(1), (5_000, 5_000));
        grid.insert_player(ConnId(2), (14_000, -3_000));
        grid.insert_player(ConnId(3), (40_000, 5_000));
        grid.insert_mob(7, (-9_999, 0));
        let near = grid.players_around((5_000, 5_000), 10_000);
        assert_eq!(near.into_iter().collect::<Vec<_>>(), [ConnId(1), ConnId(2)]);
        assert_eq!(grid.mobs_around((0, 0), 10_000).len(), 1);

        grid.move_player(ConnId(3), (40_000, 5_000), (6_000, 5_000));
        assert_eq!(grid.players_around((5_000, 5_000), 10_000).len(), 3);
        grid.remove_player(ConnId(1), (5_000, 5_000));
        grid.remove_player(ConnId(2), (14_000, -3_000));
        grid.remove_player(ConnId(3), (6_000, 5_000));
        grid.remove_mob(7, (-9_999, 0));
        assert!(grid.cells.is_empty());
    }

    #[test]
    fn the_edges_of_the_world_do_not_overflow() {
        let mut grid = Grid::new(10_000);
        grid.insert_player(ConnId(1), (i32::MAX, i32::MIN));
        assert_eq!(grid.players_around((i32::MAX, i32::MIN), 10_000).len(), 1);
        assert!(within((i32::MAX, 0), (i32::MAX - 3, 4), 5));
        assert!(!within((i32::MIN, 0), (i32::MAX, 0), i32::MAX));
    }
}

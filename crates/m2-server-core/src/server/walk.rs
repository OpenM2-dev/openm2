//! Walks: how long one takes, where a walker is along it, and the 72
//! directions monsters walk in (`docs/specs/protocol/openm2/movement.md`).

use std::cmp::Ordering;

use crate::time::Millis;

/// A walk in a straight line, from `from` to wherever the walker's position
/// says it ends.
#[derive(Debug, Clone, Copy)]
pub(super) struct Walk {
    pub from: (i32, i32),
    pub started: Millis,
    pub duration: u32,
}

/// Where a walker heading for `to` is at `now`: along its walk, or at `to`
/// once the walk is over (or without one).
pub(super) fn position_at(to: (i32, i32), walk: Option<Walk>, now: Millis) -> (i32, i32) {
    let Some(walk) = walk else {
        return to;
    };
    let elapsed = now.since(walk.started);
    if walk.duration == 0 || elapsed >= u64::from(walk.duration) {
        return to;
    }
    let part = elapsed as f64 / f64::from(walk.duration);
    let along = |from: i32, to: i32| {
        // Between two i32 values, so it fits an i32.
        (f64::from(from) + (f64::from(to) - f64::from(from)) * part).round() as i32
    };
    (along(walk.from.0, to.0), along(walk.from.1, to.1))
}

/// Milliseconds of a walk still to go at `now`.
pub(super) fn remaining(walk: Option<Walk>, now: Millis) -> u32 {
    walk.map_or(0, |walk| {
        let elapsed = now.since(walk.started);
        u32::try_from(u64::from(walk.duration).saturating_sub(elapsed)).unwrap_or(0)
    })
}

/// Percent of the run time a walk takes at `move_speed` (`movement.md`,
/// "Speed"): 100 at speed 100, more below it, less above.
pub fn walk_time_percent(move_speed: u8) -> u64 {
    let speed = u64::from(move_speed);
    match speed.cmp(&100) {
        Ordering::Less => 200 - speed,
        Ordering::Equal => 100,
        Ordering::Greater => 10_000 / speed,
    }
}

/// Milliseconds a walk of `distance` world units takes for a character that
/// runs `run_speed` units a second at movement speed 100, at `move_speed`.
pub fn walk_duration(distance: f64, run_speed: u32, move_speed: u8) -> u32 {
    // Whole milliseconds, like the client; absurd distances saturate.
    let base = (distance / f64::from(run_speed.max(1)) * 1000.0) as u64;
    let duration = base.saturating_mul(walk_time_percent(move_speed)) / 100;
    u32::try_from(duration).unwrap_or(u32::MAX)
}

/// The length of (`dx`, `dy`). `sqrt` is exactly rounded on every platform,
/// unlike `hypot`, so the server's numbers are the same everywhere.
pub(super) fn length(dx: i64, dy: i64) -> f64 {
    let square = (dx as f64) * (dx as f64) + (dy as f64) * (dy as f64);
    square.sqrt()
}

/// The protocol's directions, in steps of 5 degrees: (east, south) × 10,000
/// for rotation 0 (south, +y) to 71, turning towards east (+x). Written out
/// so no platform's sine and cosine can change the server's numbers.
pub(super) const DIRECTIONS: [(i32, i32); 72] = [
    (0, 10000),
    (872, 9962),
    (1736, 9848),
    (2588, 9659),
    (3420, 9397),
    (4226, 9063),
    (5000, 8660),
    (5736, 8192),
    (6428, 7660),
    (7071, 7071),
    (7660, 6428),
    (8192, 5736),
    (8660, 5000),
    (9063, 4226),
    (9397, 3420),
    (9659, 2588),
    (9848, 1736),
    (9962, 872),
    (10000, 0),
    (9962, -872),
    (9848, -1736),
    (9659, -2588),
    (9397, -3420),
    (9063, -4226),
    (8660, -5000),
    (8192, -5736),
    (7660, -6428),
    (7071, -7071),
    (6428, -7660),
    (5736, -8192),
    (5000, -8660),
    (4226, -9063),
    (3420, -9397),
    (2588, -9659),
    (1736, -9848),
    (872, -9962),
    (0, -10000),
    (-872, -9962),
    (-1736, -9848),
    (-2588, -9659),
    (-3420, -9397),
    (-4226, -9063),
    (-5000, -8660),
    (-5736, -8192),
    (-6428, -7660),
    (-7071, -7071),
    (-7660, -6428),
    (-8192, -5736),
    (-8660, -5000),
    (-9063, -4226),
    (-9397, -3420),
    (-9659, -2588),
    (-9848, -1736),
    (-9962, -872),
    (-10000, 0),
    (-9962, 872),
    (-9848, 1736),
    (-9659, 2588),
    (-9397, 3420),
    (-9063, 4226),
    (-8660, 5000),
    (-8192, 5736),
    (-7660, 6428),
    (-7071, 7071),
    (-6428, 7660),
    (-5736, 8192),
    (-5000, 8660),
    (-4226, 9063),
    (-3420, 9397),
    (-2588, 9659),
    (-1736, 9848),
    (-872, 9962),
];

/// A direction in steps of 5 degrees as degrees, as the messages carry it.
pub(super) fn degrees(rotation: u8) -> u16 {
    u16::from(rotation) * 5 % 360
}

/// The direction (0 to 71) closest to the way from `from` to `to`; 0 when
/// they are the same point.
pub(super) fn direction(from: (i32, i32), to: (i32, i32)) -> u8 {
    let dx = i64::from(to.0) - i64::from(from.0);
    let dy = i64::from(to.1) - i64::from(from.1);
    let mut best = (i64::MIN, 0u8);
    for (step, (east, south)) in (0u8..).zip(DIRECTIONS) {
        let along = dx * i64::from(east) + dy * i64::from(south);
        if along > best.0 {
            best = (along, step);
        }
    }
    best.1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_matches_sine_and_cosine() {
        for (step, (east, south)) in DIRECTIONS.iter().enumerate() {
            let angle = (step as f64 * 5.0).to_radians();
            assert_eq!(*east, (angle.sin() * 10_000.0).round() as i32, "{step}");
            assert_eq!(*south, (angle.cos() * 10_000.0).round() as i32, "{step}");
        }
    }

    #[test]
    fn directions_point_the_protocols_way() {
        assert_eq!(direction((0, 0), (0, 100)), 0);
        assert_eq!(direction((0, 0), (100, 0)), 18);
        assert_eq!(direction((0, 0), (0, -100)), 36);
        assert_eq!(direction((0, 0), (-100, 0)), 54);
        assert_eq!(direction((0, 0), (100, 100)), 9);
        assert_eq!(direction((5, 5), (5, 5)), 0);
    }

    #[test]
    fn a_walk_is_followed_over_time() {
        let walk = Some(Walk {
            from: (0, 0),
            started: Millis(1_000),
            duration: 1_000,
        });
        assert_eq!(position_at((100, -50), walk, Millis(1_500)), (50, -25));
        assert_eq!(position_at((100, -50), walk, Millis(9_000)), (100, -50));
        assert_eq!(remaining(walk, Millis(1_250)), 750);
        assert_eq!(remaining(walk, Millis(5_000)), 0);
        assert_eq!(remaining(None, Millis(0)), 0);
    }
}

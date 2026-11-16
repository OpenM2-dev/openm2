//! Our character steered by the movement keys in `openm2 play`
//! (`docs/specs/render.md`, "Walking with the keys"): W, A, S and D, or the
//! arrow keys, run away from the camera, towards it and to its sides, a
//! straight walk at a time up to the walls, sliding along those met at a
//! slant (openm2's rules).

use m2_core::{Command, Millis, Point, World};
use winit::keyboard::{Key, NamedKey};

use crate::walls::{self, Walls};

/// How far ahead of us a walk ends, in centimetres.
const AHEAD: f64 = 500.0;

/// The next walk is asked for when less than this is left of the last, in
/// centimetres, or when the keys and the camera point [`TURN`] degrees away
/// from where they did; but never sooner than [`AGAIN_MS`] after it.
const LEFT: f64 = 200.0;
const TURN: f64 = 8.0;
const AGAIN_MS: u64 = 100;

/// Ways tried beside the one the keys point, in degrees to either side,
/// nearest first, when that one is walled within [`SLIDE`]: a wall met at
/// a slant is slid along, one met head on stops us.
const TURNS: [f64; 2] = [25.0, 50.0];

/// A way beside the keys' one is taken only when it goes this far, in
/// centimetres.
const SLIDE: f64 = 100.0;

/// A movement key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Way {
    /// W or ↑: away from the camera.
    Forward,
    /// S or ↓: towards it.
    Back,
    /// A or ←.
    Left,
    /// D or →.
    Right,
}

impl Way {
    /// The movement key `key` is, if it is one.
    pub(crate) fn of(key: &Key) -> Option<Way> {
        let is = |c: &str| matches!(key, Key::Character(k) if k.eq_ignore_ascii_case(c));
        Some(match key {
            Key::Named(NamedKey::ArrowUp) => Way::Forward,
            Key::Named(NamedKey::ArrowDown) => Way::Back,
            Key::Named(NamedKey::ArrowLeft) => Way::Left,
            Key::Named(NamedKey::ArrowRight) => Way::Right,
            _ if is("w") => Way::Forward,
            _ if is("s") => Way::Back,
            _ if is("a") => Way::Left,
            _ if is("d") => Way::Right,
            _ => return None,
        })
    }
}

/// The movement keys held.
#[derive(Debug, Default, Clone, Copy)]
struct Held {
    forward: bool,
    back: bool,
    left: bool,
    right: bool,
}

impl Held {
    fn key(&mut self, way: Way) -> &mut bool {
        match way {
            Way::Forward => &mut self.forward,
            Way::Back => &mut self.back,
            Way::Left => &mut self.left,
            Way::Right => &mut self.right,
        }
    }
}

/// The walk the keys asked for last.
#[derive(Debug, Clone, Copy)]
struct Sent {
    /// Where the keys pointed, in degrees (as `Point::angle_to` gives
    /// them: 0 south, 90 east).
    aim: f64,
    /// Where the walk ends.
    end: Point,
    /// When it was asked for.
    at: Millis,
}

/// Our character's walks from the movement keys.
#[derive(Debug, Default)]
pub(crate) struct Steering {
    held: Held,
    /// The walk asked for last, while the keys lead.
    sent: Option<Sent>,
}

impl Steering {
    /// `way` went down: true if it was not held already.
    pub fn press(&mut self, way: Way) -> bool {
        !std::mem::replace(self.held.key(way), true)
    }

    /// `way` came up.
    pub fn release(&mut self, way: Way) {
        *self.held.key(way) = false;
    }

    /// Every key came up (the window lost the focus).
    pub fn release_all(&mut self) {
        self.held = Held::default();
    }

    /// The server refused our walk and put us at `at`: the next one starts
    /// from there, no sooner than [`AGAIN_MS`] after the last.
    pub fn refused(&mut self, at: Point) {
        if let Some(sent) = &mut self.sent {
            sent.end = at;
        }
    }

    /// One frame: the walk the keys ask for now, with the camera at `yaw`
    /// and the map's `walls`. `others` is true while something else leads
    /// our character (a fight, a pick-up, a talk, a skill, a scripted walk):
    /// the keys wait for it. Letting go of the keys stops the walk they
    /// asked for where we are.
    pub fn frame(
        &mut self,
        world: &World,
        now: Millis,
        yaw: f32,
        walls: Option<&Walls>,
        others: bool,
    ) -> Vec<Command> {
        let Some(me) = world.my_entity() else {
            self.sent = None;
            return Vec::new();
        };
        if me.dead || others {
            self.sent = None;
            return Vec::new();
        }
        let Some(aim) = self.aim(yaw) else {
            return match self.sent.take() {
                Some(_) if me.track.is_moving(now) => vec![Command::Stop],
                _ => Vec::new(),
            };
        };
        let here = me.track.point(now);
        if let Some(sent) = self.sent {
            let turned = apart(aim, sent.aim) >= TURN;
            if now.since(sent.at) < AGAIN_MS || (!turned && here.distance(sent.end) >= LEFT) {
                return Vec::new();
            }
        }
        let end = end(walls, here, aim);
        self.sent = Some(Sent { aim, end, at: now });
        // Nowhere to go, or on the way there already.
        let going = me.track.is_moving(now) && me.track.to.distance(end) < walls::NOWHERE;
        if going || end.distance(here) < walls::NOWHERE {
            return Vec::new();
        }
        vec![Command::MoveTo { path: vec![end] }]
    }

    /// Where the keys held point with the camera at `yaw` (as
    /// `m2_render::Camera` has it: 0 looks north, more turns it
    /// anticlockwise seen from above), in degrees as `Point::angle_to`
    /// gives them. `None` when no key is held, or only keys that undo each
    /// other.
    fn aim(&self, yaw: f32) -> Option<f64> {
        let held = self.held;
        let ahead = f64::from(i8::from(held.forward) - i8::from(held.back));
        let side = f64::from(i8::from(held.right) - i8::from(held.left));
        if ahead == 0.0 && side == 0.0 {
            return None;
        }
        // In the world (y to the south) the camera looks along
        // (−sin yaw, −cos yaw), and its right is (cos yaw, −sin yaw).
        let (sin, cos) = f64::from(yaw).sin_cos();
        let x = -sin * ahead + cos * side;
        let y = -cos * ahead - sin * side;
        Some(x.atan2(y).to_degrees().rem_euclid(360.0))
    }
}

/// Degrees between two directions, 0 to 180.
fn apart(a: f64, b: f64) -> f64 {
    let turn = (a - b).rem_euclid(360.0);
    turn.min(360.0 - turn)
}

/// The point `distance` centimetres from `from` towards `aim` degrees.
fn towards(from: Point, aim: f64, distance: f64) -> Point {
    let (sin, cos) = aim.to_radians().sin_cos();
    Point::new(
        (f64::from(from.x) + sin * distance).round() as i32,
        (f64::from(from.y) + cos * distance).round() as i32,
    )
}

/// Where a walk from `here` towards `aim` ends: [`AHEAD`] along it, or up
/// to the first wall. When that is under [`SLIDE`], the way turned the
/// nearest of [`TURNS`] to either side that goes at least that far (the
/// farther of the two); else the short way, or `here`.
fn end(walls: Option<&Walls>, here: Point, aim: f64) -> Point {
    let reach = |aim: f64| walls::reach(walls, here, towards(here, aim, AHEAD));
    let straight = reach(aim);
    if straight.distance(here) >= SLIDE {
        return straight;
    }
    for turn in TURNS {
        let (left, right) = (reach(aim - turn), reach(aim + turn));
        let farther = if right.distance(here) > left.distance(here) {
            right
        } else {
            left
        };
        if farther.distance(here) >= SLIDE {
            return farther;
        }
    }
    straight
}

#[cfg(test)]
mod tests {
    use std::f32::consts::FRAC_PI_2;

    use m2_content::WalkMap;
    use m2_core::{Entity, EntityKind, Me, MoveKind, Track};
    use winit::keyboard::SmolStr;

    use super::*;

    const ME: u32 = 1;

    /// Where the walled test map lies in the world.
    const BASE: Point = Point {
        x: 100_000,
        y: 200_000,
    };

    fn world(track: Track) -> World {
        let mut world = World {
            me: Some(Me {
                vid: ME,
                name: "Alice".to_owned(),
                class: 0,
                empire: m2_core::Empire::Shinsoo,
                position: (0, 0, 0),
            }),
            ..World::default()
        };
        world.entities.insert(
            ME,
            Entity {
                vid: ME,
                kind: EntityKind::Player,
                race: 0,
                track,
                angle: 0,
                z: 0,
                action: MoveKind::Wait,
                argument: 0,
                move_speed: 100,
                attack_speed: 100,
                state: 0,
                affects: 0,
                parts: [0; 4],
                dead: false,
                health: None,
                info: None,
                mount: 0,
                skills: Vec::new(),
                line: None,
            },
        );
        world
    }

    fn standing(at: Point) -> World {
        world(Track::still(at, Millis(0)))
    }

    /// Walking from `from` to `to`, leaving at `start`, for a second.
    fn walking(from: Point, to: Point, start: u64) -> World {
        world(Track {
            from,
            to,
            start: Millis(start),
            duration: 1_000,
        })
    }

    /// Where the walk a frame asked for ends.
    fn walk(commands: &[Command]) -> Option<Point> {
        match commands {
            [Command::MoveTo { path }] if path.len() == 1 => path.first().copied(),
            _ => None,
        }
    }

    /// The point `x`, `y` metres into the walled test map.
    fn at(x: f64, y: f64) -> Point {
        Point::new(BASE.x + (x * 100.0) as i32, BASE.y + (y * 100.0) as i32)
    }

    #[test]
    fn w_a_s_d_and_the_arrows_are_the_movement_keys() {
        let character = |c: &str| Key::Character(SmolStr::new(c));
        for (key, way) in [
            (character("w"), Way::Forward),
            (character("W"), Way::Forward),
            (character("s"), Way::Back),
            (character("a"), Way::Left),
            (character("d"), Way::Right),
            (Key::Named(NamedKey::ArrowUp), Way::Forward),
            (Key::Named(NamedKey::ArrowDown), Way::Back),
            (Key::Named(NamedKey::ArrowLeft), Way::Left),
            (Key::Named(NamedKey::ArrowRight), Way::Right),
        ] {
            assert_eq!(Way::of(&key), Some(way), "{key:?}");
        }
        assert_eq!(Way::of(&character("q")), None);
        assert_eq!(Way::of(&Key::Named(NamedKey::Space)), None);
    }

    #[test]
    fn the_keys_run_away_from_the_camera_towards_it_and_to_its_sides() {
        let here = Point::new(10_000, 10_000);
        let world = standing(here);
        // The camera at yaw 0 looks north.
        for (ways, end) in [
            (&[Way::Forward][..], Point::new(10_000, 9_500)),
            (&[Way::Back], Point::new(10_000, 10_500)),
            (&[Way::Left], Point::new(9_500, 10_000)),
            (&[Way::Right], Point::new(10_500, 10_000)),
            (&[Way::Forward, Way::Right], Point::new(10_354, 9_646)),
        ] {
            let mut steering = Steering::default();
            for &way in ways {
                steering.press(way);
            }
            let commands = steering.frame(&world, Millis(0), 0.0, None, false);
            assert_eq!(walk(&commands), Some(end), "{ways:?}");
        }
        // Keys that undo each other: nowhere.
        let mut steering = Steering::default();
        steering.press(Way::Forward);
        steering.press(Way::Back);
        assert!(
            steering
                .frame(&world, Millis(0), 0.0, None, false)
                .is_empty()
        );
        // Turned a quarter anticlockwise, the camera looks west.
        let mut steering = Steering::default();
        steering.press(Way::Forward);
        let commands = steering.frame(&world, Millis(0), FRAC_PI_2, None, false);
        assert_eq!(walk(&commands), Some(Point::new(9_500, 10_000)));
    }

    #[test]
    fn walks_are_asked_for_again_as_they_run_out_or_the_keys_turn() {
        let here = Point::new(10_000, 10_000);
        let mut steering = Steering::default();
        assert!(steering.press(Way::Forward));
        assert!(!steering.press(Way::Forward), "held already");
        let end = Point::new(10_000, 9_500);
        let commands = steering.frame(&standing(here), Millis(0), 0.0, None, false);
        assert_eq!(walk(&commands), Some(end));
        // Over 2 m left: nothing new, nor with the camera turned 6°.
        let on = walking(here, end, 0);
        assert!(
            steering
                .frame(&on, Millis(500), 0.0, None, false)
                .is_empty()
        );
        assert!(
            steering
                .frame(&on, Millis(550), 0.1, None, false)
                .is_empty()
        );
        // Under 2 m left: the next 5 m from where we are.
        let next = Point::new(10_000, 9_150);
        let commands = steering.frame(&on, Millis(700), 0.0, None, false);
        assert_eq!(walk(&commands), Some(next));
        // The camera turned 17°: at once, but not within a tenth of a
        // second of the last walk. Turned anticlockwise, forward bends west.
        let on = walking(Point::new(10_000, 9_650), next, 700);
        assert!(
            steering
                .frame(&on, Millis(750), 0.3, None, false)
                .is_empty()
        );
        let turned = walk(&steering.frame(&on, Millis(800), 0.3, None, false));
        assert!(turned.is_some_and(|p| p.x < 10_000), "{turned:?}");
    }

    #[test]
    fn letting_go_stops_the_walk_once() {
        let here = Point::new(10_000, 10_000);
        let mut steering = Steering::default();
        steering.press(Way::Right);
        let end = Point::new(10_500, 10_000);
        let commands = steering.frame(&standing(here), Millis(0), 0.0, None, false);
        assert_eq!(walk(&commands), Some(end));
        steering.release(Way::Right);
        let on = walking(here, end, 0);
        assert_eq!(
            steering.frame(&on, Millis(300), 0.0, None, false),
            [Command::Stop]
        );
        assert!(
            steering
                .frame(&on, Millis(400), 0.0, None, false)
                .is_empty()
        );
        // A walk that is over is not stopped.
        steering.press(Way::Right);
        assert!(walk(&steering.frame(&standing(here), Millis(500), 0.0, None, false)).is_some());
        steering.release_all();
        let there = world(Track::still(end, Millis(1_500)));
        assert!(
            steering
                .frame(&there, Millis(2_000), 0.0, None, false)
                .is_empty()
        );
    }

    #[test]
    fn the_keys_wait_while_something_else_leads_or_we_are_dead() {
        let here = Point::new(10_000, 10_000);
        let mut steering = Steering::default();
        steering.press(Way::Forward);
        assert!(
            steering
                .frame(&standing(here), Millis(0), 0.0, None, true)
                .is_empty()
        );
        let mut dead = standing(here);
        if let Some(me) = dead.entities.get_mut(&ME) {
            me.dead = true;
        }
        assert!(
            steering
                .frame(&dead, Millis(0), 0.0, None, false)
                .is_empty()
        );
        // Once nothing else leads, they walk at once.
        let commands = steering.frame(&standing(here), Millis(10), 0.0, None, false);
        assert!(walk(&commands).is_some());
        // Letting go of them while a fight walks us stops nothing: the walk
        // is not theirs.
        let fight = walking(here, Point::new(10_300, 10_000), 20);
        assert!(
            steering
                .frame(&fight, Millis(20), 0.0, None, true)
                .is_empty()
        );
        steering.release_all();
        assert!(
            steering
                .frame(&fight, Millis(200), 0.0, None, false)
                .is_empty()
        );
    }

    #[test]
    fn after_a_refused_walk_the_next_starts_where_the_server_put_us() {
        let here = Point::new(10_000, 10_000);
        let mut steering = Steering::default();
        steering.press(Way::Forward);
        assert!(walk(&steering.frame(&standing(here), Millis(0), 0.0, None, false)).is_some());
        // Put back 1 m along, 50 ms in: the next walk 50 ms later.
        let back = Point::new(10_000, 9_900);
        steering.refused(back);
        let there = world(Track::still(back, Millis(50)));
        assert!(
            steering
                .frame(&there, Millis(60), 0.0, None, false)
                .is_empty()
        );
        let commands = steering.frame(&there, Millis(100), 0.0, None, false);
        assert_eq!(walk(&commands), Some(Point::new(10_000, 9_400)));
    }

    #[test]
    fn walls_end_walks_and_those_met_at_a_slant_are_slid_along() {
        // A wall north to south across a map 30 × 20 m, 10 m in.
        let mut mask = WalkMap::open(30, 20).unwrap();
        for y in 0..20 {
            mask.set_blocked(10, y, true);
        }
        let walls = Walls::new(mask, BASE);
        // Running east from 3.5 m before it: up to it.
        let from = at(6.5, 10.0);
        let mut steering = Steering::default();
        steering.press(Way::Right);
        let commands = steering.frame(&standing(from), Millis(0), 0.0, Some(&walls), false);
        let end = walk(&commands).unwrap();
        assert_eq!(end, at(9.75, 10.0));
        assert!(walls.clear(from, end));
        // Against it, head on: no walk.
        let mut steering = Steering::default();
        steering.press(Way::Right);
        assert!(
            steering
                .frame(&standing(end), Millis(0), 0.0, Some(&walls), false)
                .is_empty()
        );
        // At a slant, to the north-east: along it, to the north.
        let mut steering = Steering::default();
        steering.press(Way::Forward);
        steering.press(Way::Right);
        let commands = steering.frame(&standing(end), Millis(0), 0.0, Some(&walls), false);
        let slid = walk(&commands).unwrap();
        assert!(slid.y < end.y - 100 && slid.x < at(10.0, 0.0).x, "{slid:?}");
        assert!(walls.clear(end, slid));
    }
}

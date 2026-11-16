//! Items on the ground in `openm2 play` (`docs/specs/render.md`, "Items on
//! the ground"): the model each one shows and how it lies, its name over
//! it, and walking up to one to pick it up
//! (`docs/specs/protocol/openm2/ground-items.md`).

use std::collections::BTreeSet;

use glam::{Mat4, Vec3};
use m2_assets::actor::WEAPON;
use m2_assets::items::Items;
use m2_core::{Command, GroundItem, Millis, Point, World};
use m2_render::Bounds;
use m2_ui::Colour;

use crate::interface::text;
use crate::walls::{self, Walls};

/// We pick an item up once our character is this close to it, in
/// centimetres (openm2's choice; the server allows 5 m).
pub(crate) const PICK_UP_REACH: f64 = 150.0;

/// Where the walls keep us from coming that close, we still ask for an item
/// this close (centimetres): within the server's 5 m, less a margin for
/// where it has us (`docs/specs/protocol/openm2/ground-items.md`).
pub(crate) const ASK_REACH: f64 = 450.0;

/// Walking up to an item or an NPC, we aim this far short of it, coming
/// along the last leg of the path.
pub(crate) const STAND: f64 = 50.0;

/// Items closer than this (centimetres) to the first of a group lie
/// together: their names share one place.
const TOGETHER: f32 = 100.0;

/// The name over an item anyone, or we, may pick up; and over one that
/// is someone else's.
const NAME_COLOUR: Colour = [235, 235, 235, 255];
const OTHERS_COLOUR: Colour = [150, 150, 150, 255];

/// The model `vnum` shows on the ground: the item table's, else the bag the
/// table names, else none (empty); and whether it is a weapon, which lies
/// down.
pub(crate) fn ground_model(items: Option<&Items>, vnum: u32) -> (String, bool) {
    let info = items.and_then(|items| items.get(vnum));
    let bag = || {
        items
            .and_then(|items| items.bag.as_ref())
            .map(|bag| bag.escape_ascii().to_string())
            .unwrap_or_default()
    };
    let path = info
        .and_then(|info| info.model.as_ref())
        .map_or_else(bag, |m| m.escape_ascii().to_string());
    (path, info.is_some_and(|info| info.record.kind == WEAPON))
}

/// How a model lies on the ground, and its box lying there, around the
/// item's spot (the origin): a weapon (made standing on its grip, the
/// blade up) is laid flat, its middle on the spot and its underside on the
/// ground; anything else stands as it was made.
pub(crate) fn lying(bounds: Bounds, weapon: bool) -> (Mat4, Bounds) {
    if !weapon {
        return (Mat4::IDENTITY, bounds);
    }
    let down = Mat4::from_rotation_x(std::f32::consts::FRAC_PI_2);
    let (min, max) = (bounds.min, bounds.max);
    let mut laid: Option<Bounds> = None;
    for corner in 0..8 {
        let pick = |bit: u32, low: f32, high: f32| if corner & bit == 0 { low } else { high };
        let point = Vec3::new(
            pick(1, min.x, max.x),
            pick(2, min.y, max.y),
            pick(4, min.z, max.z),
        );
        let p = down.transform_point3(point);
        laid = Some(laid.map_or(Bounds { min: p, max: p }, |b| b.including(p)));
    }
    let Some(laid) = laid else {
        return (Mat4::IDENTITY, bounds);
    };
    let centre = laid.center();
    let shift = Vec3::new(-centre.x, -centre.y, -laid.min.z);
    let placed = Bounds {
        min: laid.min + shift,
        max: laid.max + shift,
    };
    (Mat4::from_translation(shift) * down, placed)
}

/// Which way an item lies, in radians about the vertical: from its id, so
/// it stays the same and items side by side differ (openm2's choice).
pub(crate) fn turn(vid: u32) -> f32 {
    let degrees = vid.wrapping_mul(2_654_435_761) % 360;
    (degrees as f32).to_radians()
}

/// An item's name: the item table's; else its number.
pub(crate) fn item_name(items: Option<&Items>, vnum: u32) -> String {
    items.and_then(|items| items.get(vnum)).map_or_else(
        || format!("Item {vnum}"),
        |info| text(info.record.locale_name.as_bytes()),
    )
}

/// True if `item` is someone else's: the server named an owner, and not
/// us.
pub(crate) fn others(item: &GroundItem, world: &World) -> bool {
    let me = world.me.as_ref().map(|me| me.name.as_bytes());
    item.owner
        .as_ref()
        .is_some_and(|owner| Some(owner.as_bytes()) != me)
}

/// The colour of an item's name.
pub(crate) fn name_colour(item: &GroundItem, world: &World) -> Colour {
    if others(item, world) {
        OTHERS_COLOUR
    } else {
        NAME_COLOUR
    }
}

/// Items lying together share one place for their names, stacked a line
/// apart (openm2's choice; a monster leaves all its drops on one spot): for
/// each spot, in the order given, the first spot of its group and its line
/// in it, from 0 at the bottom.
pub(crate) fn stacked(spots: &[Vec3]) -> Vec<(usize, usize)> {
    let mut out: Vec<(usize, usize)> = Vec::with_capacity(spots.len());
    for spot in spots {
        let first = out
            .iter()
            .enumerate()
            .find(|(at, (first, _))| {
                at == first
                    && spots
                        .get(*at)
                        .is_some_and(|s| s.distance(*spot) <= TOGETHER)
            })
            .map(|(at, _)| at);
        let place = match first {
            Some(first) => (first, out.iter().filter(|(f, _)| *f == first).count()),
            None => (out.len(), 0),
        };
        out.push(place);
    }
    out
}

/// The nearest item to `here` that we may pick up and did not try yet:
/// for `play --pick-up`.
pub(crate) fn nearest_item(world: &World, here: Point, tried: &BTreeSet<u32>) -> Option<u32> {
    world
        .ground
        .iter()
        .filter(|(vid, item)| !tried.contains(vid) && !others(item, world))
        .map(|(vid, item)| (here.distance(item.at), *vid))
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, vid)| vid)
}

/// Our way to an item we were asked to pick up: walk up to it, then ask
/// the server for it.
#[derive(Debug, Default)]
pub(crate) struct Gatherer {
    /// The item.
    item: Option<u32>,
    /// Where our walk for it ends, and where the item lies.
    aimed: Option<(Point, Point)>,
}

impl Gatherer {
    /// Goes for the ground item `vid`.
    pub fn go_for(&mut self, vid: u32) {
        self.item = Some(vid);
        self.aimed = None;
    }

    /// Gives up: the player asked for something else.
    pub fn stop(&mut self) {
        self.item = None;
        self.aimed = None;
    }

    /// The server refused our walk up to the item and put us at `at`: as
    /// near as we get, so the next frame gives the item up.
    pub fn refused(&mut self, at: Point) {
        if let Some((end, _)) = &mut self.aimed {
            *end = at;
        }
    }

    /// The item we go for.
    pub fn item(&self) -> Option<u32> {
        self.item
    }

    /// One frame: walks up to the item around `walls`
    /// (`Command::MoveTo`), and once in reach stops and picks it up. Gives
    /// up when the item is gone or our character is dead; when the walls
    /// keep us from it, asks for it from as near as they let us if that is
    /// within [`ASK_REACH`], else gives up.
    pub fn frame(&mut self, world: &World, now: Millis, walls: Option<&Walls>) -> Vec<Command> {
        let Some(vid) = self.item else {
            return Vec::new();
        };
        let (Some(item), Some(me)) = (world.ground.get(&vid), world.my_entity()) else {
            self.stop();
            return Vec::new();
        };
        if me.dead {
            self.stop();
            return Vec::new();
        }
        let here = me.track.point(now);
        let walking = me.track.is_moving(now);
        let distance = here.distance(item.at);
        if distance <= PICK_UP_REACH {
            self.stop();
            let mut out = Vec::new();
            if walking {
                out.push(Command::Stop);
            }
            out.push(Command::PickUp { vid });
            return out;
        }
        if let Some((end, aimed)) = self.aimed
            && aimed == item.at
        {
            if walking {
                return Vec::new();
            }
            if end.distance(here) < walls::NOWHERE {
                // As near as the walls let us, and not in reach.
                return self.ask_from_here(vid, distance);
            }
        }
        let path = walls::path_short_of(walls, here, item.at, STAND);
        let Some(&end) = path.last().filter(|_| walls::goes_somewhere(&path, here)) else {
            return self.ask_from_here(vid, distance);
        };
        self.aimed = Some((end, item.at));
        vec![Command::MoveTo { path }]
    }

    /// We get no nearer to the item `vid`, `distance` away: done, asking
    /// for it if it is within [`ASK_REACH`] (the server says no if not).
    fn ask_from_here(&mut self, vid: u32, distance: f64) -> Vec<Command> {
        self.stop();
        if distance <= ASK_REACH {
            vec![Command::PickUp { vid }]
        } else {
            Vec::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use m2_core::{Entity, EntityKind, Me, MoveKind, Track};
    use m2_formats::script::item_list::ItemLine;
    use m2_formats::{ItemRecord, RawStr};

    const ME: u32 = 1;
    const SWORD: u32 = 500;

    fn world(at: Point, track: Track) -> World {
        let mut world = World {
            me: Some(Me {
                vid: ME,
                name: "Alice".to_owned(),
                class: 3,
                empire: m2_core::Empire::Chunjo,
                position: (0, 0, 0),
            }),
            ..World::default()
        };
        world.entities.insert(
            ME,
            Entity {
                vid: ME,
                kind: EntityKind::Player,
                race: 3,
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
        world.ground.insert(
            SWORD,
            GroundItem {
                vnum: 10,
                at,
                owner: None,
            },
        );
        world
    }

    #[test]
    fn a_refused_walk_gives_the_item_up() {
        let mut gatherer = Gatherer::default();
        gatherer.go_for(SWORD);
        let world = world(
            Point::new(1_000, 0),
            Track::still(Point::new(0, 0), Millis(0)),
        );
        assert_eq!(gatherer.frame(&world, Millis(0), None).len(), 1);
        // The server put us back where we stood.
        gatherer.refused(Point::new(0, 0));
        assert!(gatherer.frame(&world, Millis(100), None).is_empty());
        assert_eq!(gatherer.item(), None);
    }

    #[test]
    fn we_walk_up_to_an_item_then_pick_it_up() {
        let item = Point::new(1_000, 0);
        let standing = Track::still(Point::new(0, 0), Millis(0));
        let mut gatherer = Gatherer::default();
        gatherer.go_for(SWORD);
        // 10 m away: walk to 50 cm short of it, once.
        let world = world(item, standing);
        let walk = Command::MoveTo {
            path: vec![Point::new(950, 0)],
        };
        assert_eq!(gatherer.frame(&world, Millis(0), None), vec![walk.clone()]);
        let walking = Track {
            from: Point::new(0, 0),
            to: Point::new(950, 0),
            start: Millis(0),
            duration: 2_000,
        };
        let world = self::world(item, walking);
        assert!(gatherer.frame(&world, Millis(100), None).is_empty());
        // In reach on the way: stop there and pick it up.
        assert_eq!(
            gatherer.frame(&world, Millis(1_800), None),
            vec![Command::Stop, Command::PickUp { vid: SWORD }]
        );
        assert_eq!(gatherer.item(), None);
        // Standing in reach: at once; gone or dead: nothing.
        let near = Track::still(Point::new(900, 0), Millis(0));
        gatherer.go_for(SWORD);
        let world = self::world(item, near);
        assert_eq!(
            gatherer.frame(&world, Millis(0), None),
            vec![Command::PickUp { vid: SWORD }]
        );
        gatherer.go_for(SWORD);
        let mut gone = world.clone();
        gone.ground.clear();
        assert!(gatherer.frame(&gone, Millis(0), None).is_empty());
        assert_eq!(gatherer.item(), None);
        gatherer.go_for(SWORD);
        let mut dead = world;
        dead.entities.get_mut(&ME).unwrap().dead = true;
        assert!(gatherer.frame(&dead, Millis(0), None).is_empty());
        assert_eq!(gatherer.item(), None);
    }

    #[test]
    fn we_walk_around_walls_to_an_item_or_give_up() {
        // A wall 5 m east of us, 10 m long from the north edge.
        let mut walk = m2_content::WalkMap::open(20, 20).unwrap();
        for y in 0..10 {
            walk.set_blocked(5, y, true);
        }
        let walls = Walls::new(walk, Point::new(0, 0));
        let item = Point::new(1_000, 250);
        let mut gatherer = Gatherer::default();
        gatherer.go_for(SWORD);
        let world = world(item, Track::still(Point::new(250, 250), Millis(0)));
        let said = gatherer.frame(&world, Millis(0), Some(&walls));
        let [Command::MoveTo { path }] = &said[..] else {
            panic!("{said:?}");
        };
        assert!(
            path.iter().any(|p| p.y >= 1_000),
            "around the south end: {path:?}"
        );
        assert!((path.last().unwrap().distance(item) - STAND).abs() < 1.0);
        // Walled in all round: as far as the wall, then we give up.
        let mut walk = m2_content::WalkMap::open(20, 20).unwrap();
        for y in 0..20 {
            walk.set_blocked(5, y, true);
        }
        let walls = Walls::new(walk, Point::new(0, 0));
        gatherer.go_for(SWORD);
        let said = gatherer.frame(&world, Millis(0), Some(&walls));
        let [Command::MoveTo { path, .. }] = &said[..] else {
            panic!("{said:?}");
        };
        let end = path[0];
        assert!((475..500).contains(&end.x), "{path:?}");
        let there = self::world(item, Track::still(end, Millis(0)));
        assert!(
            gatherer
                .frame(&there, Millis(2_000), Some(&walls))
                .is_empty()
        );
        assert_eq!(gatherer.item(), None);
    }

    #[test]
    fn an_item_just_past_a_wall_is_asked_for_from_the_wall() {
        // A wall from 5 to 6 m east of us, all the way; the item lies
        // just past it, 4 m from us.
        let mut walk = m2_content::WalkMap::open(20, 20).unwrap();
        for y in 0..20 {
            walk.set_blocked(5, y, true);
        }
        let walls = Walls::new(walk, Point::new(0, 0));
        let item = Point::new(650, 250);
        let mut gatherer = Gatherer::default();
        gatherer.go_for(SWORD);
        let world = world(item, Track::still(Point::new(250, 250), Millis(0)));
        let said = gatherer.frame(&world, Millis(0), Some(&walls));
        let [Command::MoveTo { path }] = &said[..] else {
            panic!("{said:?}");
        };
        // At the wall, under 2 m from it and out of our own reach: we ask,
        // and the server decides.
        let end = *path.last().unwrap();
        assert!(end.distance(item) > PICK_UP_REACH, "{path:?}");
        let there = self::world(item, Track::still(end, Millis(0)));
        assert_eq!(
            gatherer.frame(&there, Millis(2_000), Some(&walls)),
            vec![Command::PickUp { vid: SWORD }]
        );
        assert_eq!(gatherer.item(), None);
        // Standing at the wall already, the same at once.
        gatherer.go_for(SWORD);
        assert_eq!(
            gatherer.frame(&there, Millis(2_000), Some(&walls)),
            vec![Command::PickUp { vid: SWORD }]
        );
    }

    #[test]
    fn someone_elses_items_are_greyed_and_left_to_them() {
        let here = Point::new(0, 0);
        let mut world = world(Point::new(500, 0), Track::still(here, Millis(0)));
        let bobs = GroundItem {
            vnum: 27_001,
            at: Point::new(100, 0),
            owner: Some("Bob".to_owned()),
        };
        world.ground.insert(600, bobs.clone());
        assert_eq!(name_colour(&bobs, &world), OTHERS_COLOUR);
        assert_eq!(name_colour(&world.ground[&SWORD], &world), NAME_COLOUR);
        let ours = GroundItem {
            owner: Some("Alice".to_owned()),
            ..bobs
        };
        assert_eq!(name_colour(&ours, &world), NAME_COLOUR);
        // Bob's is nearer, but ours to take is the sword; tried, nothing.
        assert_eq!(nearest_item(&world, here, &BTreeSet::new()), Some(SWORD));
        let tried = BTreeSet::from([SWORD]);
        assert_eq!(nearest_item(&world, here, &tried), None);
    }

    #[test]
    fn names_of_items_lying_together_stack() {
        let spots = [
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(500.0, 0.0, 0.0),
            Vec3::new(50.0, 50.0, 0.0),
            Vec3::new(0.0, 0.0, 0.0),
            // Within reach of the second item of the first group, not of
            // the first: a group of its own.
            Vec3::new(140.0, 0.0, 0.0),
        ];
        assert_eq!(stacked(&spots), [(0, 0), (1, 0), (0, 1), (0, 2), (4, 0)]);
        assert!(stacked(&[]).is_empty());
    }

    #[test]
    fn weapons_lie_flat_on_their_spot_and_the_rest_stands() {
        // A sword standing on its grip: 10 cm wide, 4 cm thick, 1 m long.
        let sword = Bounds {
            min: Vec3::new(-5.0, -2.0, -10.0),
            max: Vec3::new(5.0, 2.0, 90.0),
        };
        let (place, laid) = lying(sword, true);
        assert!(
            laid.min.abs_diff_eq(Vec3::new(-5.0, -50.0, 0.0), 1e-3),
            "{laid:?}"
        );
        assert!(
            laid.max.abs_diff_eq(Vec3::new(5.0, 50.0, 4.0), 1e-3),
            "{laid:?}"
        );
        // The tip of the blade lies on the ground's level, half a metre
        // out.
        let tip = place.transform_point3(Vec3::new(0.0, 0.0, 90.0));
        assert!(tip.abs_diff_eq(Vec3::new(0.0, -50.0, 2.0), 1e-3), "{tip:?}");
        let bag = Bounds {
            min: Vec3::new(-20.0, -20.0, 0.0),
            max: Vec3::new(20.0, 20.0, 30.0),
        };
        assert_eq!(lying(bag, false), (Mat4::IDENTITY, bag));
    }

    /// With an import of the human's client (`OPENM2_ASSETS_DIR`): the
    /// weapons lie flat, a metre or two long and a hand high; the bag, the
    /// gold and a potion were made standing on the ground.
    #[test]
    fn the_clients_ground_models_lie_on_the_ground() {
        let Some(source) = crate::actors::tests::imported() else {
            return;
        };
        for (path, weapon, length) in [
            ("ymir work/item/weapon/00010.glb", true, 136.0),
            ("ymir work/item/weapon/03000.glb", true, 222.0),
            ("ymir work/item/etc/item_bag.glb", false, 40.0),
            ("ymir work/item/etc/money.glb", false, 30.0),
            ("ymir work/item/etc/medicine_r.glb", false, 13.0),
        ] {
            let model = crate::actors::load_item(&source.files(), path).unwrap();
            let (_, laid) = lying(model.bounds.unwrap(), weapon);
            let size = laid.max - laid.min;
            assert!(laid.min.z.abs() < 1.0, "{path}: {laid:?}");
            assert!(
                (size.x.max(size.y) - length).abs() < 2.0,
                "{path}: {laid:?}"
            );
            if weapon {
                assert!(size.z < 15.0, "{path} lies flat: {laid:?}");
            }
        }
    }

    #[test]
    fn models_and_names_come_from_the_item_list_and_table() {
        let sword = ItemRecord {
            vnum: 10,
            locale_name: RawStr::from_field("Sword+0 é".as_bytes()),
            kind: WEAPON,
            ..ItemRecord::default()
        };
        let potion = ItemRecord {
            vnum: 27_001,
            locale_name: RawStr::from("Red Potion (S)"),
            kind: 3,
            ..ItemRecord::default()
        };
        let line = |vnum, model: Option<&str>| ItemLine {
            vnum,
            kind: RawStr::from("ETC"),
            icon: RawStr::from("icon/item/x.tga"),
            model: model.map(RawStr::from),
        };
        let mut items = Items::new(
            vec![sword, potion],
            &[line(10, Some("item/weapon/00010.glb")), line(27_001, None)],
        );
        let bag = "item/etc/item_bag.glb";
        assert_eq!(
            ground_model(Some(&items), 27_001),
            (String::new(), false),
            "no bag"
        );
        items.bag = Some(bag.as_bytes().to_vec());
        let items = Some(&items);
        assert_eq!(
            ground_model(items, 10),
            ("item/weapon/00010.glb".to_owned(), true)
        );
        assert_eq!(ground_model(items, 27_001), (bag.to_owned(), false));
        assert_eq!(ground_model(None, 10), (String::new(), false));
        assert_eq!(item_name(items, 10), "Sword+0 é");
        assert_eq!(item_name(items, 4), "Item 4");
        assert_ne!(turn(500), turn(501));
        assert!((0.0..std::f32::consts::TAU).contains(&turn(u32::MAX)));
    }
}

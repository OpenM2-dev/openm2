//! The names over characters' heads (`docs/specs/render.md`, "Names over
//! characters"): each living player, NPC, monster and metin stone near our
//! character, our own too, shows its name, players and monsters with their
//! level before it in a colour of its own. Players' and NPCs' names and
//! players' levels are the server's (EntityInfo); monsters' are the asset
//! folder's monster names and levels, as in the target window. A game
//! master whose mark shows has it over its name: the client's picture,
//! fading in and out as the client's effect does, else `GM`. `play` puts
//! them over the heads as labels; the Settings window turns them off.

use m2_assets::items::Monsters;
use m2_core::{Entity, EntityKind};
use m2_ui::{Colour, Label, LabelImage};

use crate::interface::{printable, text};

/// How far from our character names show, in centimetres.
pub(crate) const NAME_REACH: f32 = 5_000.0;

/// How far over a head (the top of its cylinder) a name is, in
/// centimetres.
pub(crate) const NAME_HEIGHT: f32 = 20.0;

/// How far above a name the damage numbers start, in layout pixels (times
/// the interface's scale).
pub(crate) const NAME_LINE: f32 = 15.0;

/// A player's name, ours too.
pub(crate) const PLAYER: Colour = [240, 240, 240, 255];
/// An NPC's name.
pub(crate) const NPC: Colour = [150, 215, 255, 255];
/// A monster's or a metin stone's name.
pub(crate) const MONSTER: Colour = [255, 160, 130, 255];
/// The level before a name.
pub(crate) const LEVEL: Colour = [235, 205, 110, 255];

/// The skin's look of a game master's mark (`skin.ron`).
pub(crate) const MARK_LOOK: &str = "game_master";
/// The mark's size in layout pixels: the client's 246 × 127 picture, 64
/// wide.
pub(crate) const MARK_SIZE: (f32, f32) = (64.0, 33.0);
/// The mark where the asset folder lacks the picture, and its colour.
pub(crate) const MARK_TEXT: &str = "GM";
pub(crate) const MARK: Colour = [255, 185, 40, 255];

/// How long the mark takes to fade in, show and fade out, in
/// milliseconds: the life of the one particle of the client's effect
/// script (`gm.mse`), which starts again once it is gone.
const MARK_LIFE: u64 = 2216;
/// Its opacity over that life, as the script keys it (a fraction of the
/// life, the opacity), straight between the keys; openm2's reading.
const MARK_FADE: [(f32, f32); 5] = [
    (0.0, 0.0),
    (0.1077, 0.7711),
    (0.2077, 1.0),
    (0.9026, 0.7926),
    (0.9974, 0.0),
];

/// The mark's opacity, 0 to 255, at `millis` of any clock: every mark
/// fades together.
pub(crate) fn mark_alpha(millis: u64) -> u8 {
    let at = (millis % MARK_LIFE) as f32 / MARK_LIFE as f32;
    let opacity = MARK_FADE
        .windows(2)
        .find(|keys| at <= keys[1].0)
        .map_or(0.0, |keys| {
            let ((from, a), (to, b)) = (keys[0], keys[1]);
            a + (b - a) * (at - from) / (to - from)
        });
    (opacity.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// A game master's mark, a line over its name at `at`: the picture, else
/// the text.
pub(crate) fn mark(at: (f32, f32), millis: u64) -> Label {
    Label {
        at,
        text: MARK_TEXT.to_owned(),
        colour: MARK,
        size: 0.0,
        lead: None,
        image: Some(LabelImage {
            look: MARK_LOOK.to_owned(),
            size: MARK_SIZE,
            alpha: mark_alpha(millis),
        }),
    }
}

/// What shows over a head.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NameTag {
    /// `Lv 12`, for players and monsters whose level is known.
    pub level: Option<String>,
    pub name: String,
    pub colour: Colour,
    /// A game master whose mark shows: the mark goes over the name.
    pub master: bool,
}

impl NameTag {
    /// The level as a label's lead: in its own colour, a space before the
    /// name.
    pub(crate) fn lead(&self) -> Option<(String, Colour)> {
        self.level
            .as_ref()
            .map(|level| (format!("{level} "), LEVEL))
    }
}

/// What shows over an entity, if anything: nothing over the dead, over
/// what has no name yet, or over portals, doors, buildings and horses.
pub(crate) fn name_tag(entity: &Entity, monsters: Option<&Monsters>) -> Option<NameTag> {
    if entity.dead {
        return None;
    }
    let from_table = || {
        let info = monsters?.get(u32::from(entity.race))?;
        let name = text(info.name.as_bytes());
        Some((name, u32::from(info.level)))
    };
    let (name, level, colour) = match entity.kind {
        EntityKind::Player | EntityKind::TransformedPlayer => {
            let info = entity.info.as_ref()?;
            (printable(&info.name), info.level, PLAYER)
        }
        EntityKind::Npc => match &entity.info {
            Some(info) => (printable(&info.name), 0, NPC),
            None => {
                let (name, _) = from_table()?;
                (name, 0, NPC)
            }
        },
        EntityKind::Monster | EntityKind::MetinStone => {
            let (name, level) = from_table()?;
            (name, level, MONSTER)
        }
        _ => return None,
    };
    let name = name.trim().to_owned();
    if name.is_empty() {
        return None;
    }
    let player = matches!(
        entity.kind,
        EntityKind::Player | EntityKind::TransformedPlayer
    );
    Some(NameTag {
        level: (level > 0).then(|| format!("Lv {level}")),
        name,
        colour,
        master: player && entity.is_game_master(),
    })
}

#[cfg(test)]
mod tests {
    use m2_assets::items::MonsterInfo;
    use m2_core::{Empire, EntityInfo, Millis, MoveKind, Point, Track};
    use m2_formats::RawStr;

    use super::*;

    fn entity(kind: EntityKind, race: u16, info: Option<(&str, u32)>) -> Entity {
        Entity {
            vid: 7,
            kind,
            race,
            track: Track::still(Point::new(0, 0), Millis(0)),
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
            info: info.map(|(name, level)| EntityInfo {
                name: name.to_owned(),
                empire: Empire::Other(0),
                level,
                guild: 0,
            }),
            mount: 0,
            skills: Vec::new(),
            line: None,
        }
    }

    fn monsters() -> Monsters {
        Monsters::from_entries([
            (
                101,
                MonsterInfo {
                    name: RawStr::from_bytes(b"Wild Dog"),
                    level: 1,
                },
            ),
            (
                9001,
                MonsterInfo {
                    name: RawStr::from_bytes(b"Arms Dealer"),
                    level: 0,
                },
            ),
        ])
    }

    #[test]
    fn players_show_their_level_and_name_from_the_server() {
        let tag = name_tag(&entity(EntityKind::Player, 0, Some(("Hero", 12))), None).unwrap();
        assert_eq!(tag.level.as_deref(), Some("Lv 12"));
        assert_eq!(tag.name, "Hero");
        assert_eq!(tag.colour, PLAYER);
        assert_eq!(tag.lead(), Some(("Lv 12 ".to_owned(), LEVEL)));
        assert_eq!(
            name_tag(&entity(EntityKind::Player, 0, None), None),
            None,
            "not before its EntityInfo"
        );
    }

    #[test]
    fn monsters_show_the_tables_level_and_name_npcs_their_name() {
        let monsters = monsters();
        let dog = name_tag(&entity(EntityKind::Monster, 101, None), Some(&monsters)).unwrap();
        assert_eq!(
            (dog.level.as_deref(), dog.name.as_str(), dog.colour),
            (Some("Lv 1"), "Wild Dog", MONSTER)
        );
        let npc = entity(EntityKind::Npc, 9001, Some(("Trader", 0)));
        let tag = name_tag(&npc, Some(&monsters)).unwrap();
        assert_eq!(
            (tag.level, tag.name.as_str(), tag.colour),
            (None, "Trader", NPC)
        );
        let npc = entity(EntityKind::Npc, 9001, None);
        assert_eq!(name_tag(&npc, Some(&monsters)).unwrap().name, "Arms Dealer");
        assert_eq!(
            name_tag(&entity(EntityKind::Monster, 102, None), Some(&monsters)),
            None,
            "a monster the table doesn't name"
        );
        assert_eq!(
            name_tag(&entity(EntityKind::Monster, 101, None), None),
            None
        );
    }

    #[test]
    fn the_dead_and_the_rest_show_nothing() {
        let monsters = monsters();
        let mut dog = entity(EntityKind::Monster, 101, None);
        dog.dead = true;
        assert_eq!(name_tag(&dog, Some(&monsters)), None);
        for kind in [EntityKind::Warp, EntityKind::Door, EntityKind::Horse] {
            assert_eq!(
                name_tag(&entity(kind, 101, Some(("x", 1))), Some(&monsters)),
                None
            );
        }
        let blank = entity(EntityKind::Player, 0, Some((" ", 3)));
        assert_eq!(name_tag(&blank, None), None);
    }

    #[test]
    fn a_game_master_whose_mark_shows_has_it_over_its_name() {
        let mut hero = entity(EntityKind::Player, 0, Some(("Hero", 30)));
        assert!(!name_tag(&hero, None).unwrap().master);
        hero.state = 2;
        assert!(name_tag(&hero, None).unwrap().master);
        // Only players: the server sends the flag for no other.
        let monsters = monsters();
        let mut dog = entity(EntityKind::Monster, 101, None);
        dog.state = 2;
        assert!(!name_tag(&dog, Some(&monsters)).unwrap().master);
        let mark = mark((400.0, 285.0), 0);
        assert_eq!((mark.text.as_str(), mark.colour), ("GM", MARK));
        let image = mark.image.unwrap();
        assert_eq!(
            (image.look.as_str(), image.size),
            ("game_master", (64.0, 33.0))
        );
    }

    #[test]
    fn the_mark_fades_in_shows_and_fades_out_again() {
        assert_eq!(mark_alpha(0), 0);
        // At its keys: 0.2077 of 2216 ms is about 460 ms, fully shown.
        assert_eq!(mark_alpha(460), 255);
        assert!(
            (195..=198).contains(&mark_alpha(239)),
            "{}",
            mark_alpha(239)
        );
        assert!(
            (200..=203).contains(&mark_alpha(2000)),
            "{}",
            mark_alpha(2000)
        );
        // Gone at the end of its life, then again from the start.
        assert_eq!(mark_alpha(2215), 0);
        assert_eq!(mark_alpha(2216), 0);
        assert_eq!(mark_alpha(2216 + 460), 255);
    }
}

//! NPCs and their shops in `openm2 play` (`docs/specs/ui.md`, "The shop";
//! `docs/specs/protocol/openm2/shops.md`): walking up to an NPC to talk to
//! it, the shop window, and the question before buying or selling. The
//! server has the last word: what we bought or sold changes the bag and our
//! gold when its answer comes.
//!
//! - A click on an NPC walks up to it and talks to it; its shop opens if it
//!   has one, with the inventory beside it.
//! - A click on a place of the shop asks before buying it. When it costs
//!   more than our gold or the bag has no room for it, the chat says so
//!   instead.
//! - An item of the bag put on the shop asks before selling it.
//! - The X, Escape or walking away closes the shop.

use std::collections::{BTreeMap, BTreeSet};

use m2_assets::items::Items;
use m2_core::{
    BAG_SLOTS, ChatKind, Command, EntityKind, GOLD_POINT, Item, Millis, Point, SHOP_PLACES,
    ShopGood, ShopRefusal, Window, World,
};
use m2_ui::{Colour, SlotItem, Ui};

use crate::ground::STAND;
use crate::interface::{chat_colour, ids, text};
use crate::inventory::{PAGE_CELLS, grouped};
use crate::walls::{self, Walls};

/// We talk to an NPC once our character is this close to it, in
/// centimetres (openm2's choice; the server allows 5 m).
pub(crate) const TALK_REACH: f64 = 250.0;

/// The shop closes once our character is farther than this from its NPC,
/// in centimetres (openm2's choice; the server forgets it past 12 m).
pub(crate) const SHOP_REACH: f64 = 1_000.0;

/// Columns of a page of the bag; a tall item takes the cells below its slot.
const BAG_COLUMNS: u16 = 5;

/// Colour of an item's name in its tooltip, and of the other lines.
const NAME_COLOUR: Colour = [255, 225, 150, 255];
const DETAIL_COLOUR: Colour = [220, 220, 220, 255];

/// Our way to an NPC we were asked to talk to: walk up to it, then talk,
/// or ask it about upgrading an item dropped onto it.
#[derive(Debug, Default)]
pub(crate) struct Talker {
    /// The NPC.
    npc: Option<u32>,
    /// Where our walk to it ends, and where it stood then.
    aimed: Option<(Point, Point)>,
    /// The bag slot of the item dropped onto the NPC: asked about instead
    /// of talking (`upgrade.rs`).
    upgrade: Option<u16>,
}

impl Talker {
    /// Goes to the NPC `vid` to talk to it.
    pub fn go_to(&mut self, vid: u32) {
        self.npc = Some(vid);
        self.aimed = None;
        self.upgrade = None;
    }

    /// Goes to the NPC `vid` to ask it about upgrading the item at bag
    /// slot `slot`, dropped onto it.
    pub fn go_to_upgrade(&mut self, vid: u32, slot: u16) {
        self.go_to(vid);
        self.upgrade = Some(slot);
    }

    /// Gives up: the player asked for something else.
    pub fn stop(&mut self) {
        self.npc = None;
        self.aimed = None;
        self.upgrade = None;
    }

    /// The server refused our walk up to the NPC and put us at `at`: as
    /// near as we get, so the next frame gives the talk up.
    pub fn refused(&mut self, at: Point) {
        if let Some((end, _)) = &mut self.aimed {
            *end = at;
        }
    }

    /// The NPC we go to.
    pub fn npc(&self) -> Option<u32> {
        self.npc
    }

    /// One frame: walks up to the NPC around `walls`, and once in reach
    /// stops and talks to it. Gives up when the NPC is gone, our
    /// character is dead, or the walls keep us from it.
    pub fn frame(&mut self, world: &World, now: Millis, walls: Option<&Walls>) -> Vec<Command> {
        let Some(vid) = self.npc else {
            return Vec::new();
        };
        let (Some(npc), Some(me)) = (world.entities.get(&vid), world.my_entity()) else {
            self.stop();
            return Vec::new();
        };
        if me.dead {
            self.stop();
            return Vec::new();
        }
        let here = me.track.point(now);
        let there = npc.track.point(now);
        let walking = me.track.is_moving(now);
        let distance = here.distance(there);
        if distance <= TALK_REACH {
            let upgrade = self.upgrade;
            self.stop();
            let mut out = Vec::new();
            if walking {
                out.push(Command::Stop);
            }
            out.push(match upgrade {
                Some(slot) => Command::GiveItem { vid, slot },
                None => Command::Talk { vid },
            });
            return out;
        }
        if let Some((end, aimed)) = self.aimed
            && aimed == there
        {
            if walking {
                return Vec::new();
            }
            if end.distance(here) < walls::NOWHERE {
                // As near as the walls let us, and not in reach.
                self.stop();
                return Vec::new();
            }
        }
        let path = walls::path_short_of(walls, here, there, STAND);
        let Some(&end) = path.last().filter(|_| walls::goes_somewhere(&path, here)) else {
            self.stop();
            return Vec::new();
        };
        self.aimed = Some((end, there));
        vec![Command::MoveTo { path }]
    }
}

/// True when the open shop's NPC is gone or farther than [`SHOP_REACH`]
/// from our character: the shop closes.
pub(crate) fn walked_away(world: &World, now: Millis) -> bool {
    world
        .shop
        .as_ref()
        .is_some_and(|shop| walked_away_from(world, now, shop.vid))
}

/// True when the NPC `vid` is gone or farther than [`SHOP_REACH`] from our
/// character: the shop or teacher we are looking at closes.
pub(crate) fn walked_away_from(world: &World, now: Millis, vid: u32) -> bool {
    let Some(me) = world.my_entity() else {
        return false;
    };
    world
        .entities
        .get(&vid)
        .is_none_or(|npc| me.track.point(now).distance(npc.track.point(now)) > SHOP_REACH)
}

/// The nearest NPC to `here` we did not talk to yet: for `play --shop`.
pub(crate) fn nearest_npc(
    world: &World,
    here: Point,
    now: Millis,
    tried: &BTreeSet<u32>,
) -> Option<u32> {
    world
        .entities
        .values()
        .filter(|entity| entity.kind == EntityKind::Npc && !tried.contains(&entity.vid))
        .map(|entity| (here.distance(entity.track.point(now)), entity.vid))
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, vid)| vid)
}

/// What the question asks about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Trade {
    /// Buying what lies in a place of the shop, as it was when asked.
    Buy { position: u8, good: ShopGood },
    /// Selling the item in a slot of the bag, as it was when asked.
    Sell { slot: u16, vnum: u32, count: u8 },
}

/// The shop window's state between frames.
pub(crate) struct ShopWindow {
    items: Items,
    /// The NPC whose shop is shown, to tell when one opens.
    open: Option<u32>,
    goods: BTreeMap<u8, ShopGood>,
    /// The question's trade.
    asking: Option<Trade>,
    /// The world's bag and gold as last seen.
    bag: BTreeMap<(Window, u16), Item>,
    gold: u32,
}

impl ShopWindow {
    pub fn new() -> Self {
        Self {
            items: Items::default(),
            open: None,
            goods: BTreeMap::new(),
            asking: None,
            bag: BTreeMap::new(),
            gold: 0,
        }
    }

    /// The asset folder's items (names, sizes, icons, prices).
    pub fn set_items(&mut self, items: Items) {
        self.items = items;
    }

    /// Forgets the game (on leaving it).
    pub fn reset(&mut self, ui: &mut Ui) {
        self.open = None;
        self.goods.clear();
        self.asking = None;
        ui.set_visible(ids::SHOP, false);
        ui.set_visible(ids::TRADE, false);
    }

    /// True while a shop is shown.
    pub fn is_open(&self) -> bool {
        self.open.is_some()
    }

    /// Escape on the question: closes it. False if there was none.
    pub fn cancel_question(&mut self, ui: &mut Ui) -> bool {
        let asked = self.asking.take().is_some();
        ui.set_visible(ids::TRADE, false);
        asked
    }

    /// A click on place `cell` of the shop: the question, or what stands in
    /// the way, in the chat.
    pub fn ask_buy(&mut self, ui: &mut Ui, cell: usize) {
        let Some((position, good)) = u8::try_from(cell)
            .ok()
            .and_then(|position| Some((position, *self.goods.get(&position)?)))
        else {
            return;
        };
        if good.price > self.gold {
            notice(ui, "You do not have enough gold.");
        } else if !self.bag_has_room(self.height(good.vnum)) {
            notice(ui, "There is no room in your bag.");
        } else {
            self.asking = Some(Trade::Buy { position, good });
        }
    }

    /// An item of the bag put on the shop: the question.
    pub fn ask_sell(&mut self, ui: &mut Ui, slot: u16) {
        let Some(item) = self.bag.get(&(Window::Inventory, slot)) else {
            return;
        };
        if slot >= BAG_SLOTS {
            notice(ui, "Take it off before selling it.");
            return;
        }
        self.asking = Some(Trade::Sell {
            slot,
            vnum: item.vnum,
            count: item.count,
        });
    }

    /// The question's answer (`yes`: buy or sell), if what it asked about
    /// is still there.
    pub fn answer(&mut self, ui: &mut Ui, yes: bool) -> Option<Command> {
        let asked = self.asking.take()?;
        ui.set_visible(ids::TRADE, false);
        if !yes || !self.still_there(asked) {
            return None;
        }
        Some(match asked {
            Trade::Buy { position, .. } => Command::Buy { position },
            Trade::Sell { slot, .. } => Command::Sell { slot },
        })
    }

    /// The server refused a purchase: the chat says why.
    pub fn refused(ui: &mut Ui, refusal: ShopRefusal) {
        notice(
            ui,
            match refusal {
                ShopRefusal::NotEnoughGold => "You do not have enough gold.",
                ShopRefusal::BagFull => "There is no room in your bag.",
                ShopRefusal::Other(_) => "The shop will not sell it.",
            },
        );
    }

    fn still_there(&self, asked: Trade) -> bool {
        match asked {
            Trade::Buy { position, good } => self.goods.get(&position) == Some(&good),
            Trade::Sell { slot, vnum, .. } => self
                .bag
                .get(&(Window::Inventory, slot))
                .is_some_and(|item| item.vnum == vnum),
        }
    }

    /// Cells an item takes in the bag, from the top.
    fn height(&self, vnum: u32) -> u16 {
        self.items
            .get(vnum)
            .map_or(1, |info| u16::from(info.record.size.clamp(1, 3)))
    }

    /// True if some slot of the bag has `height` free cells from it down,
    /// on one page (the server places what we buy).
    fn bag_has_room(&self, height: u16) -> bool {
        let mut taken = [false; BAG_SLOTS as usize];
        for ((window, slot), item) in &self.bag {
            if *window != Window::Inventory || *slot >= BAG_SLOTS {
                continue;
            }
            for below in 0..self.height(item.vnum) {
                if let Some(cell) = taken.get_mut(usize::from(slot + below * BAG_COLUMNS)) {
                    *cell = true;
                }
            }
        }
        (0..BAG_SLOTS).any(|slot| {
            let row = slot % PAGE_CELLS / BAG_COLUMNS;
            row + height <= PAGE_CELLS / BAG_COLUMNS
                && (0..height)
                    .all(|below| taken.get(usize::from(slot + below * BAG_COLUMNS)) == Some(&false))
        })
    }

    /// The icon, size and count of an item for a place.
    fn slot_item(&self, vnum: u32, count: u8) -> SlotItem {
        let icon = self
            .items
            .get(vnum)
            .and_then(|info| info.icon.as_deref())
            .map(|path| String::from_utf8_lossy(path).into_owned())
            .unwrap_or_default();
        SlotItem {
            icon,
            count: u32::from(count),
            height: self.height(vnum),
            dimmed: false,
            cooldown: None,
        }
    }

    /// An item's name: the item table's.
    fn name(&self, vnum: u32) -> String {
        self.items.get(vnum).map_or_else(
            || format!("Item {vnum}"),
            |info| text(info.record.locale_name.as_bytes()),
        )
    }

    /// `5 Red Potion (S)`, `Sword+0`.
    fn named(&self, vnum: u32, count: u8) -> String {
        let name = self.name(vnum);
        if count > 1 {
            format!("{count} {name}")
        } else {
            name
        }
    }

    /// The question's text and its button.
    fn question(&self, asked: Trade) -> (String, &'static str) {
        match asked {
            Trade::Buy { good, .. } => (
                format!(
                    "Buy {} for {} gold?",
                    self.named(good.vnum, good.count),
                    grouped(good.price)
                ),
                "Buy",
            ),
            Trade::Sell { vnum, count, .. } => {
                // What the shop pays is the server's; the item table says
                // what one piece sells for.
                let each = self
                    .items
                    .get(vnum)
                    .map_or(0, |info| info.record.sell_price);
                let text = if count > 1 {
                    format!(
                        "Sell {} at {} gold each?",
                        self.named(vnum, count),
                        grouped(each)
                    )
                } else {
                    format!("Sell {} for {} gold?", self.name(vnum), grouped(each))
                };
                (text, "Sell")
            }
        }
    }

    /// The lines shown over a place of the shop: name, count, price.
    fn tooltip(&self, cell: usize) -> Option<Vec<(String, Colour)>> {
        let good = self.goods.get(&u8::try_from(cell).ok()?)?;
        let mut lines = vec![(self.name(good.vnum), NAME_COLOUR)];
        if good.count > 1 {
            lines.push((format!("{} pieces", good.count), DETAIL_COLOUR));
        }
        lines.push((
            format!("Price: {} gold", grouped(good.price)),
            DETAIL_COLOUR,
        ));
        Some(lines)
    }

    /// Writes the open shop (titled `title`, its NPC's name), the question
    /// and the tooltip into the windows. True when a shop opened since the
    /// last call: the inventory opens beside it.
    pub fn update(&mut self, ui: &mut Ui, world: &World, title: Option<String>) -> bool {
        self.bag.clone_from(&world.items);
        self.gold = world.points.get(&GOLD_POINT).copied().unwrap_or(0);
        let shop = world.shop.as_ref();
        let opened = shop.is_some_and(|shop| Some(shop.vid) != self.open);
        self.open = shop.map(|shop| shop.vid);
        match shop {
            Some(shop) => self.goods.clone_from(&shop.goods),
            None => self.goods.clear(),
        }
        ui.set_visible(ids::SHOP, shop.is_some());
        if opened {
            ui.raise(ids::SHOP);
            ui.set_text(ids::SHOP_TITLE, title.as_deref().unwrap_or("Shop"));
        }
        ui.clear_slots(ids::SHOP_GOODS);
        for (place, good) in &self.goods {
            if *place < SHOP_PLACES {
                let shown = self.slot_item(good.vnum, good.count);
                ui.set_slot(ids::SHOP_GOODS, usize::from(*place), Some(shown));
            }
        }

        // The question goes with the shop, or with what it asks about.
        if let Some(asked) = self.asking
            && (shop.is_none() || !self.still_there(asked))
        {
            self.asking = None;
        }
        let question = self.asking.map(|asked| self.question(asked));
        if question.is_some() && !ui.is_visible(ids::TRADE) {
            ui.raise(ids::TRADE);
        }
        ui.set_visible(ids::TRADE, question.is_some());
        if let Some((text, button)) = question {
            ui.set_text(ids::TRADE_TEXT, &text);
            ui.set_text(ids::TRADE_YES, button);
        }
        if let Some(lines) = ui
            .hovered_slot()
            .filter(|(id, _)| *id == ids::SHOP_GOODS)
            .and_then(|(_, cell)| self.tooltip(cell))
        {
            ui.set_tooltip(lines);
        }
        opened
    }
}

/// A line of ours in the chat, in the colour of the server's information.
fn notice(ui: &mut Ui, text: &str) {
    ui.push_line(ids::CHAT_LOG, text, chat_colour(ChatKind::Info));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interface::{GAME, INVENTORY, SHOP, SKIN};
    use m2_core::{Entity, Me, MoveKind, Shop, Track};
    use m2_formats::script::item_list::ItemLine;
    use m2_formats::{ItemRecord, RawStr};
    use m2_ui::{Fonts, Layout, Skins};

    const ME: u32 = 1;
    const SHOPKEEPER: u32 = 2;

    fn ui() -> Ui {
        let mut ui = Ui::new(Skins::parse(SKIN).unwrap(), Fonts::empty());
        for layout in [GAME, INVENTORY, SHOP] {
            ui.add(&Layout::parse(layout).unwrap()).unwrap();
        }
        ui.layout(1280.0, 800.0);
        ui
    }

    fn items() -> Items {
        let record = |vnum, name: &str, size, sell_price| ItemRecord {
            vnum,
            locale_name: RawStr::from(name),
            size,
            sell_price,
            ..ItemRecord::default()
        };
        let line = |vnum, icon: &str| ItemLine {
            vnum,
            kind: RawStr::from("ETC"),
            icon: RawStr::from(icon),
            model: None,
        };
        Items::new(
            vec![
                record(10, "Sword+0", 2, 100),
                record(27001, "Red Potion (S)", 1, 12),
            ],
            &[
                line(10, "icon/item/00010.tga"),
                line(27001, "icon/item/27001.tga"),
            ],
        )
    }

    fn entity(vid: u32, kind: EntityKind, at: Point) -> Entity {
        Entity {
            vid,
            kind,
            race: 3,
            track: Track::still(at, Millis(0)),
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
        }
    }

    fn item(vnum: u32, count: u8) -> Item {
        Item {
            vnum,
            count,
            flags: 0,
            anti_flags: 0,
            sockets: [0; 3],
            bonuses: Vec::new(),
        }
    }

    /// Alice at the origin with 200 gold, potions in slot 3 and a sword
    /// worn; the shopkeeper 2 m east, its shop open: potions for 50 in
    /// place 0, a sword for 1,000 in place 5.
    fn world() -> World {
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
        world
            .entities
            .insert(ME, entity(ME, EntityKind::Player, Point::new(0, 0)));
        world.entities.insert(
            SHOPKEEPER,
            entity(SHOPKEEPER, EntityKind::Npc, Point::new(200, 0)),
        );
        world.points.insert(GOLD_POINT, 200);
        world.items.insert((Window::Inventory, 3), item(27001, 5));
        world.items.insert((Window::Inventory, 94), item(10, 1));
        let goods = [
            (
                0,
                ShopGood {
                    vnum: 27001,
                    price: 50,
                    count: 5,
                },
            ),
            (
                5,
                ShopGood {
                    vnum: 10,
                    price: 1_000,
                    count: 1,
                },
            ),
        ];
        world.shop = Some(Shop {
            vid: SHOPKEEPER,
            goods: goods.into_iter().collect(),
        });
        world
    }

    fn window() -> ShopWindow {
        let mut window = ShopWindow::new();
        window.set_items(items());
        window
    }

    #[test]
    fn we_walk_up_to_an_npc_then_talk_to_it() {
        let mut world = world();
        world.entities.get_mut(&SHOPKEEPER).unwrap().track =
            Track::still(Point::new(1_000, 0), Millis(0));
        let mut talker = Talker::default();
        talker.go_to(SHOPKEEPER);
        let walk = Command::MoveTo {
            path: vec![Point::new(950, 0)],
        };
        assert_eq!(talker.frame(&world, Millis(0), None), vec![walk]);
        // In reach: talk.
        world.entities.get_mut(&ME).unwrap().track = Track::still(Point::new(800, 0), Millis(0));
        assert_eq!(
            talker.frame(&world, Millis(10), None),
            vec![Command::Talk { vid: SHOPKEEPER }]
        );
        assert_eq!(talker.npc(), None);
        // Gone: nothing.
        talker.go_to(77);
        assert!(talker.frame(&world, Millis(20), None).is_empty());
        assert_eq!(talker.npc(), None);
        // An item dropped onto it: in reach, ask about upgrading it; once.
        talker.go_to_upgrade(SHOPKEEPER, 3);
        assert_eq!(
            talker.frame(&world, Millis(30), None),
            vec![Command::GiveItem {
                vid: SHOPKEEPER,
                slot: 3
            }]
        );
        talker.go_to(SHOPKEEPER);
        assert_eq!(
            talker.frame(&world, Millis(40), None),
            vec![Command::Talk { vid: SHOPKEEPER }]
        );
    }

    #[test]
    fn the_shop_closes_when_we_walk_away() {
        let mut world = world();
        assert!(!walked_away(&world, Millis(0)));
        world.entities.get_mut(&ME).unwrap().track = Track::still(Point::new(-900, 0), Millis(0));
        assert!(walked_away(&world, Millis(0)), "11 m from the shopkeeper");
        world.entities.remove(&SHOPKEEPER);
        assert!(walked_away(&world, Millis(0)), "the shopkeeper left");
        world.shop = None;
        assert!(!walked_away(&world, Millis(0)), "no shop");
        let tried = BTreeSet::new();
        let world = self::world();
        assert_eq!(
            nearest_npc(&world, Point::new(0, 0), Millis(0), &tried),
            Some(SHOPKEEPER)
        );
        assert_eq!(
            nearest_npc(&world, Point::new(0, 0), Millis(0), &[SHOPKEEPER].into()),
            None
        );
    }

    #[test]
    fn the_window_shows_the_goods_and_their_prices() {
        let (mut ui, mut window) = (ui(), window());
        assert!(window.update(&mut ui, &world(), Some("General Store".to_owned())));
        assert!(!window.update(&mut ui, &world(), None), "opened once");
        assert!(ui.is_visible(ids::SHOP));
        assert_eq!(ui.text(ids::SHOP_TITLE), Some("General Store"));
        assert_eq!(
            ui.slot(ids::SHOP_GOODS, 0),
            Some(&SlotItem {
                icon: "icon/item/27001.tga".to_owned(),
                count: 5,
                height: 1,
                dimmed: false,
                cooldown: None,
            })
        );
        assert_eq!(ui.slot(ids::SHOP_GOODS, 5).map(|s| s.height), Some(2));
        assert_eq!(ui.slot(ids::SHOP_GOODS, 1), None);
        let lines = |cell| -> Vec<String> {
            window
                .tooltip(cell)
                .unwrap_or_default()
                .into_iter()
                .map(|(text, _)| text)
                .collect()
        };
        assert_eq!(lines(0), ["Red Potion (S)", "5 pieces", "Price: 50 gold"]);
        assert_eq!(lines(5), ["Sword+0", "Price: 1,000 gold"]);
        // Closed by the session: the window goes.
        let mut closed = world();
        closed.shop = None;
        window.update(&mut ui, &closed, None);
        assert!(!ui.is_visible(ids::SHOP));
        assert!(!window.is_open());
    }

    #[test]
    fn buying_asks_first_and_says_what_stands_in_the_way() {
        let (mut ui, mut window) = (ui(), window());
        window.update(&mut ui, &world(), None);
        window.ask_buy(&mut ui, 0);
        window.update(&mut ui, &world(), None);
        assert!(ui.is_visible(ids::TRADE));
        assert_eq!(
            ui.text(ids::TRADE_TEXT),
            Some("Buy 5 Red Potion (S) for 50 gold?")
        );
        assert_eq!(ui.text(ids::TRADE_YES), Some("Buy"));
        assert_eq!(
            window.answer(&mut ui, true),
            Some(Command::Buy { position: 0 })
        );
        assert!(!ui.is_visible(ids::TRADE));
        // Cancel and Escape buy nothing.
        window.ask_buy(&mut ui, 0);
        assert_eq!(window.answer(&mut ui, false), None);
        window.ask_buy(&mut ui, 0);
        assert!(window.cancel_question(&mut ui));
        assert_eq!(window.answer(&mut ui, true), None);
        // Too dear, an empty place: no question.
        window.ask_buy(&mut ui, 5);
        window.ask_buy(&mut ui, 7);
        assert_eq!(window.answer(&mut ui, true), None);
        assert_eq!(ui.lines(ids::CHAT_LOG), ["You do not have enough gold."]);
        // A full bag: no room for the potions.
        let mut full = world();
        for slot in 0..BAG_SLOTS {
            full.items.insert((Window::Inventory, slot), item(27001, 1));
        }
        window.update(&mut ui, &full, None);
        window.ask_buy(&mut ui, 0);
        assert_eq!(window.answer(&mut ui, true), None);
        assert_eq!(
            ui.lines(ids::CHAT_LOG).last().copied(),
            Some("There is no room in your bag.")
        );
    }

    #[test]
    fn the_bag_has_room_for_tall_items_on_one_page() {
        let (mut ui, mut window) = (ui(), window());
        // Every cell of both pages full but the last two rows' first
        // column of the second page: room for a sword (2 high), not for
        // something 3 high.
        let mut world = world();
        world.items.clear();
        for slot in 0..BAG_SLOTS {
            if slot != 80 && slot != 85 {
                world
                    .items
                    .insert((Window::Inventory, slot), item(27001, 1));
            }
        }
        window.update(&mut ui, &world, None);
        assert!(window.bag_has_room(1));
        assert!(window.bag_has_room(2));
        assert!(!window.bag_has_room(3));
        // A sword in slot 80 takes slot 85 too.
        world.items.insert((Window::Inventory, 80), item(10, 1));
        window.update(&mut ui, &world, None);
        assert!(!window.bag_has_room(1));
    }

    #[test]
    fn selling_asks_first() {
        let (mut ui, mut window) = (ui(), window());
        window.update(&mut ui, &world(), None);
        window.ask_sell(&mut ui, 3);
        window.update(&mut ui, &world(), None);
        assert_eq!(
            ui.text(ids::TRADE_TEXT),
            Some("Sell 5 Red Potion (S) at 12 gold each?")
        );
        assert_eq!(ui.text(ids::TRADE_YES), Some("Sell"));
        assert_eq!(
            window.answer(&mut ui, true),
            Some(Command::Sell { slot: 3 })
        );
        // A worn item: take it off first.
        window.ask_sell(&mut ui, 94);
        assert_eq!(window.answer(&mut ui, true), None);
        assert_eq!(ui.lines(ids::CHAT_LOG), ["Take it off before selling it."]);
        // The item went meanwhile: the question goes too.
        window.ask_sell(&mut ui, 3);
        let mut sold = world();
        sold.items.remove(&(Window::Inventory, 3));
        window.update(&mut ui, &sold, None);
        assert!(!ui.is_visible(ids::TRADE));
        assert_eq!(window.answer(&mut ui, true), None);
    }
}

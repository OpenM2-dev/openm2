//! The inventory and the quickslot bar of `openm2 play` (`docs/specs/ui.md`,
//! "The inventory"): what the bag, the worn slots and the quickslots show,
//! and what the player's clicks and keys ask the session for. The server
//! has the last word: a move or a use is sent, and the windows change when
//! the server's answer changes the world (`inventory.md`).
//!
//! - Left click on an item picks it up (it follows the pointer); a left click
//!   on a slot puts it there: a move in the bag, wearing it, taking it off,
//!   or a quickslot. Dragging and letting go does the same. Let go over the
//!   world, an item of the bag is dropped on the ground after a question.
//! - An item of the bag put onto another (an upgrade scroll onto what it
//!   upgrades) asks the server what that does (`upgrade.rs`); let go over
//!   an NPC, the NPC is asked about upgrading it (`play.rs`).
//! - Right click uses an item (wears it, takes it off, uses it up).
//! - A quickslot picked up and put on another swaps them; let go over the
//!   world, it is emptied.
//! - A skill picked up in the skills window (`skills.rs`) and put on a
//!   quickslot goes there.
//! - Keys 1 to 4 and F1 to F4 use the quickslots of the page the bar shows:
//!   an item, or a skill; I opens the inventory.
//! - The bar has four pages of eight, as the original's taskbar: its up and
//!   down buttons go to the page before and after (round from the last to
//!   the first), Shift and 1 to 4 choose one, and while Alt is held the
//!   bar shows the next one (the original's, as it shows names).

use std::collections::BTreeMap;

use m2_assets::actor::ROD;
use m2_assets::items::Items;
use m2_core::{
    BAG_SLOTS, Command, GOLD_POINT, Item, QUICK_PAGE_SLOTS, QUICK_PAGES, QuickSlot, QuickSlotKind,
    WORN_FROM, Window, World,
};
use m2_ui::{Colour, Cooldown, PointerButton, SlotItem, Ui, UiEvent};

use crate::interface::{ids, text};

/// Cells of one page of the bag (5 × 9); the bag has two.
pub(crate) const PAGE_CELLS: u16 = 45;
/// Limit type of a level limit in the item table.
const LEVEL_LIMIT: u8 = 1;
/// Colour of an item's name in its tooltip.
const NAME_COLOUR: Colour = [255, 225, 150, 255];
/// Colour of the other tooltip lines.
const DETAIL_COLOUR: Colour = [220, 220, 220, 255];

/// The worn slots: node id, wear position (`inventory.md`, "Worn items")
/// and what the slot is for.
pub(crate) const WORN: [(&str, u16, &str); 11] = [
    (ids::WORN_BODY, 0, "Armour"),
    (ids::WORN_HEAD, 1, "Helmet"),
    (ids::WORN_SHOES, 2, "Shoes"),
    (ids::WORN_BRACELET, 3, "Bracelet"),
    (ids::WORN_WEAPON, 4, "Weapon"),
    (ids::WORN_NECKLACE, 5, "Necklace"),
    (ids::WORN_EARRINGS, 6, "Earrings"),
    (ids::WORN_UNIQUE1, 7, "Unique item"),
    (ids::WORN_UNIQUE2, 8, "Unique item"),
    (ids::WORN_ARROW, 9, "Arrows"),
    (ids::WORN_SHIELD, 10, "Shield"),
];

/// What the pointer carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Held {
    /// The item in this slot of the inventory window (bag or worn).
    Item(u16),
    /// What this quickslot holds.
    Quick(u8),
    /// This skill, from the skills window.
    Skill(u8),
}

/// How a skill shows on a quickslot: its icon at our grade, and its name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SkillShown {
    pub icon: String,
    pub name: String,
}

/// A place the player can point at: a slot of the inventory window or a
/// quickslot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Place {
    Slot(u16),
    Quick(u8),
}

/// An item of the bag let go of over the world: the question asks before
/// it is dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Dropping {
    slot: u16,
    /// What was there and how many, when asked.
    vnum: u32,
    count: u8,
}

/// The item kind of books that make a character forget its skills (the
/// client's numbering): `play` asks before using one.
const FORGET_KIND: u8 = 22;

/// The looks of the quickslot bar's page numbers, 1 to 4: the client's
/// taskbar digits, as its keys' labels.
const PAGE_NUMBERS: [&str; QUICK_PAGES as usize] = ["key_1", "key_2", "key_3", "key_4"];

/// The inventory's state between frames.
pub(crate) struct Inventory {
    items: Items,
    page: u16,
    held: Option<Held>,
    /// The slot grid and cell it was picked up from (letting go there
    /// keeps it on the pointer).
    held_at: Option<(String, usize)>,
    /// The item the drop question asks about.
    asking: Option<Dropping>,
    /// The book the question before forgetting asks about: its slot and
    /// number.
    forgetting: Option<(u16, u32)>,
    /// The world's items and quickslots as last seen.
    bag: BTreeMap<(Window, u16), Item>,
    quick: BTreeMap<u8, QuickSlot>,
    /// The quickslot bar's page, 0 to 3.
    quick_page: u8,
    /// Alt is held: the bar shows the next page.
    alt: bool,
    /// How our skills show, by number (from the skills window).
    skills: BTreeMap<u8, SkillShown>,
    /// The skills cooling down.
    cooldowns: BTreeMap<u8, Cooldown>,
    /// Skills a quickslot was used for, not taken yet.
    used_skills: Vec<u8>,
}

impl Inventory {
    pub fn new() -> Self {
        Self {
            items: Items::default(),
            page: 0,
            held: None,
            held_at: None,
            asking: None,
            forgetting: None,
            bag: BTreeMap::new(),
            quick: BTreeMap::new(),
            quick_page: 0,
            alt: false,
            skills: BTreeMap::new(),
            cooldowns: BTreeMap::new(),
            used_skills: Vec::new(),
        }
    }

    /// The skills cooling down, for their quickslots.
    pub fn set_cooldowns(&mut self, cooldowns: BTreeMap<u8, Cooldown>) {
        self.cooldowns = cooldowns;
    }

    /// How our skills show on the quickslots, by number.
    pub fn set_skills(&mut self, skills: BTreeMap<u8, SkillShown>) {
        self.skills = skills;
    }

    /// Puts skill `skill` on the pointer, picked up at cell `cell` of the
    /// slot grid `id` (the skills window): a quickslot takes it.
    pub fn hold_skill(&mut self, skill: u8, id: &str, cell: usize) {
        self.held = Some(Held::Skill(skill));
        self.held_at = Some((id.to_owned(), cell));
    }

    /// The skills quickslots were used for since the last call.
    pub fn take_skills(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.used_skills)
    }

    /// The asset folder's items (names, sizes, icons).
    pub fn set_items(&mut self, items: Items) {
        self.items = items;
    }

    /// Forgets the game: nothing carried, the first pages (on leaving it,
    /// as on a warp: the original's game window starts again on its first
    /// quickslot page).
    pub fn reset(&mut self, ui: &mut Ui) {
        self.drop_held();
        self.asking = None;
        ui.set_visible(ids::DROP, false);
        self.page = 0;
        self.quick_page = 0;
        self.alt = false;
        self.bag.clear();
        self.quick.clear();
        self.used_skills.clear();
        ui.set_held(None);
        ui.set_tooltip(Vec::new());
        ui.set_visible(ids::INVENTORY, false);
    }

    /// Opens or closes the inventory window.
    pub fn toggle(&mut self, ui: &mut Ui) {
        let open = !ui.is_visible(ids::INVENTORY);
        ui.set_visible(ids::INVENTORY, open);
        if open {
            ui.raise(ids::INVENTORY);
        } else if matches!(self.held, Some(Held::Item(_))) {
            self.drop_held();
        }
    }

    fn drop_held(&mut self) {
        self.held = None;
        self.held_at = None;
    }

    /// Escape: closes the drop question, else puts back what the pointer
    /// carries, else closes the inventory. False if there was none.
    pub fn cancel(&mut self, ui: &mut Ui) -> bool {
        if self.asking.take().is_some() {
            ui.set_visible(ids::DROP, false);
            true
        } else if self.forgetting.take().is_some() {
            ui.set_visible(ids::FORGET, false);
            true
        } else if self.held.is_some() {
            self.drop_held();
            true
        } else if ui.is_visible(ids::INVENTORY) {
            ui.set_visible(ids::INVENTORY, false);
            true
        } else {
            false
        }
    }

    /// True while the pointer carries something.
    pub fn holding(&self) -> bool {
        self.held.is_some()
    }

    /// The slot of the item of the bag the pointer carries, if it carries
    /// one (not a worn item, a quickslot or a skill).
    pub fn held_bag_item(&self) -> Option<u16> {
        match self.held? {
            Held::Item(slot) if slot < BAG_SLOTS => Some(slot),
            Held::Item(_) | Held::Quick(_) | Held::Skill(_) => None,
        }
    }

    /// The slot of the item of the bag that covers cell `cell` (its top
    /// slot, or one of the cells below of a tall item), if one does.
    fn covering(&self, cell: u16) -> Option<u16> {
        if cell >= BAG_SLOTS {
            return None;
        }
        let page = cell / PAGE_CELLS;
        self.bag.iter().find_map(|((window, slot), item)| {
            if *window != Window::Inventory || *slot >= BAG_SLOTS || *slot / PAGE_CELLS != page {
                return None;
            }
            let height = self
                .items
                .get(item.vnum)
                .map_or(1, |info| u16::from(info.record.size.clamp(1, 3)));
            let covers = (0..height).any(|row| slot + row * 5 == cell);
            covers.then_some(*slot)
        })
    }

    /// True while the pointer carries something or the drop question or
    /// the one before forgetting asks: Escape is for them first.
    pub fn busy(&self) -> bool {
        self.held.is_some() || self.asking.is_some() || self.forgetting.is_some()
    }

    /// Puts back what the pointer carries (it went onto a shop): the slot
    /// of the item, if one of the bag or the body was carried.
    pub fn put_back(&mut self) -> Option<u16> {
        let held = self.held?;
        self.drop_held();
        match held {
            Held::Item(slot) => Some(slot),
            Held::Quick(_) | Held::Skill(_) => None,
        }
    }

    /// A press on the world (not on a window) while something is carried:
    /// an item of the bag is asked about before it is dropped, a carried
    /// quickslot is emptied, a worn item is put back.
    pub fn world_press(&mut self) -> Option<Command> {
        let held = self.held?;
        self.drop_held();
        match held {
            Held::Quick(index) => Some(Command::ClearQuickSlot { index }),
            Held::Item(slot) if slot < BAG_SLOTS => {
                let item = self.item_at(slot)?;
                self.asking = Some(Dropping {
                    slot,
                    vnum: item.vnum,
                    count: item.count,
                });
                self.forgetting = None;
                None
            }
            Held::Item(_) | Held::Skill(_) => None,
        }
    }

    /// The drop question's answer (`drop`: yes): the whole stack goes, if
    /// it is still there.
    pub fn answer_drop(&mut self, drop: bool) -> Option<Command> {
        let asked = self.asking.take()?;
        let item = self
            .item_at(asked.slot)
            .filter(|item| item.vnum == asked.vnum)?;
        drop.then_some(Command::Drop {
            slot: asked.slot,
            count: asked.count.min(item.count),
        })
    }

    /// The answer to the question before forgetting (`forget`: yes): the
    /// book is used, if it is still there.
    pub fn answer_forget(&mut self, forget: bool) -> Option<Command> {
        let (slot, vnum) = self.forgetting.take()?;
        self.item_at(slot).filter(|item| item.vnum == vnum)?;
        forget.then_some(Command::UseItem {
            window: Window::Inventory,
            slot,
        })
    }

    /// A button of the inventory or the taskbar. True if it was one.
    pub fn clicked(&mut self, ui: &mut Ui, id: &str) -> bool {
        match id {
            ids::INVENTORY_PAGE1 => self.page = 0,
            ids::INVENTORY_PAGE2 => self.page = 1,
            ids::INVENTORY_CLOSE | ids::TASKBAR_INVENTORY => self.toggle(ui),
            ids::TASKBAR_PAGE_UP => {
                self.set_quick_page((self.quick_page + QUICK_PAGES - 1) % QUICK_PAGES);
            }
            ids::TASKBAR_PAGE_DOWN => self.set_quick_page((self.quick_page + 1) % QUICK_PAGES),
            _ => return false,
        }
        true
    }

    /// The quickslot bar's page `page` (0 to 3: Shift and 1 to 4, the up
    /// and down buttons). Another number changes nothing.
    pub fn set_quick_page(&mut self, page: u8) {
        if page < QUICK_PAGES && page != self.quick_page {
            self.quick_page = page;
            self.bar_turned();
        }
    }

    /// Alt held (`true`) or let go: while it is held the bar shows the
    /// page after the chosen one.
    pub fn set_alt(&mut self, alt: bool) {
        if alt != self.alt {
            self.alt = alt;
            self.bar_turned();
        }
    }

    /// The page the bar shows: the chosen one, or the next while Alt is
    /// held.
    pub fn shown_quick_page(&self) -> u8 {
        (self.quick_page + u8::from(self.alt)) % QUICK_PAGES
    }

    /// The bar shows another page: a quickslot carried from it is no longer
    /// under the cell it was picked up from, so letting go there puts it
    /// on that cell's quickslot of the page now shown.
    fn bar_turned(&mut self) {
        if matches!(self.held, Some(Held::Quick(_))) {
            self.held_at = None;
        }
    }

    /// Key `key` of the bar (0 to 7: 1 to 4, then F1 to F4) used: the
    /// quickslot under it on the page shown ([`Inventory::use_quick`]).
    pub fn use_key(&mut self, key: u8) -> Option<Command> {
        if key >= QUICK_PAGE_SLOTS {
            return None;
        }
        self.use_quick(self.shown_quick_page() * QUICK_PAGE_SLOTS + key)
    }

    /// What quickslot `index` (0 to 31) is used for: its item, if it holds
    /// one that is still there. A skill on it is noted for
    /// [`Inventory::take_skills`].
    pub fn use_quick(&mut self, index: u8) -> Option<Command> {
        let quick = self.quick.get(&index)?;
        match quick.kind {
            QuickSlotKind::Item => self.use_slot(u16::from(quick.position)),
            QuickSlotKind::Skill => {
                self.used_skills.push(quick.position);
                None
            }
            QuickSlotKind::Other(_) => None,
        }
    }

    /// Where a cell of a slot grid points.
    fn place(&self, id: &str, cell: usize) -> Option<Place> {
        let cell = u16::try_from(cell).ok()?;
        match id {
            ids::INVENTORY_BAG if cell < PAGE_CELLS => {
                Some(Place::Slot(self.page * PAGE_CELLS + cell))
            }
            ids::TASKBAR_QUICK => u8::try_from(cell)
                .ok()
                .filter(|cell| *cell < QUICK_PAGE_SLOTS)
                .map(|cell| Place::Quick(self.shown_quick_page() * QUICK_PAGE_SLOTS + cell)),
            worn => WORN
                .iter()
                .find(|(node, _, _)| *node == worn)
                .map(|(_, position, _)| Place::Slot(WORN_FROM + position)),
        }
    }

    fn item_at(&self, slot: u16) -> Option<&Item> {
        self.bag.get(&(Window::Inventory, slot))
    }

    /// Using the item at `slot`: UseItem, but a book that forgets asks
    /// first.
    fn use_slot(&mut self, slot: u16) -> Option<Command> {
        let item = self.item_at(slot)?;
        let forgets = self
            .items
            .get(item.vnum)
            .is_some_and(|info| info.record.kind == FORGET_KIND);
        if forgets && slot < BAG_SLOTS {
            self.forgetting = Some((slot, item.vnum));
            self.asking = None;
            return None;
        }
        Some(Command::UseItem {
            window: Window::Inventory,
            slot,
        })
    }

    /// The name of the item at bag `slot`, if there is one.
    pub fn name_at(&self, slot: u16) -> Option<String> {
        self.item_at(slot).map(|item| self.name(item.vnum))
    }

    /// A slot event: picking up, putting down, using. Returns what to ask
    /// the session for.
    pub fn on_event(&mut self, event: &UiEvent) -> Option<Command> {
        match event {
            UiEvent::SlotPressed { id, cell, button } => {
                let place = self.place(id, *cell)?;
                match button {
                    PointerButton::Main => self.press(place, id, *cell),
                    PointerButton::Secondary => {
                        if self.held.is_some() {
                            self.drop_held();
                            return None;
                        }
                        match place {
                            Place::Slot(slot) => self.use_slot(slot),
                            Place::Quick(index) => self.use_quick(index),
                        }
                    }
                }
            }
            UiEvent::SlotReleased { id, cell } => {
                // Let go somewhere else than where it was picked up: a drag.
                let from = self.held_at.as_ref()?;
                if (from.0.as_str(), from.1) == (id.as_str(), *cell) {
                    return None;
                }
                let place = self.place(id, *cell)?;
                self.put(place)
            }
            UiEvent::Clicked(_)
            | UiEvent::Submitted { .. }
            | UiEvent::Cancelled(_)
            | UiEvent::Changed(_) => None,
        }
    }

    fn press(&mut self, place: Place, id: &str, cell: usize) -> Option<Command> {
        if self.held.is_some() {
            return self.put(place);
        }
        let held = match place {
            Place::Slot(slot) => self.item_at(slot).map(|_| Held::Item(slot)),
            Place::Quick(index) => self
                .quick
                .contains_key(&index)
                .then_some(Held::Quick(index)),
        };
        if held.is_some() {
            self.held = held;
            self.held_at = Some((id.to_owned(), cell));
        }
        None
    }

    /// Puts what the pointer carries at `place`.
    fn put(&mut self, place: Place) -> Option<Command> {
        let held = self.held?;
        self.drop_held();
        match (held, place) {
            (Held::Item(from), Place::Slot(to)) if from < BAG_SLOTS && to < BAG_SLOTS => {
                let vnum = self.item_at(from)?.vnum;
                match self.covering(to) {
                    // Onto itself: nothing.
                    Some(onto) if onto == from => None,
                    // Onto another item: the server says what that does
                    // (an upgrade scroll offers its upgrade).
                    Some(onto) if self.item_at(onto).is_some_and(|item| item.vnum != vnum) => {
                        Some(Command::UseItemOnItem { slot: from, onto })
                    }
                    _ => {
                        let count = self.item_at(from)?.count;
                        Some(Command::MoveItem {
                            from: (Window::Inventory, from),
                            to: (Window::Inventory, to),
                            count,
                        })
                    }
                }
            }
            (Held::Item(from), Place::Slot(to)) if from != to => {
                let count = self.item_at(from)?.count;
                Some(Command::MoveItem {
                    from: (Window::Inventory, from),
                    to: (Window::Inventory, to),
                    count,
                })
            }
            (Held::Item(from), Place::Quick(index)) if from < BAG_SLOTS => {
                let position = u8::try_from(from).ok()?;
                let slot = QuickSlot {
                    kind: QuickSlotKind::Item,
                    position,
                };
                Some(Command::SetQuickSlot { index, slot })
            }
            (Held::Quick(first), Place::Quick(second)) if first != second => {
                Some(Command::SwapQuickSlots { first, second })
            }
            (Held::Skill(skill), Place::Quick(index)) => {
                let slot = QuickSlot {
                    kind: QuickSlotKind::Skill,
                    position: skill,
                };
                Some(Command::SetQuickSlot { index, slot })
            }
            // Put back where it was, a worn item on the quickslot bar, a
            // quickslot in the bag: nothing.
            _ => None,
        }
    }

    /// The icon, size and count of an item for a slot.
    fn slot_item(&self, item: &Item, dimmed: bool) -> SlotItem {
        let info = self.items.get(item.vnum);
        let icon = info
            .and_then(|info| info.icon.as_deref())
            .map(|path| String::from_utf8_lossy(path).into_owned())
            .unwrap_or_default();
        SlotItem {
            icon,
            count: u32::from(item.count),
            height: info.map_or(1, |info| u16::from(info.record.size.clamp(1, 3))),
            dimmed,
            cooldown: None,
        }
    }

    /// Writes the bag's open page, the worn items, the quickslots, gold, the
    /// carried item and the tooltip into the windows.
    pub fn update(&mut self, ui: &mut Ui, world: &World) {
        self.bag.clone_from(&world.items);
        self.quick.clone_from(&world.quickslots);
        // What was picked up may be gone (used up, moved by the server).
        let still_there = match self.held {
            Some(Held::Item(slot)) => self.item_at(slot).is_some(),
            Some(Held::Quick(index)) => self.quick.contains_key(&index),
            Some(Held::Skill(_)) | None => true,
        };
        if !still_there {
            self.drop_held();
        }
        let held_slot = match self.held {
            Some(Held::Item(slot)) => Some(slot),
            _ => None,
        };

        ui.clear_slots(ids::INVENTORY_BAG);
        let first = self.page * PAGE_CELLS;
        for cell in 0..PAGE_CELLS {
            let slot = first + cell;
            if let Some(item) = self.item_at(slot) {
                let shown = self.slot_item(item, held_slot == Some(slot));
                ui.set_slot(ids::INVENTORY_BAG, usize::from(cell), Some(shown));
            }
        }
        for (id, position, _) in WORN {
            let slot = WORN_FROM + position;
            let shown = self
                .item_at(slot)
                .map(|item| self.slot_item(item, held_slot == Some(slot)));
            ui.set_slot(id, 0, shown);
        }
        let first = self.shown_quick_page() * QUICK_PAGE_SLOTS;
        for cell in 0..QUICK_PAGE_SLOTS {
            let index = first + cell;
            let dimmed = self.held == Some(Held::Quick(index));
            let shown = self
                .quick_item(index)
                .map(|(item, _)| SlotItem {
                    height: 1,
                    ..self.slot_item(item, dimmed)
                })
                .or_else(|| {
                    let cooldown = self.quick_cooldown(index);
                    self.quick_skill(index).map(|shown| SlotItem {
                        cooldown,
                        ..skill_item(shown, dimmed)
                    })
                });
            ui.set_slot(ids::TASKBAR_QUICK, usize::from(cell), shown);
        }
        ui.set_look(
            ids::TASKBAR_PAGE,
            PAGE_NUMBERS[usize::from(self.shown_quick_page())],
        );
        let gold = world.points.get(&GOLD_POINT).copied().unwrap_or(0);
        ui.set_text(ids::INVENTORY_GOLD, &grouped(gold));
        ui.set_latched(ids::INVENTORY_PAGE1, self.page == 0);
        ui.set_latched(ids::INVENTORY_PAGE2, self.page == 1);

        let held = match self.held {
            Some(Held::Item(slot)) => self.item_at(slot).map(|item| self.slot_item(item, false)),
            Some(Held::Quick(index)) => self
                .quick_item(index)
                .map(|(item, _)| SlotItem {
                    height: 1,
                    ..self.slot_item(item, false)
                })
                .or_else(|| {
                    self.quick_skill(index)
                        .map(|shown| skill_item(shown, false))
                }),
            Some(Held::Skill(skill)) => self.skills.get(&skill).map(|s| skill_item(s, false)),
            None => None,
        };
        ui.set_held(held);

        // The question goes when its item does.
        if let Some(asked) = self.asking
            && self
                .item_at(asked.slot)
                .is_none_or(|item| item.vnum != asked.vnum)
        {
            self.asking = None;
        }
        let asking = self.asking.map(|asked| self.drop_question(asked));
        if asking.is_some() && !ui.is_visible(ids::DROP) {
            ui.raise(ids::DROP);
        }
        ui.set_visible(ids::DROP, asking.is_some());
        if let Some(question) = asking {
            ui.set_text(ids::DROP_TEXT, &question);
        }
        if let Some((slot, vnum)) = self.forgetting
            && self.item_at(slot).is_none_or(|item| item.vnum != vnum)
        {
            self.forgetting = None;
        }
        if self.forgetting.is_some() && !ui.is_visible(ids::FORGET) {
            ui.raise(ids::FORGET);
        }
        ui.set_visible(ids::FORGET, self.forgetting.is_some());
        let tooltip = ui
            .hovered_slot()
            .and_then(|(id, cell)| self.tooltip(id, cell))
            .unwrap_or_default();
        ui.set_tooltip(tooltip);
    }

    /// How the skill quickslot `index` stands for shows.
    fn quick_skill(&self, index: u8) -> Option<&SkillShown> {
        let quick = self.quick.get(&index)?;
        if quick.kind != QuickSlotKind::Skill {
            return None;
        }
        self.skills.get(&quick.position)
    }

    /// The cooldown of the skill quickslot `index` stands for, if it is
    /// cooling down.
    fn quick_cooldown(&self, index: u8) -> Option<Cooldown> {
        let quick = self.quick.get(&index)?;
        if quick.kind != QuickSlotKind::Skill {
            return None;
        }
        self.cooldowns.get(&quick.position).copied()
    }

    /// The item quickslot `index` stands for, and its slot in the bag.
    fn quick_item(&self, index: u8) -> Option<(&Item, u16)> {
        let quick = self.quick.get(&index)?;
        if quick.kind != QuickSlotKind::Item {
            return None;
        }
        let slot = u16::from(quick.position);
        self.item_at(slot).map(|item| (item, slot))
    }

    /// An item's name: the item table's.
    fn name(&self, vnum: u32) -> String {
        self.items.get(vnum).map_or_else(
            || format!("Item {vnum}"),
            |info| text(info.record.locale_name.as_bytes()),
        )
    }

    /// The drop question: `Drop Sword+0?`, `Drop 5 Red Potion (S)?`.
    fn drop_question(&self, asked: Dropping) -> String {
        let name = self.name(asked.vnum);
        if asked.count > 1 {
            format!("Drop {} {name}?", asked.count)
        } else {
            format!("Drop {name}?")
        }
    }

    /// The lines shown over a cell: the item's name and level, or what an
    /// empty worn slot is for.
    fn tooltip(&self, id: &str, cell: usize) -> Option<Vec<(String, Colour)>> {
        let item = match self.place(id, cell)? {
            Place::Slot(slot) => match self.item_at(slot) {
                Some(item) => item,
                None => {
                    let (_, _, what) = WORN.iter().find(|(node, _, _)| *node == id)?;
                    return Some(vec![((*what).to_owned(), DETAIL_COLOUR)]);
                }
            },
            Place::Quick(index) => match self.quick_item(index) {
                Some((item, _)) => item,
                None => {
                    let skill = self.quick_skill(index)?;
                    return Some(vec![(skill.name.clone(), NAME_COLOUR)]);
                }
            },
        };
        let mut lines = Vec::new();
        let info = self.items.get(item.vnum);
        lines.push((self.name(item.vnum), NAME_COLOUR));
        if item.count > 1 {
            lines.push((format!("{} pieces", item.count), DETAIL_COLOUR));
        }
        if let Some(level) = info.and_then(|info| {
            info.record
                .limits
                .iter()
                .find(|limit| limit.kind == LEVEL_LIMIT && limit.value > 0)
        }) {
            lines.push((format!("From level {}", level.value), DETAIL_COLOUR));
        }
        // A fishing pole: its points, the bait on it, and when it is full,
        // who betters it.
        if let Some(info) = info.filter(|info| info.record.kind == ROD) {
            let most = info
                .record
                .values
                .get(2)
                .map_or(0, |v| u32::try_from(*v).unwrap_or(0));
            let points = item.sockets[0];
            lines.push((format!("Points: {points} / {most}"), DETAIL_COLOUR));
            let bait = item.sockets[2];
            lines.push((
                if bait == 0 {
                    "No bait on the hook".to_owned()
                } else {
                    format!("Bait: {}", self.name(bait))
                },
                DETAIL_COLOUR,
            ));
            if most > 0 && points >= most {
                lines.push(("Full: the Fisherman can better it.".to_owned(), NAME_COLOUR));
            }
        }
        Some(lines)
    }
}

/// A skill's icon for a slot.
fn skill_item(shown: &SkillShown, dimmed: bool) -> SlotItem {
    SlotItem {
        icon: shown.icon.clone(),
        count: 0,
        height: 1,
        dimmed,
        cooldown: None,
    }
}

/// A number with commas between thousands: `2,000,000`.
pub(crate) fn grouped(value: u32) -> String {
    let digits = value.to_string();
    let mut out = String::new();
    for (at, digit) in digits.chars().enumerate() {
        if at > 0 && (digits.len() - at).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interface::{GAME, INVENTORY, SKIN};
    use m2_formats::script::item_list::ItemLine;
    use m2_formats::{ItemRecord, RawStr, proto::TypeValue};
    use m2_ui::{Fonts, Layout, Skins};

    fn ui() -> Ui {
        let mut ui = Ui::new(Skins::parse(SKIN).unwrap(), Fonts::empty());
        for layout in [GAME, INVENTORY] {
            ui.add(&Layout::parse(layout).unwrap()).unwrap();
        }
        ui.layout(1280.0, 800.0);
        ui
    }

    fn inventory() -> Inventory {
        let sword = ItemRecord {
            vnum: 10,
            locale_name: RawStr::from_field("Sword+0 é".as_bytes()),
            size: 2,
            limits: [TypeValue { kind: 1, value: 5 }, TypeValue::default()],
            ..ItemRecord::default()
        };
        let potion = ItemRecord {
            vnum: 27001,
            locale_name: RawStr::from("Red Potion (S)"),
            size: 1,
            ..ItemRecord::default()
        };
        let line = |vnum, icon: &str| ItemLine {
            vnum,
            kind: RawStr::from("ETC"),
            icon: RawStr::from(icon),
            model: None,
        };
        // Kind 22: a book that makes the character forget its skills.
        let book = ItemRecord {
            vnum: 70,
            locale_name: RawStr::from("Blank Book"),
            kind: FORGET_KIND,
            size: 1,
            ..ItemRecord::default()
        };
        let items = Items::new(
            vec![sword, potion, book],
            &[
                line(10, "icon/item/00010.tga"),
                line(27001, "icon/item/27001.tga"),
                line(70, "icon/item/00070.tga"),
            ],
        );
        let mut inventory = Inventory::new();
        inventory.set_items(items);
        inventory
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

    fn world() -> World {
        let mut world = World::default();
        world.items.insert((Window::Inventory, 0), item(10, 1));
        world.items.insert((Window::Inventory, 7), item(27001, 5));
        world.items.insert((Window::Inventory, 50), item(27001, 1));
        world.items.insert((Window::Inventory, 94), item(10, 1));
        world.quickslots.insert(
            2,
            QuickSlot {
                kind: QuickSlotKind::Item,
                position: 7,
            },
        );
        world.points.insert(GOLD_POINT, 1_234_567);
        world
    }

    fn press(id: &str, cell: usize, button: PointerButton) -> UiEvent {
        UiEvent::SlotPressed {
            id: id.to_owned(),
            cell,
            button,
        }
    }

    fn release(id: &str, cell: usize) -> UiEvent {
        UiEvent::SlotReleased {
            id: id.to_owned(),
            cell,
        }
    }

    #[test]
    fn the_windows_show_the_bag_page_the_worn_items_and_the_quickslots() {
        let (mut ui, mut inventory) = (ui(), inventory());
        inventory.update(&mut ui, &world());
        let bag = |ui: &Ui, cell| ui.slot(ids::INVENTORY_BAG, cell).cloned();
        assert_eq!(
            bag(&ui, 0),
            Some(SlotItem {
                icon: "icon/item/00010.tga".to_owned(),
                count: 1,
                height: 2,
                dimmed: false,
                cooldown: None,
            })
        );
        assert_eq!(bag(&ui, 7).map(|i| i.count), Some(5));
        assert!(ui.slot(ids::WORN_WEAPON, 0).is_some(), "slot 94: weapon");
        assert_eq!(
            ui.slot(ids::TASKBAR_QUICK, 2).map(|i| i.icon.as_str()),
            Some("icon/item/27001.tga")
        );
        assert_eq!(ui.text(ids::INVENTORY_GOLD), Some("1,234,567"));
        // The second page holds slot 50.
        assert!(inventory.clicked(&mut ui, ids::INVENTORY_PAGE2));
        inventory.update(&mut ui, &world());
        assert_eq!(bag(&ui, 0), None);
        assert_eq!(bag(&ui, 5).map(|i| i.count), Some(1));
    }

    #[test]
    fn items_are_picked_up_and_put_down_by_click_or_drag() {
        let (mut ui, mut inventory) = (ui(), inventory());
        inventory.update(&mut ui, &world());
        // Click, click: a move in the bag, with the whole stack.
        let main = PointerButton::Main;
        assert_eq!(
            inventory.on_event(&press(ids::INVENTORY_BAG, 7, main)),
            None
        );
        assert_eq!(inventory.on_event(&release(ids::INVENTORY_BAG, 7)), None);
        inventory.update(&mut ui, &world());
        assert!(ui.held().is_some());
        assert!(ui.slot(ids::INVENTORY_BAG, 7).unwrap().dimmed);
        assert_eq!(
            inventory.on_event(&press(ids::INVENTORY_BAG, 12, main)),
            Some(Command::MoveItem {
                from: (Window::Inventory, 7),
                to: (Window::Inventory, 12),
                count: 5
            })
        );
        inventory.update(&mut ui, &world());
        assert!(ui.held().is_none());
        // Drag the sword onto the body: wearing it is a move to slot 90.
        assert_eq!(
            inventory.on_event(&press(ids::INVENTORY_BAG, 0, main)),
            None
        );
        assert_eq!(
            inventory.on_event(&release(ids::WORN_BODY, 0)),
            Some(Command::MoveItem {
                from: (Window::Inventory, 0),
                to: (Window::Inventory, 90),
                count: 1
            })
        );
        // A bag item onto the quickslot bar.
        inventory.on_event(&press(ids::INVENTORY_BAG, 7, main));
        assert_eq!(
            inventory.on_event(&press(ids::TASKBAR_QUICK, 5, main)),
            Some(Command::SetQuickSlot {
                index: 5,
                slot: QuickSlot {
                    kind: QuickSlotKind::Item,
                    position: 7
                }
            })
        );
        // A worn item cannot go there; putting back does nothing.
        inventory.on_event(&press(ids::WORN_WEAPON, 0, main));
        assert_eq!(
            inventory.on_event(&press(ids::TASKBAR_QUICK, 0, main)),
            None
        );
        inventory.on_event(&press(ids::INVENTORY_BAG, 7, main));
        assert_eq!(
            inventory.on_event(&press(ids::INVENTORY_BAG, 7, main)),
            None
        );
        // Empty slots give nothing to pick up.
        assert_eq!(
            inventory.on_event(&press(ids::INVENTORY_BAG, 30, main)),
            None
        );
        assert!(!inventory.holding());
        assert_eq!(inventory.world_press(), None, "nothing carried");
    }

    #[test]
    fn quickslots_swap_empty_and_use() {
        let (mut ui, mut inventory) = (ui(), inventory());
        inventory.update(&mut ui, &world());
        let main = PointerButton::Main;
        assert_eq!(
            inventory.use_quick(2),
            Some(Command::UseItem {
                window: Window::Inventory,
                slot: 7
            })
        );
        assert_eq!(inventory.use_quick(3), None);
        inventory.on_event(&press(ids::TASKBAR_QUICK, 2, main));
        assert_eq!(
            inventory.on_event(&release(ids::TASKBAR_QUICK, 6)),
            Some(Command::SwapQuickSlots {
                first: 2,
                second: 6
            })
        );
        inventory.on_event(&press(ids::TASKBAR_QUICK, 2, main));
        assert!(inventory.holding());
        assert_eq!(
            inventory.world_press(),
            Some(Command::ClearQuickSlot { index: 2 })
        );
        // Right click uses: in the bag, on the body (taking it off), on the
        // bar.
        let secondary = PointerButton::Secondary;
        assert_eq!(
            inventory.on_event(&press(ids::INVENTORY_BAG, 7, secondary)),
            Some(Command::UseItem {
                window: Window::Inventory,
                slot: 7
            })
        );
        assert_eq!(
            inventory.on_event(&press(ids::WORN_WEAPON, 0, secondary)),
            Some(Command::UseItem {
                window: Window::Inventory,
                slot: 94
            })
        );
        assert_eq!(
            inventory.on_event(&press(ids::TASKBAR_QUICK, 2, secondary)),
            Some(Command::UseItem {
                window: Window::Inventory,
                slot: 7
            })
        );
        // A quickslot whose item moved away does nothing.
        let mut moved = world();
        moved.items.remove(&(Window::Inventory, 7));
        inventory.update(&mut ui, &moved);
        assert_eq!(inventory.use_quick(2), None);
        assert_eq!(ui.slot(ids::TASKBAR_QUICK, 2), None);
    }

    #[test]
    fn a_bag_item_let_go_over_the_world_is_dropped_after_asking() {
        let (mut ui, mut inventory) = (ui(), inventory());
        inventory.update(&mut ui, &world());
        let main = PointerButton::Main;
        // The potions: the question names the stack; Drop sends all five.
        inventory.on_event(&press(ids::INVENTORY_BAG, 7, main));
        assert_eq!(inventory.world_press(), None);
        assert!(!inventory.holding(), "put back while asking");
        inventory.update(&mut ui, &world());
        assert!(ui.is_visible(ids::DROP));
        assert_eq!(ui.text(ids::DROP_TEXT), Some("Drop 5 Red Potion (S)?"));
        assert_eq!(
            inventory.answer_drop(true),
            Some(Command::Drop { slot: 7, count: 5 })
        );
        inventory.update(&mut ui, &world());
        assert!(!ui.is_visible(ids::DROP));
        // Cancel, Escape: nothing goes.
        inventory.on_event(&press(ids::INVENTORY_BAG, 0, main));
        inventory.world_press();
        inventory.update(&mut ui, &world());
        assert_eq!(ui.text(ids::DROP_TEXT), Some("Drop Sword+0 é?"));
        assert_eq!(inventory.answer_drop(false), None);
        inventory.on_event(&press(ids::INVENTORY_BAG, 0, main));
        inventory.world_press();
        assert!(inventory.cancel(&mut ui));
        assert!(!ui.is_visible(ids::DROP));
        assert_eq!(inventory.answer_drop(true), None);
        // The item went meanwhile (used, moved): the question goes too.
        inventory.on_event(&press(ids::INVENTORY_BAG, 7, main));
        inventory.world_press();
        let mut moved = world();
        moved.items.remove(&(Window::Inventory, 7));
        inventory.update(&mut ui, &moved);
        assert!(!ui.is_visible(ids::DROP));
        assert_eq!(inventory.answer_drop(true), None);
        // A worn item is put back, not dropped.
        inventory.update(&mut ui, &world());
        inventory.on_event(&press(ids::WORN_WEAPON, 0, main));
        assert_eq!(inventory.world_press(), None);
        inventory.update(&mut ui, &world());
        assert!(!ui.is_visible(ids::DROP));
    }

    #[test]
    fn an_item_put_onto_another_asks_the_server() {
        let (mut ui, mut inventory) = (ui(), inventory());
        inventory.update(&mut ui, &world());
        let main = PointerButton::Main;
        // The potions onto the sword's lower half: onto the sword.
        inventory.on_event(&press(ids::INVENTORY_BAG, 7, main));
        assert_eq!(
            inventory.on_event(&press(ids::INVENTORY_BAG, 5, main)),
            Some(Command::UseItemOnItem { slot: 7, onto: 0 })
        );
        // Onto nothing they move.
        inventory.on_event(&press(ids::INVENTORY_BAG, 7, main));
        assert!(matches!(
            inventory.on_event(&press(ids::INVENTORY_BAG, 12, main)),
            Some(Command::MoveItem { .. })
        ));
        // The sword onto its own lower half: nothing.
        inventory.on_event(&press(ids::INVENTORY_BAG, 0, main));
        assert_eq!(
            inventory.on_event(&press(ids::INVENTORY_BAG, 5, main)),
            None
        );
        // What the pointer carries for an NPC: only an item of the bag.
        inventory.on_event(&press(ids::INVENTORY_BAG, 7, main));
        assert_eq!(inventory.held_bag_item(), Some(7));
        assert_eq!(inventory.put_back(), Some(7));
        inventory.on_event(&press(ids::WORN_WEAPON, 0, main));
        assert_eq!(inventory.held_bag_item(), None);
    }

    #[test]
    fn a_book_that_forgets_asks_before_it_is_used() {
        let (mut ui, mut inventory) = (ui(), inventory());
        let mut world = world();
        world.items.insert((Window::Inventory, 12), item(70, 1));
        world.quickslots.insert(
            3,
            QuickSlot {
                kind: QuickSlotKind::Item,
                position: 12,
            },
        );
        inventory.update(&mut ui, &world);
        let secondary = PointerButton::Secondary;
        let used = |slot| {
            Some(Command::UseItem {
                window: Window::Inventory,
                slot,
            })
        };
        // A right click asks; Forget uses it.
        assert_eq!(
            inventory.on_event(&press(ids::INVENTORY_BAG, 12, secondary)),
            None
        );
        inventory.update(&mut ui, &world);
        assert!(ui.is_visible(ids::FORGET) && inventory.busy());
        assert_eq!(inventory.answer_forget(true), used(12));
        inventory.update(&mut ui, &world);
        assert!(!ui.is_visible(ids::FORGET));
        // From its quickslot too; Cancel and Escape use nothing.
        assert_eq!(inventory.use_quick(3), None);
        assert_eq!(inventory.answer_forget(false), None);
        assert_eq!(inventory.use_quick(3), None);
        inventory.update(&mut ui, &world);
        assert!(inventory.cancel(&mut ui));
        assert!(!ui.is_visible(ids::FORGET));
        assert_eq!(inventory.answer_forget(true), None);
        // The book gone meanwhile: the question goes too.
        inventory.on_event(&press(ids::INVENTORY_BAG, 12, secondary));
        world.items.remove(&(Window::Inventory, 12));
        inventory.update(&mut ui, &world);
        assert!(!ui.is_visible(ids::FORGET));
        assert_eq!(inventory.answer_forget(true), None);
        // Other items are used at once.
        assert_eq!(
            inventory.on_event(&press(ids::INVENTORY_BAG, 7, secondary)),
            used(7)
        );
    }

    #[test]
    fn escape_puts_back_then_closes() {
        let (mut ui, mut inventory) = (ui(), inventory());
        inventory.update(&mut ui, &world());
        assert!(!inventory.cancel(&mut ui), "nothing to do");
        inventory.toggle(&mut ui);
        assert!(ui.is_visible(ids::INVENTORY));
        inventory.on_event(&press(ids::INVENTORY_BAG, 0, PointerButton::Main));
        assert!(inventory.cancel(&mut ui));
        assert!(ui.is_visible(ids::INVENTORY), "first the carried item");
        assert!(inventory.cancel(&mut ui));
        assert!(!ui.is_visible(ids::INVENTORY));
        assert!(inventory.clicked(&mut ui, ids::TASKBAR_INVENTORY));
        assert!(ui.is_visible(ids::INVENTORY));
    }

    /// The bar's four pages, as the original's: the buttons go round, Shift
    /// and 1 to 4 choose a page, Alt shows the next; the keys, the slots
    /// and what is put on them are the page's shown.
    #[test]
    fn the_quickslot_bar_has_four_pages() {
        let (mut ui, mut inventory) = (ui(), inventory());
        let mut world = world();
        world.quickslots.insert(
            10,
            QuickSlot {
                kind: QuickSlotKind::Item,
                position: 0,
            },
        );
        world.quickslots.insert(
            31,
            QuickSlot {
                kind: QuickSlotKind::Item,
                position: 50,
            },
        );
        inventory.update(&mut ui, &world);
        let shown = |ui: &Ui, cell| ui.slot(ids::TASKBAR_QUICK, cell).map(|s| s.count);
        // Page 1: quickslot 2 holds the 5 potions, its number shows 1.
        assert_eq!(shown(&ui, 2), Some(5));
        assert_eq!(ui.look(ids::TASKBAR_PAGE), Some("key_1"));
        // The page after: quickslot 10 (key 3, the sword) shows; key 3 uses it.
        assert!(inventory.clicked(&mut ui, ids::TASKBAR_PAGE_DOWN));
        inventory.update(&mut ui, &world);
        assert_eq!(ui.look(ids::TASKBAR_PAGE), Some("key_2"));
        assert_eq!(shown(&ui, 2), Some(1));
        assert_eq!(
            inventory.use_key(2),
            Some(Command::UseItem {
                window: Window::Inventory,
                slot: 0
            })
        );
        // A potion put on its F2 goes to quickslot 13.
        let main = PointerButton::Main;
        inventory.on_event(&press(ids::INVENTORY_BAG, 7, main));
        assert_eq!(
            inventory.on_event(&press(ids::TASKBAR_QUICK, 5, main)),
            Some(Command::SetQuickSlot {
                index: 13,
                slot: QuickSlot {
                    kind: QuickSlotKind::Item,
                    position: 7
                }
            })
        );
        // Before the first page is the last: quickslot 31 is its F4.
        assert!(inventory.clicked(&mut ui, ids::TASKBAR_PAGE_UP));
        assert!(inventory.clicked(&mut ui, ids::TASKBAR_PAGE_UP));
        inventory.update(&mut ui, &world);
        assert_eq!(ui.look(ids::TASKBAR_PAGE), Some("key_4"));
        assert_eq!(shown(&ui, 7), Some(1));
        assert_eq!(shown(&ui, 2), None);
        assert_eq!(
            inventory.use_key(7),
            Some(Command::UseItem {
                window: Window::Inventory,
                slot: 50
            })
        );
        // ... and after the last, the first.
        assert!(inventory.clicked(&mut ui, ids::TASKBAR_PAGE_DOWN));
        assert_eq!(inventory.shown_quick_page(), 0);
        // Shift and 3: the third page; Alt held shows the fourth.
        inventory.set_quick_page(2);
        inventory.set_alt(true);
        inventory.update(&mut ui, &world);
        assert_eq!(ui.look(ids::TASKBAR_PAGE), Some("key_4"));
        assert_eq!(shown(&ui, 7), Some(1));
        inventory.set_alt(false);
        inventory.update(&mut ui, &world);
        assert_eq!(ui.look(ids::TASKBAR_PAGE), Some("key_3"));
        assert_eq!(shown(&ui, 7), None);
        // No fifth page, no ninth key.
        inventory.set_quick_page(4);
        assert_eq!(inventory.shown_quick_page(), 2);
        assert_eq!(inventory.use_key(8), None);
        // Leaving the game: the first page again.
        inventory.reset(&mut ui);
        assert_eq!(inventory.shown_quick_page(), 0);
    }

    #[test]
    fn tooltips_name_the_item() {
        let inventory = {
            let (mut ui, mut inventory) = (ui(), inventory());
            inventory.update(&mut ui, &world());
            inventory
        };
        let names = |id, cell| -> Vec<String> {
            inventory
                .tooltip(id, cell)
                .unwrap_or_default()
                .into_iter()
                .map(|(text, _)| text)
                .collect()
        };
        assert_eq!(names(ids::INVENTORY_BAG, 0), ["Sword+0 é", "From level 5"]);
        assert_eq!(names(ids::INVENTORY_BAG, 7), ["Red Potion (S)", "5 pieces"]);
        assert_eq!(names(ids::WORN_HEAD, 0), ["Helmet"]);
        assert!(names(ids::INVENTORY_BAG, 30).is_empty());
        assert_eq!(grouped(0), "0");
        assert_eq!(grouped(999), "999");
        assert_eq!(grouped(2_000_000_000), "2,000,000,000");
    }

    #[test]
    fn a_poles_tooltip_says_its_points_and_bait() {
        let pole = ItemRecord {
            vnum: 27400,
            locale_name: RawStr::from("Fishing Pole+1"),
            kind: ROD,
            size: 3,
            values: [10, 5, 10, 100, 27400, 0],
            ..ItemRecord::default()
        };
        let worm = ItemRecord {
            vnum: 27801,
            locale_name: RawStr::from("Worm"),
            kind: 3,
            subtype: 6,
            size: 1,
            ..ItemRecord::default()
        };
        let mut inventory = Inventory::new();
        inventory.set_items(Items::new(vec![pole, worm], &[]));
        let mut world = World::default();
        let baited = Item {
            sockets: [3, 0, 27801],
            ..item(27400, 1)
        };
        let full = Item {
            sockets: [10, 0, 0],
            ..item(27400, 1)
        };
        world.items.insert((Window::Inventory, 0), baited);
        world.items.insert((Window::Inventory, 1), full);
        let mut ui = ui();
        inventory.update(&mut ui, &world);
        let lines = |cell| -> Vec<String> {
            inventory
                .tooltip(ids::INVENTORY_BAG, cell)
                .unwrap_or_default()
                .into_iter()
                .map(|(text, _)| text)
                .collect()
        };
        assert_eq!(lines(0), ["Fishing Pole+1", "Points: 3 / 10", "Bait: Worm"]);
        assert_eq!(
            lines(1),
            [
                "Fishing Pole+1",
                "Points: 10 / 10",
                "No bait on the hook",
                "Full: the Fisherman can better it."
            ]
        );
    }
}

//! NPCs' shops (`docs/specs/server/world.md`, "Shops"): talking to an NPC,
//! its shop opening, buying from it, selling to it and closing it.

use std::sync::Arc;

use m2_content::Item;
use m2_protocol::profile::openm2::{
    self as msg, ServerMessage, ShopBuy, ShopOpened, ShopRefused, ShopSell, Talk, TeacherOpened,
};

use super::grid::within;
use super::items::{BAG, Held, item_set};
use super::{Holder, Server, Stage};
use crate::time::{ConnId, Millis};

/// The shop a player has open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct OpenShop {
    /// The NPC's vid.
    vid: u32,
    /// The NPC's number: whose goods.
    vnum: u32,
}

/// ShopRefused's reasons.
const NOT_ENOUGH_GOLD: u8 = 1;
const BAG_FULL: u8 = 2;

/// What a shop sells, place by place.
fn shop_goods(goods: &[m2_content::ShopGood]) -> Vec<msg::ShopGood> {
    let mut goods: Vec<msg::ShopGood> = goods
        .iter()
        .map(|good| msg::ShopGood {
            position: good.position,
            vnum: good.item,
            price: good.price,
            count: good.count,
        })
        .collect();
    goods.sort_by_key(|good| good.position);
    goods
}

impl Server {
    /// Talk: the player talks to an NPC it sees within
    /// [`crate::Config::talk_reach`]. A teacher (`teachers.toml`) sends
    /// TeacherOpened and forgets any shop open, a stable (`mounts.toml`)
    /// StableOpened likewise, a smith (`upgrades.toml`) a line that says
    /// how it upgrades; an NPC with a shop opens it, in place of any shop
    /// open before; any other does nothing.
    /// Anything else is ignored.
    pub(super) fn talk(&mut self, now: Millis, conn: ConnId, talk: Talk) {
        if self.stage(conn) != Some(Stage::Game) {
            self.close(now, conn, "a game message outside the game");
            return;
        }
        let vid = talk.vid;
        let Some(player) = self.player(conn) else {
            return;
        };
        if !player.alive()
            || !player.seen.contains(&vid)
            || !matches!(self.vids.get(&vid), Some(Holder::Mob))
        {
            return;
        }
        let from = player.position(now);
        let Some(mob) = self.mobs.get(&vid) else {
            return;
        };
        if !mob.is_npc() || !within(from, mob.position(now), self.config.talk_reach) {
            return;
        }
        let vnum = mob.vnum();
        let content = Arc::clone(&self.content);
        if let Some(teacher) = content.teacher(vnum) {
            let opened = TeacherOpened {
                vid,
                job: teacher.job.job().unwrap_or_default(),
                group: teacher.group,
                level: self.config.skill_group_level,
            };
            if let Some(player) = self.player_mut(conn) {
                player.shop = None;
            }
            self.send(conn, ServerMessage::TeacherOpened(opened));
            return;
        }
        if self.is_stable(vnum) {
            self.open_stable(conn, vid);
            return;
        }
        if content.upgrades().smith(vnum).is_some() {
            if let Some(player) = self.player_mut(conn) {
                player.shop = None;
            }
            self.info(
                conn,
                "Drag an item from your bag onto the smith to upgrade it.",
            );
            return;
        }
        let goods = content.shop(vnum);
        if goods.is_empty() {
            if content.fishing().is_fisher(vnum) {
                self.info(
                    conn,
                    "Drag a fishing pole full of points from your bag onto the Fisherman to better it.",
                );
            }
            return;
        }
        let Some(player) = self.player_mut(conn) else {
            return;
        };
        player.shop = Some(OpenShop { vid, vnum });
        let opened = ShopOpened {
            vid,
            goods: shop_goods(goods),
        };
        self.send(conn, ServerMessage::ShopOpened(opened));
    }

    /// The NPC whose shop the player on `conn` has open, if it is still
    /// there and within [`crate::Config::shop_reach`]; otherwise the shop
    /// is forgotten (openm2: QCX keeps it however far the player walks).
    fn shop_near(&mut self, now: Millis, conn: ConnId) -> Option<u32> {
        let player = self.player(conn)?;
        let shop = player.shop?;
        let from = player.position(now);
        let near = self.mobs.get(&shop.vid).is_some_and(|mob| {
            mob.vnum() == shop.vnum && within(from, mob.position(now), self.config.shop_reach)
        });
        if near {
            return Some(shop.vnum);
        }
        if let Some(player) = self.player_mut(conn) {
            player.shop = None;
        }
        None
    }

    /// ShopBuy: the player buys the good at a place of its open shop, the
    /// whole stack for its price. Less gold than the price or no room in the
    /// bag gets ShopRefused, with nothing taken (openm2: QCX takes the gold
    /// anyway). Otherwise the gold goes, the item goes into the bag (onto
    /// stacks of it first), and PointsChanged and its ItemSets follow. An
    /// empty place or no shop open is ignored.
    pub(super) fn shop_buy(&mut self, now: Millis, conn: ConnId, buy: ShopBuy) {
        if !self.may_handle_items(now, conn) {
            return;
        }
        let Some(vnum) = self.shop_near(now, conn) else {
            return;
        };
        let content = Arc::clone(&self.content);
        let Some(good) = content
            .shop(vnum)
            .iter()
            .find(|good| good.position == buy.position)
            .copied()
        else {
            return;
        };
        let Some(player) = self.player_mut(conn) else {
            return;
        };
        if player.character.gold < good.price {
            let refused = ShopRefused {
                reason: NOT_ENOUGH_GOLD,
            };
            self.send(conn, ServerMessage::ShopRefused(refused));
            return;
        }
        let Some(changes) = player.bag.give(&content, good.item, good.count) else {
            let refused = ShopRefused { reason: BAG_FULL };
            self.send(conn, ServerMessage::ShopRefused(refused));
            return;
        };
        player.character.gold -= good.price;
        self.send_points(conn);
        for (slot, held) in changes {
            self.send(conn, item_set(slot, Some(held)));
        }
    }

    /// ShopSell: the player sells `count` of the item at a place of its
    /// bag to its open shop, for the item's sell value each (its sell price,
    /// at most its buy price; openm2: QCX sells the whole stack and pays
    /// the sell price once). The count must be 1
    /// to the stack's; worn items cannot be sold. The item's ItemSet goes
    /// first, then PointsChanged (QCX's order). Gold past
    /// [`crate::Config::max_gold`] is refused with a line in the chat;
    /// anything else is ignored.
    pub(super) fn shop_sell(&mut self, now: Millis, conn: ConnId, sell: ShopSell) {
        if !self.may_handle_items(now, conn) {
            return;
        }
        if self.shop_near(now, conn).is_none() {
            return;
        }
        let slot = sell.slot;
        let Some(held) = self.player(conn).and_then(|player| player.bag.get(slot)) else {
            return;
        };
        if slot >= BAG || sell.count == 0 || sell.count > held.count {
            return;
        }
        let each = self.content.item(held.vnum).map_or(0, Item::sell_value);
        let price = u64::from(each) * u64::from(sell.count);
        let max_gold = self.config.max_gold;
        let Some(player) = self.player_mut(conn) else {
            return;
        };
        let gold = u64::from(player.character.gold) + price;
        let Some(gold) = u32::try_from(gold).ok().filter(|gold| *gold <= max_gold) else {
            self.info(conn, "You cannot carry more gold.");
            return;
        };
        let left = (held.count > sell.count).then_some(Held {
            count: held.count - sell.count,
            ..held
        });
        player.bag.set(slot, left);
        player.character.gold = gold;
        self.send(conn, item_set(slot, left));
        self.send_points(conn);
    }

    /// ShopClose: the player's shop is forgotten. Outside the game it
    /// closes the connection, as every game message does.
    pub(super) fn shop_close(&mut self, now: Millis, conn: ConnId) {
        if self.stage(conn) != Some(Stage::Game) {
            self.close(now, conn, "a game message outside the game");
            return;
        }
        if let Some(player) = self.player_mut(conn) {
            player.shop = None;
        }
    }
}

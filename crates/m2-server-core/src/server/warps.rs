//! Warps (`docs/specs/server/world.md`, "Warps"): a warp
//! takes the players who come within 3 m of it to its place, a point of the
//! world, maybe on another map; a goto, to a point of the map it stands
//! on. As in the original, each looks around every half second.

use std::collections::{BTreeMap, BTreeSet};

use m2_content::{Monster, MonsterKind};
use m2_protocol::profile::openm2::{EntityRemove, ServerMessage};

use super::grid::within;
use super::world::VIEW_STEP;
use super::{Event, Server, Stage};
use crate::time::{ConnId, Millis};

/// How near a warp takes a player, in world units (the original's 3 m).
pub(super) const WARP_REACH: i32 = 300;
/// How often the warps look around, in milliseconds (the original's half
/// second).
pub(super) const WARP_EVERY: u64 = 500;

/// A warp in the world: where it stands and where it takes players.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Warp {
    pub at: (i32, i32),
    pub to: (i32, i32),
}

impl Server {
    /// Where `monster`, standing at `at`, takes players: its `to`, a goto's
    /// from the corner of the map it stands on. None for what takes nobody,
    /// a goto on no map, or a place on no map in a world with maps.
    pub(super) fn warp_of(&self, monster: &Monster, at: (i32, i32)) -> Option<(i32, i32)> {
        let to = monster.to_cm()?;
        let to = match monster.kind {
            MonsterKind::Warp => to,
            MonsterKind::Goto => {
                let (x, y) = self.content.map_at(at)?.origin_cm();
                (x.checked_add(to.0)?, y.checked_add(to.1)?)
            }
            _ => return None,
        };
        let no_maps = self.content.maps().next().is_none();
        (no_maps || self.content.map_at(to).is_some()).then_some(to)
    }

    /// Every [`WARP_EVERY`], each warp takes the living players in the
    /// game within [`WARP_REACH`] of it to its place, each player by one
    /// warp; not those with a shop open (as the original, which takes
    /// nobody busy with a trade).
    pub(super) fn warp_tick(&mut self, now: Millis) {
        if now < self.next_warps {
            return;
        }
        self.next_warps = now.plus(WARP_EVERY);
        let mut taken: BTreeMap<ConnId, (i32, i32)> = BTreeMap::new();
        for warp in self.warps.values() {
            // The grid keeps players where they last looked around, up to a
            // step of their walk behind.
            for conn in self.grid.players_around(warp.at, WARP_REACH + VIEW_STEP) {
                if taken.contains_key(&conn) || self.stage(conn) != Some(Stage::Game) {
                    continue;
                }
                let Some(player) = self.player(conn) else {
                    continue;
                };
                let near = within(player.position(now), warp.at, WARP_REACH);
                if near && player.alive() && player.shop.is_none() {
                    taken.insert(conn, warp.to);
                }
            }
        }
        for (conn, to) in taken {
            self.take_to(now, conn, to);
        }
    }

    /// The player on `conn` is taken to `to` at once: it stops where it
    /// was, the monsters fighting it give it up, and it shows again there
    /// ([`Server::show_again`]).
    pub(super) fn take_to(&mut self, now: Millis, conn: ConnId, to: (i32, i32)) {
        self.stop_fishing(conn);
        self.release_hunters(conn, now);
        let watched_before = self.watchers_of(conn);
        let Some(player) = self.player_mut(conn) else {
            return;
        };
        player.walk = None;
        player.character.x = to.0;
        player.character.y = to.1;
        player.moved_at = now;
        let character = player.character.id;
        self.show_again(now, conn, &watched_before);
        self.provoke_near(conn, now);
        self.event(Event::Warped {
            conn,
            character,
            to,
        });
    }

    /// The player on `conn` shows again where it now is (after a restart or
    /// a warp): the grid keeps it there; it gets its own EntityRemove,
    /// EntitySpawn and EntityInfo, and its mount while it rides; the
    /// players that saw it before and still do get the same, and its
    /// skills; the others see it come or go as [`Server::update_view`]
    /// links them; and it sees the monsters and items around it there.
    pub(super) fn show_again(
        &mut self,
        now: Millis,
        conn: ConnId,
        watched_before: &BTreeSet<ConnId>,
    ) {
        let Some(player) = self.player_mut(conn) else {
            return;
        };
        let (vid, after) = (player.vid, player.at());
        let before = std::mem::replace(&mut player.seen_from, after);
        self.grid.move_player(conn, before, after);
        let remove = ServerMessage::EntityRemove(EntityRemove { vid });
        self.send(conn, remove.clone());
        if let (Some(spawn), Some(info)) = (self.spawn_of(conn), self.info_of(conn)) {
            self.send(conn, spawn.clone());
            self.send(conn, info.clone());
            self.show_rider(conn, conn);
            self.update_view(conn);
            let still = self.watchers_of(conn);
            for watcher in watched_before.intersection(&still) {
                self.send(*watcher, remove.clone());
                self.send(*watcher, spawn.clone());
                self.send(*watcher, info.clone());
                self.show_rider(conn, *watcher);
                self.show_skills(conn, *watcher);
            }
        }
        self.update_mob_view(conn, now);
        self.update_ground_view(conn);
    }
}

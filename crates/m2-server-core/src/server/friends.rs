//! Friends: requests and their answers, ending a friendship, and who of a
//! player's friends is in the game (`docs/specs/server/session.md`,
//! "Friends"). A friendship is between two characters, both ways; it is
//! loaded with the characters and written on its own, and a login waits
//! for its account's friendship writes as it waits for its saves.

use m2_protocol::profile::openm2::{
    AnswerFriend, AskFriend, Friend, FriendAsked, FriendChanged, FriendNotice, Friends,
    RemoveFriend, ServerMessage,
};

use super::{Effect, Event, Server, Stage, Waiter, valid_character_name};
use crate::store::{AccountId, RequestId, StoreRequest, StoredFriend};
use crate::time::{ConnId, Millis};

/// The most friends one Friends message carries, and so the most a
/// character has, whatever [`crate::Config::max_friends`] says.
pub const MAX_FRIENDS: usize = 64;

/// FriendNotice's notices (`friends.md`).
mod notice {
    /// Our request went out.
    pub const SENT: u8 = 1;
    /// Nobody of that name is in the game.
    pub const NOT_ONLINE: u8 = 2;
    /// We are friends already.
    pub const ALREADY: u8 = 3;
    /// Our list is full.
    pub const OURS_FULL: u8 = 4;
    /// Their list is full.
    pub const THEIRS_FULL: u8 = 5;
    /// Too many requests: ours in a short time, or waiting for them.
    pub const TOO_MANY: u8 = 6;
    /// They declined.
    pub const DECLINED: u8 = 7;
    /// Our request ended unanswered: it expired or they left.
    pub const UNANSWERED: u8 = 8;
    /// Their request to us ended: it expired or they left.
    pub const ENDED: u8 = 9;
}

/// FriendChanged's statuses.
mod status {
    /// No longer a friend.
    pub const REMOVED: u8 = 0;
    /// A friend out of the game.
    pub const OFFLINE: u8 = 1;
    /// A friend in the game.
    pub const ONLINE: u8 = 2;
}

/// A friend request waiting for its answer, under (the asked character,
/// the asker) in [`Server::friend_asks`]. Both are in the game while it
/// waits: it ends when either leaves.
#[derive(Debug, Clone, Copy)]
pub(super) struct FriendAsk {
    /// The asked player's connection.
    asked: ConnId,
    /// The asker's connection.
    asker: ConnId,
    /// When it ends unanswered.
    expires: Millis,
}

/// A character in the game: its number, name and account.
struct Me {
    id: u32,
    name: String,
    account: AccountId,
}

impl Server {
    /// AskFriend: checked in turn (too many requests, nobody of that name
    /// in the game, friends already, a list full); a request from them to
    /// us waiting counts as accepting it, and a request of ours waiting
    /// gets `SENT` again without asking them twice. Otherwise they get
    /// FriendAsked, we get `SENT`, and the request waits
    /// [`crate::Config::friend_ask_time`]. A name that is no character's,
    /// or our own, is ignored.
    pub(super) fn ask_friend(&mut self, now: Millis, conn: ConnId, ask: AskFriend) {
        if self.stage(conn) != Some(Stage::Game) {
            self.close(now, conn, "a friend request outside the game");
            return;
        }
        let Some(me) = self.me(conn) else {
            return;
        };
        let name = ask.name;
        if !valid_character_name(name.as_bytes()) || name.eq_ignore_ascii_case(&me.name) {
            self.event(Event::Ignored {
                conn,
                message: "AskFriend",
            });
            return;
        }
        let (burst, window) = (self.config.friend_ask_burst, self.config.friend_ask_window);
        let Some(state) = self.conns.get_mut(&conn) else {
            return;
        };
        state.asked_at.retain(|at| now.since(*at) < window);
        if state.asked_at.len() >= burst {
            self.friend_notice(conn, name, notice::TOO_MANY);
            return;
        }
        state.asked_at.push(now);
        let Some((other, them)) = self.in_game(&name).and_then(|c| Some((c, self.me(c)?))) else {
            self.friend_notice(conn, name, notice::NOT_ONLINE);
            return;
        };
        if self.lists(conn, them.id) {
            self.friend_notice(conn, them.name, notice::ALREADY);
            return;
        }
        if self.friends_full(conn) {
            self.friend_notice(conn, them.name, notice::OURS_FULL);
            return;
        }
        if self.friends_full(other) {
            self.friend_notice(conn, them.name, notice::THEIRS_FULL);
            return;
        }
        if self.friend_asks.contains_key(&(me.id, them.id)) {
            // They asked us first: asking them back accepts.
            self.friend_asks.remove(&(me.id, them.id));
            self.befriend(conn, other);
            return;
        }
        if self.friend_asks.contains_key(&(them.id, me.id)) {
            self.friend_notice(conn, them.name, notice::SENT);
            return;
        }
        let waiting = self
            .friend_asks
            .keys()
            .filter(|(asked, _)| *asked == them.id)
            .count();
        if waiting >= self.config.max_friend_asks {
            self.friend_notice(conn, them.name, notice::TOO_MANY);
            return;
        }
        let expires = now.plus(self.config.friend_ask_time);
        self.friend_asks.insert(
            (them.id, me.id),
            FriendAsk {
                asked: other,
                asker: conn,
                expires,
            },
        );
        self.send(
            other,
            ServerMessage::FriendAsked(FriendAsked { name: me.name }),
        );
        self.friend_notice(conn, them.name, notice::SENT);
    }

    /// AnswerFriend: a request from that player to us must be waiting,
    /// else we get `ENDED` (our question is out of date). Declining tells
    /// the asker; accepting makes the friendship when both lists have
    /// room.
    pub(super) fn answer_friend(&mut self, now: Millis, conn: ConnId, answer: AnswerFriend) {
        if self.stage(conn) != Some(Stage::Game) {
            self.close(now, conn, "a friend answer outside the game");
            return;
        }
        let Some(me) = self.me(conn) else {
            return;
        };
        let asker = self
            .in_game(&answer.name)
            .and_then(|c| Some((c, self.me(c)?)))
            .filter(|(_, asker)| self.friend_asks.contains_key(&(me.id, asker.id)));
        let Some((asker_conn, asker)) = asker else {
            self.friend_notice(conn, answer.name, notice::ENDED);
            return;
        };
        self.friend_asks.remove(&(me.id, asker.id));
        if answer.accept {
            self.befriend(conn, asker_conn);
        } else {
            self.friend_notice(asker_conn, me.name, notice::DECLINED);
        }
    }

    /// RemoveFriend: the friend must be in our list, by name in any case,
    /// else it is ignored. The friendship ends on both sides at once,
    /// whether or not the friend is in the game: we get FriendChanged
    /// `REMOVED`, and so does the friend if it is in the game with that
    /// character.
    pub(super) fn remove_friend(&mut self, now: Millis, conn: ConnId, remove: RemoveFriend) {
        if self.stage(conn) != Some(Stage::Game) {
            self.close(now, conn, "removing a friend outside the game");
            return;
        }
        let Some(me) = self.me(conn) else {
            return;
        };
        let friend = self.player(conn).and_then(|player| {
            player
                .character
                .friends
                .iter()
                .find(|f| f.name.eq_ignore_ascii_case(&remove.name))
                .cloned()
        });
        let Some(friend) = friend else {
            self.event(Event::Ignored {
                conn,
                message: "RemoveFriend",
            });
            return;
        };
        self.change_friends(me.account, me.id, |friends| {
            friends.retain(|f| f.id != friend.id);
        });
        self.send(
            conn,
            ServerMessage::FriendChanged(FriendChanged {
                name: friend.name.clone(),
                status: status::REMOVED,
            }),
        );
        self.write_friendship(
            StoreRequest::RemoveFriendship {
                characters: [me.id, friend.id],
            },
            [me.account, friend.account],
        );
        let told = self.change_friends(friend.account, friend.id, |friends| {
            friends.retain(|f| f.id != me.id);
        });
        if let Some(other) = told {
            self.send(
                other,
                ServerMessage::FriendChanged(FriendChanged {
                    name: me.name,
                    status: status::REMOVED,
                }),
            );
        }
        self.event(Event::FriendsParted {
            a: me.id,
            b: friend.id,
        });
    }

    /// The player on `conn` entered the game: it gets its friends, each
    /// marked as in the game or not, and its friends in the game whose
    /// lists hold it get FriendChanged `ONLINE`.
    pub(super) fn friends_entered(&mut self, conn: ConnId) {
        let Some((me, friends)) = self
            .me(conn)
            .zip(self.player(conn).map(|p| p.character.friends.clone()))
        else {
            return;
        };
        let list: Vec<Friend> = friends
            .iter()
            .take(MAX_FRIENDS)
            .map(|friend| Friend {
                name: friend.name.clone(),
                online: self.friend_in_game(friend).is_some(),
            })
            .collect();
        if !list.is_empty() {
            self.send(conn, ServerMessage::Friends(Friends { friends: list }));
        }
        self.tell_friends(conn, &me, &friends, status::ONLINE);
    }

    /// The player on `conn` leaves the game: its friends in the game whose
    /// lists hold it get FriendChanged `OFFLINE`, and the requests it made
    /// or was asked end (the other side gets `ENDED` or `UNANSWERED`).
    pub(super) fn friends_left(&mut self, conn: ConnId) {
        let Some((me, friends)) = self
            .me(conn)
            .zip(self.player(conn).map(|p| p.character.friends.clone()))
        else {
            return;
        };
        self.tell_friends(conn, &me, &friends, status::OFFLINE);
        let ended: Vec<(u32, u32)> = self
            .friend_asks
            .keys()
            .filter(|(asked, asker)| *asked == me.id || *asker == me.id)
            .copied()
            .collect();
        for key in ended {
            self.end_friend_ask(key, Some(conn));
        }
    }

    /// Requests whose time is up end: the asker gets `UNANSWERED`, the
    /// asked `ENDED`.
    pub(super) fn expire_friend_asks(&mut self, now: Millis) {
        let ended: Vec<(u32, u32)> = self
            .friend_asks
            .iter()
            .filter(|(_, ask)| now >= ask.expires)
            .map(|(key, _)| *key)
            .collect();
        for key in ended {
            self.end_friend_ask(key, None);
        }
    }

    /// Ends a request unanswered: each side in the game hears of it, by the
    /// other's name, but the one `leaving`.
    fn end_friend_ask(&mut self, key: (u32, u32), leaving: Option<ConnId>) {
        let Some(ask) = self.friend_asks.remove(&key) else {
            return;
        };
        let asked = self.me(ask.asked).filter(|me| me.id == key.0);
        let asker = self.me(ask.asker).filter(|me| me.id == key.1);
        let (Some(asked), Some(asker)) = (asked, asker) else {
            return;
        };
        if leaving != Some(ask.asker) {
            self.friend_notice(ask.asker, asked.name, notice::UNANSWERED);
        }
        if leaving != Some(ask.asked) {
            self.friend_notice(ask.asked, asker.name, notice::ENDED);
        }
    }

    /// Two players in the game become friends, when both lists have room:
    /// each list gains the other, the friendship is written, and both get
    /// FriendChanged `ONLINE`. A full list stops it, and each side hears
    /// whose.
    fn befriend(&mut self, a: ConnId, b: ConnId) {
        let (Some(first), Some(second)) = (self.me(a), self.me(b)) else {
            return;
        };
        if self.lists(a, second.id) {
            return;
        }
        if self.friends_full(a) {
            self.friend_notice(a, second.name, notice::OURS_FULL);
            self.friend_notice(b, first.name, notice::THEIRS_FULL);
            return;
        }
        if self.friends_full(b) {
            self.friend_notice(a, second.name, notice::THEIRS_FULL);
            self.friend_notice(b, first.name, notice::OURS_FULL);
            return;
        }
        self.friend_asks.remove(&(first.id, second.id));
        self.friend_asks.remove(&(second.id, first.id));
        let of = |me: &Me| StoredFriend {
            id: me.id,
            name: me.name.clone(),
            account: me.account,
        };
        let (to_first, to_second) = (of(&second), of(&first));
        self.change_friends(first.account, first.id, |friends| {
            add_friend(friends, to_first.clone());
        });
        self.change_friends(second.account, second.id, |friends| {
            add_friend(friends, to_second.clone());
        });
        self.write_friendship(
            StoreRequest::AddFriendship {
                characters: [first.id, second.id],
            },
            [first.account, second.account],
        );
        self.send(
            a,
            ServerMessage::FriendChanged(FriendChanged {
                name: second.name,
                status: status::ONLINE,
            }),
        );
        self.send(
            b,
            ServerMessage::FriendChanged(FriendChanged {
                name: first.name,
                status: status::ONLINE,
            }),
        );
        self.event(Event::FriendsMade {
            a: first.id,
            b: second.id,
        });
    }

    /// Changes the friends of character `character` of `account` wherever
    /// the server holds it: in the game, picked, or in the character list.
    /// A login of that account loading its characters now loads them again
    /// once the friendship written with this change has landed (its load
    /// may hold the old friends). Returns the connection if that character
    /// is in the game, to be told.
    fn change_friends(
        &mut self,
        account: AccountId,
        character: u32,
        change: impl Fn(&mut Vec<StoredFriend>),
    ) -> Option<ConnId> {
        let conn = *self.online.get(&account)?;
        let state = self.conns.get_mut(&conn)?;
        for listed in state.characters.iter_mut().filter(|c| c.id == character) {
            change(&mut listed.friends);
        }
        let mut in_game = false;
        if let Some(player) = state.player.as_mut()
            && player.character.id == character
        {
            change(&mut player.character.friends);
            in_game = state.stage == Stage::Game;
        }
        if state.stage == Stage::Joining && state.waiting.take().is_some() {
            self.deferred.entry(account).or_default().push(conn);
        }
        in_game.then_some(conn)
    }

    /// Writes a friendship change. Each of the two accounts counts it among
    /// its saves under way, so a login of either waits for it (`saved`).
    fn write_friendship(&mut self, what: StoreRequest, accounts: [AccountId; 2]) {
        let [first, second] = accounts;
        *self.saving.entry(first).or_default() += 1;
        if second != first {
            *self.saving.entry(second).or_default() += 1;
        }
        self.next_request += 1;
        let request = RequestId(self.next_request);
        self.requests.insert(request, Waiter::Friendship(accounts));
        self.out.push(Effect::Store { request, what });
    }

    /// Tells each friend in the game whose list holds `me` that it came or
    /// went.
    fn tell_friends(&mut self, conn: ConnId, me: &Me, friends: &[StoredFriend], status: u8) {
        for friend in friends {
            let Some(other) = self.friend_in_game(friend) else {
                continue;
            };
            if other != conn && self.lists(other, me.id) {
                self.send(
                    other,
                    ServerMessage::FriendChanged(FriendChanged {
                        name: me.name.clone(),
                        status,
                    }),
                );
            }
        }
    }

    /// The connection of `friend` if it is in the game with that
    /// character.
    fn friend_in_game(&self, friend: &StoredFriend) -> Option<ConnId> {
        self.in_game(&friend.name).filter(|conn| {
            self.player(*conn)
                .is_some_and(|p| p.character.id == friend.id)
        })
    }

    /// Whether the player on `conn` has character `id` among its friends.
    fn lists(&self, conn: ConnId, id: u32) -> bool {
        self.player(conn)
            .is_some_and(|p| p.character.friends.iter().any(|f| f.id == id))
    }

    /// Whether the player on `conn` has as many friends as it may.
    fn friends_full(&self, conn: ConnId) -> bool {
        let most = self.config.max_friends.min(MAX_FRIENDS);
        self.player(conn)
            .is_none_or(|p| p.character.friends.len() >= most)
    }

    /// The character the player on `conn` plays.
    fn me(&self, conn: ConnId) -> Option<Me> {
        let state = self.conns.get(&conn)?;
        let player = state.player.as_ref()?;
        Some(Me {
            id: player.character.id,
            name: player.character.name.clone(),
            account: state.account.as_ref()?.id,
        })
    }

    fn friend_notice(&mut self, conn: ConnId, name: String, notice: u8) {
        self.send(
            conn,
            ServerMessage::FriendNotice(FriendNotice { name, notice }),
        );
    }
}

/// Adds a friend to a list kept in order of names in lowercase, once.
fn add_friend(friends: &mut Vec<StoredFriend>, friend: StoredFriend) {
    if friends.iter().any(|f| f.id == friend.id) {
        return;
    }
    friends.push(friend);
    friends.sort_by_key(|f| f.name.to_ascii_lowercase());
}

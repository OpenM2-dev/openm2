//! A harness for the server core: it steps the server, answers its store
//! requests from memory at once, says Hello and signs in as the client does
//! (the edge's checks of sign-ins answered by the test) and keeps a log of
//! everything that happened, one line each.

#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;

use m2_content::Content;
use m2_protocol::profile::openm2::{
    CharacterMove, ClearQuickSlot, ClientMessage, CreateCharacter, DropGold, DropItem, EnterGame,
    Hello, Hit, KeyLogin, LearnSkillGroup, MoveItem, PROTOCOL, PickUp, Pong, ProviderLogin,
    RaiseSkill, RaiseStatus, Restart, SelectCharacter, SendChat, ServerMessage, SetQuickSlot,
    SetTarget, ShopBuy, ShopClose, ShopSell, SignInOptions, StartSignIn, SwapQuickSlots, Talk,
    TicketLogin, UseItem, UseItemOnSkill, UseSkill,
};
use m2_server_core::{
    Account, AccountId, Character, Config, ConnId, CreateRefusal, Effect, Identity, Input,
    KeySignIn, Listener, Millis, ProviderOffer, RequestId, Server, SignIn, SignInRefusal,
    StoreAnswer, StoreError, StoreRequest, StoredFriend,
};

pub const SEED: [u8; 32] = [7; 32];
/// The Unix time, in seconds, at the server's time 0: 2027-01-15.
pub const EPOCH: u64 = 1_800_000_000;
pub const HOME: IpAddr = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1));

/// The provider the harness's server offers, and its issuer.
pub const PROVIDER: &str = "example";
pub const ISSUER: &str = "https://id.example";

/// The provider the harness's server offers.
pub fn offer() -> ProviderOffer {
    ProviderOffer {
        id: PROVIDER.to_owned(),
        name: "Example ID".to_owned(),
        authorize: format!("{ISSUER}/authorize"),
        client_id: "openm2".to_owned(),
    }
}

/// The provider's player `subject`.
pub fn identity(subject: &str) -> Identity {
    Identity {
        issuer: ISSUER.to_owned(),
        subject: subject.to_owned(),
    }
}

/// The identity of a device's key, as the edge hands it in.
pub fn key_identity(key: [u8; 32]) -> Identity {
    Identity {
        issuer: m2_protocol::key_sign_in::ISSUER.to_owned(),
        subject: m2_protocol::key_sign_in::PublicKey::from_bytes(key).to_string(),
    }
}

/// The account name the harness's store gives the provider's player
/// `subject` (storage names it with a hash instead).
pub fn account_of(subject: &str) -> String {
    format!("{PROVIDER}:{subject}")
}

/// Accounts and characters, as a database would keep them.
#[derive(Debug, Default)]
pub struct Store {
    /// Each account's name and empire.
    pub accounts: BTreeMap<u64, (String, Option<u8>)>,
    pub characters: BTreeMap<u32, (u64, Character)>,
    /// Providers' players' accounts, by issuer and subject.
    pub identities: BTreeMap<(String, String), u64>,
    /// Friendships, each once: the lower character number first.
    pub friendships: BTreeSet<(u32, u32)>,
    next_character: u32,
}

impl Store {
    fn answer(&mut self, what: StoreRequest) -> StoreAnswer {
        match what {
            StoreRequest::IdentityLogin {
                provider,
                issuer,
                subject,
            } => {
                let id = match self.identities.get(&(issuer.clone(), subject.clone())) {
                    Some(id) => *id,
                    None => {
                        let id = self.accounts.len() as u64 + 1;
                        let name = format!("{provider}:{subject}");
                        self.accounts.insert(id, (name, None));
                        self.identities.insert((issuer, subject), id);
                        id
                    }
                };
                let (name, empire) = self.accounts.get(&id).cloned().unwrap_or_default();
                StoreAnswer::LoginChecked(Ok(Account {
                    id: AccountId(id),
                    name,
                    empire,
                }))
            }
            StoreRequest::LoadCharacters { account } => StoreAnswer::Characters(Ok(self
                .characters
                .values()
                .filter(|(owner, _)| *owner == account.0)
                .map(|(_, character)| Character {
                    friends: self.friends_of(character.id),
                    ..character.clone()
                })
                .collect())),
            StoreRequest::CreateCharacter {
                account,
                mut character,
            } => {
                let taken = self
                    .characters
                    .values()
                    .any(|(_, c)| c.name.eq_ignore_ascii_case(&character.name));
                let owned: Vec<&Character> = self
                    .characters
                    .values()
                    .filter(|(owner, _)| *owner == account.0)
                    .map(|(_, c)| c)
                    .collect();
                if taken {
                    return StoreAnswer::CharacterCreated(Err(CreateRefusal::NameTaken));
                }
                if owned.len() >= 4 || owned.iter().any(|c| c.slot == character.slot) {
                    return StoreAnswer::CharacterCreated(Err(CreateRefusal::NoRoom));
                }
                self.next_character += 1;
                character.id = self.next_character;
                character.friends.clear();
                if let Some((_, empire)) = self.accounts.get_mut(&account.0) {
                    empire.get_or_insert(character.empire);
                }
                self.characters
                    .insert(character.id, (account.0, character.clone()));
                StoreAnswer::CharacterCreated(Ok(character))
            }
            StoreRequest::SaveCharacter {
                account,
                mut character,
            } => {
                // Only a newer save is kept, as storage does; friends are
                // never saved with it.
                let newer = self
                    .characters
                    .get(&character.id)
                    .is_none_or(|(_, stored)| stored.saves < character.saves);
                if newer {
                    character.friends.clear();
                    self.characters.insert(character.id, (account.0, character));
                }
                StoreAnswer::Saved(Ok(()))
            }
            StoreRequest::AddFriendship { characters } => {
                StoreAnswer::FriendshipWritten(self.pair(characters).map(|pair| {
                    self.friendships.insert(pair);
                }))
            }
            StoreRequest::RemoveFriendship { characters: [a, b] } => {
                StoreAnswer::FriendshipWritten(if a == b {
                    Err(StoreError("no such friendship".to_owned()))
                } else {
                    self.friendships.remove(&(a.min(b), a.max(b)));
                    Ok(())
                })
            }
        }
    }

    /// Two characters that exist, the lower number first.
    fn pair(&self, [a, b]: [u32; 2]) -> Result<(u32, u32), StoreError> {
        if a == b || !self.characters.contains_key(&a) || !self.characters.contains_key(&b) {
            return Err(StoreError("no such friendship".to_owned()));
        }
        Ok((a.min(b), a.max(b)))
    }

    /// The friends of character `id`, by name in lowercase.
    fn friends_of(&self, id: u32) -> Vec<StoredFriend> {
        let mut friends: Vec<StoredFriend> = self
            .friendships
            .iter()
            .filter_map(|&(a, b)| {
                if a == id {
                    Some(b)
                } else if b == id {
                    Some(a)
                } else {
                    None
                }
            })
            .filter_map(|friend| self.characters.get(&friend))
            .map(|(owner, character)| StoredFriend {
                id: character.id,
                name: character.name.clone(),
                account: AccountId(*owner),
            })
            .collect();
        friends.sort_by_key(|f| f.name.to_ascii_lowercase());
        friends
    }
}

/// What the harness remembers of a connection, as its client would.
#[derive(Debug, Default, Clone)]
pub struct Client {
    pub welcomed: bool,
    pub ticket: Option<Vec<u8>>,
    /// The last SignInOptions.
    pub options: Option<SignInOptions>,
    /// The account name of a SignedIn (its ticket goes in `ticket`).
    pub account: Option<String>,
    /// The last LoginRefused's reason.
    pub refused: Option<u8>,
    pub closed: bool,
    pub vid: Option<u32>,
}

pub struct Harness {
    pub server: Server,
    pub store: Store,
    pub now: Millis,
    pub log: String,
    pub clients: BTreeMap<ConnId, Client>,
    /// Every message sent and every event, in order.
    pub sent: Vec<(ConnId, ServerMessage)>,
    pub events: Vec<m2_server_core::Event>,
    /// Log entity kinds, angles and times too (off for the session scenarios).
    pub detailed: bool,
    /// Keep saves unanswered until [`Harness::release_saves`], as a slow
    /// database would.
    pub hold_saves: bool,
    /// Keep friendships' writes unanswered likewise.
    pub hold_friendships: bool,
    /// Keep loads of characters unanswered likewise.
    pub hold_loads: bool,
    pub held: Vec<(RequestId, StoreRequest)>,
    /// Providers' sign-ins waiting for [`Harness::answer_sign_in`].
    pub sign_ins: Vec<(RequestId, SignIn)>,
    /// Devices' keys waiting for [`Harness::answer_key_sign_in`].
    pub key_sign_ins: Vec<(RequestId, KeySignIn)>,
    /// Leave ticks that do nothing out of the log.
    pub quiet_ticks: bool,
    /// The empire the next CreateCharacter carries ([`Harness::empire`]).
    empire: Option<u8>,
    next_conn: u64,
}

impl Harness {
    pub fn new() -> Self {
        Self::with_config(Config::default())
    }

    pub fn with_config(config: Config) -> Self {
        Self::with_content(config, Content::default())
    }

    /// A server whose world holds `content`, logging in detail. Without
    /// providers in `config` it offers [`PROVIDER`].
    pub fn with_content(mut config: Config, content: Content) -> Self {
        if config.providers.is_empty() {
            config.providers.push(offer());
        }
        Self::exactly(config, content)
    }

    /// A server with no providers at all.
    pub fn without_providers() -> Self {
        Self::exactly(Config::default(), Content::default())
    }

    /// A server that takes devices' keys and has no providers.
    pub fn with_key_sign_in() -> Self {
        let config = Config {
            key_sign_in: true,
            ..Config::default()
        };
        Self::exactly(config, Content::default())
    }

    fn exactly(config: Config, content: Content) -> Self {
        let detailed = !content.groups().is_empty() || !content.npcs().is_empty();
        Self {
            server: Server::with_content(config, Arc::new(content), SEED).started_at(EPOCH),
            store: Store::default(),
            now: Millis(1_000),
            log: String::new(),
            clients: BTreeMap::new(),
            sent: Vec::new(),
            events: Vec::new(),
            detailed,
            hold_saves: false,
            hold_friendships: false,
            hold_loads: false,
            held: Vec::new(),
            sign_ins: Vec::new(),
            key_sign_ins: Vec::new(),
            quiet_ticks: false,
            empire: None,
            next_conn: 0,
        }
    }

    pub fn note(&mut self, text: &str) {
        let _ = writeln!(self.log, "-- {text}");
    }

    pub fn wait(&mut self, ms: u64) {
        self.now = self.now.plus(ms);
    }

    pub fn tick(&mut self) {
        self.step(Input::Tick);
    }

    pub fn step(&mut self, input: Input) -> Vec<Effect> {
        let line = format!("@{} {}", self.now.0, show_input(&input));
        let tick = matches!(input, Input::Tick);
        let effects = self.server.step(self.now, input);
        if !(self.quiet_ticks && tick && effects.is_empty()) {
            let _ = writeln!(self.log, "{line}");
        }
        let mut all = Vec::new();
        for effect in effects {
            let _ = writeln!(self.log, "   -> {}", show_effect_as(&effect, self.detailed));
            match &effect {
                Effect::Send { conn, message } => {
                    self.sent.push((*conn, message.clone()));
                    let client = self.clients.entry(*conn).or_default();
                    match message {
                        ServerMessage::Welcome(_) => client.welcomed = true,
                        ServerMessage::SignedIn(s) => {
                            client.ticket = Some(s.ticket.clone());
                            client.account = Some(s.account.clone());
                        }
                        ServerMessage::SignInOptions(o) => client.options = Some(o.clone()),
                        ServerMessage::LoginRefused(r) => client.refused = Some(r.reason),
                        ServerMessage::MainCharacter(m) => client.vid = Some(m.vid),
                        _ => {}
                    }
                }
                Effect::Close { conn } => self.clients.entry(*conn).or_default().closed = true,
                Effect::Event(event) => self.events.push(event.clone()),
                Effect::Store { .. }
                | Effect::CheckSignIn { .. }
                | Effect::CheckKeySignIn { .. } => {}
            }
            all.push(effect);
        }
        let mut answers = Vec::new();
        let (checks, mut all): (Vec<Effect>, Vec<Effect>) = all.into_iter().partition(|effect| {
            matches!(
                effect,
                Effect::CheckSignIn { .. } | Effect::CheckKeySignIn { .. }
            )
        });
        for effect in checks {
            match effect {
                Effect::CheckSignIn { request, sign_in } => self.sign_ins.push((request, sign_in)),
                Effect::CheckKeySignIn { request, sign_in } => {
                    self.key_sign_ins.push((request, sign_in));
                }
                _ => {}
            }
        }
        for effect in &mut all {
            if let Effect::Store { request, what } = effect {
                let what = std::mem::replace(
                    what,
                    StoreRequest::LoadCharacters {
                        account: AccountId(0),
                    },
                );
                let held =
                    match what {
                        StoreRequest::SaveCharacter { .. } => self.hold_saves,
                        StoreRequest::AddFriendship { .. }
                        | StoreRequest::RemoveFriendship { .. } => self.hold_friendships,
                        StoreRequest::LoadCharacters { .. } => self.hold_loads,
                        StoreRequest::IdentityLogin { .. }
                        | StoreRequest::CreateCharacter { .. } => false,
                    };
                if held {
                    self.held.push((*request, what));
                    continue;
                }
                answers.push((*request, self.store.answer(what)));
            }
        }
        for (request, answer) in answers {
            all.extend(self.step(Input::Stored { request, answer }));
        }
        all
    }

    /// Accepts a connection and says Hello as the client does.
    pub fn connect(&mut self, listener: Listener) -> ConnId {
        self.connect_from(listener, HOME)
    }

    pub fn connect_from(&mut self, listener: Listener, peer: IpAddr) -> ConnId {
        let conn = self.accept_from(listener, peer);
        if !self.client(conn).closed {
            self.hello(conn, PROTOCOL);
        }
        conn
    }

    /// Accepts a connection without saying anything on it.
    pub fn accept_from(&mut self, listener: Listener, peer: IpAddr) -> ConnId {
        self.next_conn += 1;
        let conn = ConnId(self.next_conn);
        self.step(Input::Accepted {
            conn,
            listener,
            peer,
        });
        conn
    }

    pub fn hello(&mut self, conn: ConnId, protocol: u16) {
        let hello = Hello {
            protocol,
            build: "openm2 test".to_owned(),
        };
        self.send(conn, ClientMessage::Hello(hello));
    }

    pub fn client(&self, conn: ConnId) -> Client {
        self.clients.get(&conn).cloned().unwrap_or_default()
    }

    pub fn send(&mut self, conn: ConnId, message: ClientMessage) -> Vec<Effect> {
        self.step(Input::Message(conn, message))
    }

    pub fn close(&mut self, conn: ConnId) {
        self.step(Input::Closed {
            conn,
            reason: "the client left".to_owned(),
        });
    }

    /// Signs in on an auth connection as the provider's player `subject`:
    /// StartSignIn, ProviderLogin and, when the server hands it to the edge,
    /// the edge's answer that the provider confirmed it.
    pub fn login(&mut self, conn: ConnId, subject: &str) {
        self.start_sign_in(conn);
        let waiting = self.sign_ins.len();
        self.provider_login(conn, PROVIDER, "the-code");
        if self.sign_ins.len() > waiting {
            let (request, _) = self.sign_ins.remove(waiting);
            let result = Ok(identity(subject));
            self.step(Input::SignInChecked { request, result });
        }
    }

    pub fn ticket(&mut self, conn: ConnId, user: &str, ticket: &[u8]) {
        let login = TicketLogin {
            username: user.to_owned(),
            ticket: ticket.to_vec(),
        };
        self.send(conn, ClientMessage::TicketLogin(login));
    }

    /// Signs in on the auth server as `subject` and joins the game server
    /// with the ticket; returns the game connection.
    pub fn join(&mut self, subject: &str) -> ConnId {
        let auth = self.connect(Listener::Auth);
        self.login(auth, subject);
        let client = self.client(auth);
        let (account, ticket) = (
            client.account.expect("signed in"),
            client.ticket.expect("signed in"),
        );
        self.close(auth);
        let game = self.connect(Listener::Game);
        self.ticket(game, &account, &ticket);
        game
    }

    /// Chooses the empire the next character asked for carries: the
    /// account's first character picks it.
    pub fn empire(&mut self, _conn: ConnId, empire: u8) {
        self.empire = Some(empire);
    }

    pub fn create(&mut self, conn: ConnId, slot: u8, name: &str, class: u8) {
        let create = CreateCharacter {
            slot,
            name: name.to_owned(),
            class,
            empire: self.empire,
        };
        self.send(conn, ClientMessage::CreateCharacter(create));
    }

    pub fn select(&mut self, conn: ConnId, slot: u8) {
        self.send(
            conn,
            ClientMessage::SelectCharacter(SelectCharacter { slot }),
        );
    }

    pub fn enter(&mut self, conn: ConnId) {
        self.send(conn, ClientMessage::EnterGame(EnterGame));
    }

    /// Joins, creates a character in slot 0 (on an empty account) and enters.
    pub fn play(&mut self, subject: &str, name: &str, empire: u8) -> ConnId {
        let game = self.join(subject);
        self.empire(game, empire);
        self.create(game, 0, name, 0);
        self.select(game, 0);
        self.enter(game);
        game
    }

    pub fn walk(&mut self, conn: ConnId, kind: u8, x: i32, y: i32) {
        self.walk_at(conn, kind, x, y, self.now);
    }

    /// A move that says it was made at `time` (the client's guess at the
    /// server's clock), as one sent late or delayed on the way.
    pub fn walk_at(&mut self, conn: ConnId, kind: u8, x: i32, y: i32, time: Millis) {
        let walk = CharacterMove {
            kind,
            argument: 0,
            angle: 90,
            x,
            y,
            time: time.wire(),
        };
        self.send(conn, ClientMessage::CharacterMove(walk));
    }

    /// The character on `conn` walks from `from` to `to` as the client
    /// does: the walk, then, once it is over (at 3 m/s, slower than any
    /// walker), the stop. The server checks moves, so a stop far from
    /// where it has the character is refused.
    pub fn go(&mut self, conn: ConnId, from: (i32, i32), to: (i32, i32)) {
        self.walk(conn, 1, to.0, to.1);
        let dx = f64::from(to.0) - f64::from(from.0);
        let dy = f64::from(to.1) - f64::from(from.1);
        self.wait((dx.hypot(dy) / 300.0 * 1000.0).ceil() as u64);
        self.walk(conn, 0, to.0, to.1);
    }

    pub fn say(&mut self, conn: ConnId, kind: u8, text: &str) {
        let chat = SendChat {
            kind,
            text: text.to_owned(),
        };
        self.send(conn, ClientMessage::SendChat(chat));
    }

    pub fn pong(&mut self, conn: ConnId) {
        self.send(conn, ClientMessage::Pong(Pong));
    }

    pub fn set_target(&mut self, conn: ConnId, vid: u32) {
        self.send(conn, ClientMessage::SetTarget(SetTarget { vid }));
    }

    pub fn attack(&mut self, conn: ConnId, vid: u32) {
        self.send(conn, ClientMessage::Hit(Hit { vid, skill: 0 }));
    }

    /// A hit of skill `skill` on `vid`.
    pub fn skill_hit(&mut self, conn: ConnId, skill: u8, vid: u32) {
        self.send(conn, ClientMessage::Hit(Hit { vid, skill }));
    }

    pub fn use_skill(&mut self, conn: ConnId, skill: u8, target: Option<u32>) {
        self.send(conn, ClientMessage::UseSkill(UseSkill { skill, target }));
    }

    pub fn raise_skill(&mut self, conn: ConnId, skill: u8) {
        self.send(conn, ClientMessage::RaiseSkill(RaiseSkill { skill }));
    }

    pub fn raise_status(&mut self, conn: ConnId, point: u8) {
        self.send(conn, ClientMessage::RaiseStatus(RaiseStatus { point }));
    }

    pub fn restart(&mut self, conn: ConnId, town: bool) {
        self.send(conn, ClientMessage::Restart(Restart { town }));
    }

    pub fn talk(&mut self, conn: ConnId, vid: u32) {
        self.send(conn, ClientMessage::Talk(Talk { vid }));
    }

    pub fn ride(&mut self, conn: ConnId, ride: bool) {
        self.send(
            conn,
            ClientMessage::RideMount(m2_protocol::profile::openm2::RideMount { ride }),
        );
    }

    pub fn train(&mut self, conn: ConnId, vid: u32) {
        self.send(
            conn,
            ClientMessage::TrainMount(m2_protocol::profile::openm2::TrainMount { vid }),
        );
    }

    pub fn cast(&mut self, conn: ConnId, angle: u16) {
        self.send(
            conn,
            ClientMessage::Cast(m2_protocol::profile::openm2::Cast { angle }),
        );
    }

    pub fn reel_in(&mut self, conn: ConnId) {
        self.send(
            conn,
            ClientMessage::ReelIn(m2_protocol::profile::openm2::ReelIn),
        );
    }

    pub fn give_item(&mut self, conn: ConnId, vid: u32, slot: u16) {
        self.send(
            conn,
            ClientMessage::GiveItem(m2_protocol::profile::openm2::GiveItem { vid, slot }),
        );
    }

    pub fn use_item_on_item(&mut self, conn: ConnId, slot: u16, onto: u16) {
        self.send(
            conn,
            ClientMessage::UseItemOnItem(m2_protocol::profile::openm2::UseItemOnItem {
                slot,
                onto,
            }),
        );
    }

    pub fn upgrade(
        &mut self,
        conn: ConnId,
        smith: Option<u32>,
        scroll: Option<u16>,
        slot: u16,
        vnum: u32,
    ) {
        self.send(
            conn,
            ClientMessage::Upgrade(m2_protocol::profile::openm2::Upgrade {
                smith,
                scroll,
                slot,
                vnum,
            }),
        );
    }

    pub fn learn(&mut self, conn: ConnId, vid: u32) {
        self.send(
            conn,
            ClientMessage::LearnSkillGroup(LearnSkillGroup { vid }),
        );
    }

    pub fn buy(&mut self, conn: ConnId, position: u8) {
        self.send(conn, ClientMessage::ShopBuy(ShopBuy { position }));
    }

    pub fn sell(&mut self, conn: ConnId, slot: u16, count: u8) {
        self.send(conn, ClientMessage::ShopSell(ShopSell { slot, count }));
    }

    pub fn close_shop(&mut self, conn: ConnId) {
        self.send(conn, ClientMessage::ShopClose(ShopClose));
    }

    pub fn use_item(&mut self, conn: ConnId, slot: u16) {
        self.send(conn, ClientMessage::UseItem(UseItem { window: 1, slot }));
    }

    pub fn use_item_on_skill(&mut self, conn: ConnId, slot: u16, skill: u8) {
        self.send(
            conn,
            ClientMessage::UseItemOnSkill(UseItemOnSkill { slot, skill }),
        );
    }

    /// A new server on the same store, as after a restart, with its time 0
    /// at Unix time `epoch`; the connections are gone.
    pub fn restart_server(&mut self, content: Content, epoch: u64) {
        let config = self.server.config().clone();
        self.server = Server::with_content(config, Arc::new(content), SEED).started_at(epoch);
        self.clients.clear();
        self.note(&format!(
            "restarted, time 0 at the epoch +{}s",
            epoch - EPOCH
        ));
    }

    pub fn move_item(&mut self, conn: ConnId, from: u16, to: u16) {
        let moved = MoveItem {
            from_window: 1,
            from_slot: from,
            to_window: 1,
            to_slot: to,
            count: 0,
        };
        self.send(conn, ClientMessage::MoveItem(moved));
    }

    pub fn drop_item(&mut self, conn: ConnId, slot: u16, count: u8) {
        let drop = DropItem {
            window: 1,
            slot,
            count,
        };
        self.send(conn, ClientMessage::DropItem(drop));
    }

    pub fn drop_gold(&mut self, conn: ConnId, amount: u32) {
        self.send(conn, ClientMessage::DropGold(DropGold { amount }));
    }

    pub fn pick_up(&mut self, conn: ConnId, vid: u32) {
        self.send(conn, ClientMessage::PickUp(PickUp { vid }));
    }

    pub fn set_quickslot(&mut self, conn: ConnId, index: u8, kind: u8, position: u8) {
        let set = SetQuickSlot {
            index,
            kind,
            position,
        };
        self.send(conn, ClientMessage::SetQuickSlot(set));
    }

    pub fn clear_quickslot(&mut self, conn: ConnId, index: u8) {
        self.send(
            conn,
            ClientMessage::ClearQuickSlot(ClearQuickSlot { index }),
        );
    }

    pub fn swap_quickslots(&mut self, conn: ConnId, first: u8, second: u8) {
        self.send(
            conn,
            ClientMessage::SwapQuickSlots(SwapQuickSlots { first, second }),
        );
    }

    /// The ground items `conn` sees now, as (vid, vnum), from what it was
    /// sent.
    pub fn ground_seen(&self, conn: ConnId) -> Vec<(u32, u32)> {
        let mut seen = BTreeMap::new();
        for (to, message) in &self.sent {
            if *to != conn {
                continue;
            }
            match message {
                ServerMessage::GroundItemAdd(add) => {
                    seen.insert(add.vid, add.vnum);
                }
                ServerMessage::GroundItemRemove(remove) => {
                    seen.remove(&remove.vid);
                }
                _ => {}
            }
        }
        seen.into_iter().collect()
    }

    /// The points `conn` holds now, by number, from the Points and
    /// PointsChanged it was sent.
    pub fn points(&self, conn: ConnId) -> BTreeMap<u8, u32> {
        let mut points = BTreeMap::new();
        for (to, message) in &self.sent {
            if *to != conn {
                continue;
            }
            match message {
                ServerMessage::Points(all) => {
                    points = all.points.iter().map(|p| (p.point, p.value)).collect();
                }
                ServerMessage::PointsChanged(changed) => {
                    for p in &changed.points {
                        if p.value == 0 {
                            points.remove(&p.point);
                        } else {
                            points.insert(p.point, p.value);
                        }
                    }
                }
                _ => {}
            }
        }
        points
    }

    /// Point `point` of `conn` now; 0 when it was never sent.
    pub fn point(&self, conn: ConnId, point: u8) -> u32 {
        self.points(conn).get(&point).copied().unwrap_or(0)
    }

    /// Asks for the providers on an auth connection.
    pub fn start_sign_in(&mut self, conn: ConnId) {
        self.send(conn, ClientMessage::StartSignIn(StartSignIn));
    }

    /// Relays a provider's code, with a good verifier and redirect.
    pub fn provider_login(&mut self, conn: ConnId, provider: &str, code: &str) {
        let login = ProviderLogin {
            provider: provider.to_owned(),
            code: code.to_owned(),
            verifier: "v".repeat(43),
            redirect: "http://127.0.0.1:50000/callback".to_owned(),
        };
        self.send(conn, ClientMessage::ProviderLogin(login));
    }

    /// Answers the oldest sign-in check waiting.
    pub fn answer_sign_in(&mut self, result: Result<Identity, SignInRefusal>) -> SignIn {
        let (request, sign_in) = self.sign_ins.remove(0);
        self.step(Input::SignInChecked { request, result });
        sign_in
    }

    /// Sends a device's key and a signature; the core never checks the
    /// signature itself, the edge does.
    pub fn key_login(&mut self, conn: ConnId, public_key: [u8; 32]) {
        let login = KeyLogin {
            public_key,
            signature: [0x5A; 64],
        };
        self.send(conn, ClientMessage::KeyLogin(login));
    }

    /// Answers the oldest key check waiting.
    pub fn answer_key_sign_in(&mut self, result: Result<Identity, SignInRefusal>) -> KeySignIn {
        let (request, sign_in) = self.key_sign_ins.remove(0);
        self.step(Input::SignInChecked { request, result });
        sign_in
    }

    /// Answers the saves held so far, in order.
    pub fn release_saves(&mut self) {
        for (request, what) in std::mem::take(&mut self.held) {
            let answer = self.store.answer(what);
            self.step(Input::Stored { request, answer });
        }
    }
}

pub fn show_input(input: &Input) -> String {
    match input {
        Input::Accepted {
            conn,
            listener,
            peer,
        } => format!("{conn} accepted on {listener} from {peer}"),
        Input::Message(conn, message) => format!("{conn} sends {}", show_client(message)),
        Input::Closed { conn, reason } => format!("{conn} closed: {reason}"),
        Input::Stored { request, answer } => {
            format!("store answers #{}: {}", request.0, show_answer(answer))
        }
        Input::SignInChecked { request, result } => match result {
            Ok(identity) => format!(
                "sign-in #{} checked: {} {}",
                request.0, identity.issuer, identity.subject
            ),
            Err(refusal) => format!("sign-in #{} refused: {refusal:?}", request.0),
        },
        Input::Tick => "tick".to_owned(),
        Input::Shutdown => "shutdown".to_owned(),
    }
}

/// A ticket for the log: its length and first two bytes, never all of it.
fn show_ticket(ticket: &[u8]) -> String {
    match ticket {
        [a, b, ..] => format!("{a:02x}{b:02x}… ({} bytes)", ticket.len()),
        _ => format!("({} bytes)", ticket.len()),
    }
}

fn show_client(message: &ClientMessage) -> String {
    match message {
        ClientMessage::Hello(h) => format!("Hello protocol={} {:?}", h.protocol, h.build),
        ClientMessage::TicketLogin(t) => {
            format!("TicketLogin {} {}", t.username, show_ticket(&t.ticket))
        }
        ClientMessage::CreateCharacter(c) => format!(
            "CreateCharacter slot={} {} class={}{}",
            c.slot,
            c.name,
            c.class,
            c.empire
                .map_or_else(String::new, |e| format!(" empire={e}"))
        ),
        ClientMessage::SelectCharacter(s) => format!("SelectCharacter {}", s.slot),
        ClientMessage::CharacterMove(m) => {
            format!("CharacterMove kind={} ({}, {})", m.kind, m.x, m.y)
        }
        ClientMessage::SendChat(c) => format!("SendChat kind={} {:?}", c.kind, c.text),
        ClientMessage::SendWhisper(w) => format!("SendWhisper to={} {:?}", w.name, w.text),
        ClientMessage::AskFriend(a) => format!("AskFriend {}", a.name),
        ClientMessage::AnswerFriend(a) => format!("AnswerFriend {} accept={}", a.name, a.accept),
        ClientMessage::RemoveFriend(r) => format!("RemoveFriend {}", r.name),
        ClientMessage::SetTarget(t) => format!("SetTarget vid={}", t.vid),
        ClientMessage::Hit(h) => format!("Hit vid={} skill={}", h.vid, h.skill),
        ClientMessage::UseItem(u) => format!("UseItem {}:{}", u.window, u.slot),
        ClientMessage::MoveItem(m) => format!(
            "MoveItem {}:{} -> {}:{} count={}",
            m.from_window, m.from_slot, m.to_window, m.to_slot, m.count
        ),
        ClientMessage::DropItem(d) => format!("DropItem {}:{} count={}", d.window, d.slot, d.count),
        ClientMessage::DropGold(d) => format!("DropGold {}", d.amount),
        ClientMessage::PickUp(p) => format!("PickUp vid={}", p.vid),
        ClientMessage::SetQuickSlot(q) => format!(
            "SetQuickSlot {} kind={} position={}",
            q.index, q.kind, q.position
        ),
        ClientMessage::ClearQuickSlot(q) => format!("ClearQuickSlot {}", q.index),
        ClientMessage::SwapQuickSlots(q) => format!("SwapQuickSlots {} {}", q.first, q.second),
        ClientMessage::Talk(t) => format!("Talk vid={}", t.vid),
        ClientMessage::ShopBuy(b) => format!("ShopBuy {}", b.position),
        ClientMessage::ShopSell(s) => format!("ShopSell {} count={}", s.slot, s.count),
        ClientMessage::UseSkill(u) => format!("UseSkill {} target={:?}", u.skill, u.target),
        ClientMessage::RaiseSkill(r) => format!("RaiseSkill {}", r.skill),
        ClientMessage::RaiseStatus(r) => format!("RaiseStatus {}", r.point),
        ClientMessage::LearnSkillGroup(l) => format!("LearnSkillGroup vid={}", l.vid),
        ClientMessage::UseItemOnSkill(u) => format!("UseItemOnSkill {} skill={}", u.slot, u.skill),
        ClientMessage::Restart(r) => format!("Restart town={}", r.town),
        ClientMessage::RideMount(r) => format!("RideMount ride={}", r.ride),
        ClientMessage::TrainMount(t) => format!("TrainMount vid={}", t.vid),
        ClientMessage::GiveItem(a) => format!("GiveItem vid={} {}", a.vid, a.slot),
        ClientMessage::UseItemOnItem(u) => format!("UseItemOnItem {} onto {}", u.slot, u.onto),
        ClientMessage::Upgrade(u) => format!(
            "Upgrade smith={:?} scroll={:?} {} vnum={}",
            u.smith, u.scroll, u.slot, u.vnum
        ),
        ClientMessage::ProviderLogin(p) => format!("ProviderLogin {} {}", p.provider, p.redirect),
        other => other.name().to_owned(),
    }
}

fn show_answer(answer: &StoreAnswer) -> String {
    match answer {
        StoreAnswer::LoginChecked(Ok(a)) => format!("login ok, account {} {:?}", a.id.0, a.empire),
        StoreAnswer::LoginChecked(Err(e)) => format!("login failed: {}", e.0),
        StoreAnswer::Characters(Ok(list)) => format!(
            "characters [{}]",
            list.iter()
                .map(|c| format!("{}:{}", c.slot, c.name))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        StoreAnswer::Characters(Err(e)) => format!("characters failed: {}", e.0),
        StoreAnswer::CharacterCreated(Ok(c)) => {
            format!("created #{} {} in slot {}", c.id, c.name, c.slot)
        }
        StoreAnswer::CharacterCreated(Err(e)) => format!("not created: {e:?}"),
        StoreAnswer::Saved(r) => format!("saved: {r:?}"),
        StoreAnswer::FriendshipWritten(r) => format!("friendship written: {r:?}"),
    }
}

pub fn show_effect(effect: &Effect) -> String {
    show_effect_as(effect, false)
}

pub fn show_effect_as(effect: &Effect, detailed: bool) -> String {
    match effect {
        Effect::Send { conn, message } => {
            format!("send {conn} {}", show_server_as(message, detailed))
        }
        Effect::Close { conn } => format!("close {conn}"),
        Effect::Store { request, what } => format!("store #{}: {}", request.0, show_request(what)),
        Effect::CheckSignIn { request, sign_in } => format!(
            "check sign-in #{}: {} {} nonce={}",
            request.0, sign_in.provider, sign_in.redirect, sign_in.nonce
        ),
        Effect::CheckKeySignIn { request, sign_in } => format!(
            "check key sign-in #{}: {} {} nonce={}",
            request.0,
            sign_in.conn,
            m2_protocol::key_sign_in::PublicKey::from_bytes(sign_in.public_key),
            sign_in.nonce
        ),
        Effect::Event(event) => format!("event: {event}"),
    }
}

fn show_request(what: &StoreRequest) -> String {
    match what {
        StoreRequest::IdentityLogin {
            provider,
            issuer,
            subject,
        } => format!("identity login {provider} {issuer} {subject}"),
        StoreRequest::LoadCharacters { account } => format!("load characters of {}", account.0),
        StoreRequest::CreateCharacter { account, character } => format!(
            "create {} for {} in slot {} at ({}, {}) empire {}{}",
            character.name,
            account.0,
            character.slot,
            character.x,
            character.y,
            character.empire,
            belongings(character)
        ),
        StoreRequest::SaveCharacter { character, .. } => format!(
            "save #{} at ({}, {}) played {}s{}",
            character.id,
            character.x,
            character.y,
            character.playtime,
            belongings(character)
        ),
        StoreRequest::AddFriendship { characters: [a, b] } => {
            format!("make #{a} and #{b} friends")
        }
        StoreRequest::RemoveFriendship { characters: [a, b] } => {
            format!("end the friendship of #{a} and #{b}")
        }
    }
}

/// A stored character's gold, items, quickslots, skill group and skills,
/// when it has any.
fn belongings(character: &Character) -> String {
    let mut text = String::new();
    if character.gold > 0 {
        let _ = write!(text, " gold={}", character.gold);
    }
    if !character.items.is_empty() {
        let items: Vec<String> = character
            .items
            .iter()
            .map(|item| format!("{}:{}x{}", item.slot, item.vnum, item.count))
            .collect();
        let _ = write!(text, " items=[{}]", items.join(" "));
    }
    if !character.quickslots.is_empty() {
        let quick: Vec<String> = character
            .quickslots
            .iter()
            .map(|q| format!("{}:{}/{}", q.index, q.kind, q.position))
            .collect();
        let _ = write!(text, " quickslots=[{}]", quick.join(" "));
    }
    if character.skill_group != 0 {
        let _ = write!(text, " group={}", character.skill_group);
    }
    if character.mount_level != 0 {
        let _ = write!(
            text,
            " mount={} stamina={} mount_points={}",
            character.mount_level, character.mount_stamina, character.mount_skill_points
        );
    }
    if !character.skills.is_empty() {
        let skills: Vec<String> = character
            .skills
            .iter()
            .map(|s| match s.next_read {
                0 => format!("{}={}", s.skill, s.level),
                next => format!("{}={}@+{}s", s.skill, s.level, next.saturating_sub(EPOCH)),
            })
            .collect();
        let _ = write!(text, " skills=[{}]", skills.join(" "));
    }
    if character.read_bonus != 0 {
        let _ = write!(text, " read_bonus={}", character.read_bonus);
    }
    text
}

pub fn show_server(message: &ServerMessage) -> String {
    show_server_as(message, false)
}

/// Points as `number=value`, by number.
fn show_points(points: &[m2_protocol::profile::openm2::PointValue]) -> String {
    points
        .iter()
        .map(|p| format!("{}={}", p.point, p.value))
        .collect::<Vec<_>>()
        .join(" ")
}

/// EntitySpawn's and EntityUpdate's state flags, as words after a line.
fn show_state(state: u8) -> String {
    let mut words = String::new();
    if state & 1 != 0 {
        words.push_str(" dead");
    }
    if state & 2 != 0 {
        words.push_str(" game-master");
    }
    words
}

/// EntitySpawn's and EntityUpdate's `affects`, in words.
fn show_affects(affects: u64) -> String {
    let names = ["poisoned", "burning", "slowed", "stunned", "hidden"];
    let mut words = String::new();
    for (bit, name) in names.iter().enumerate() {
        if affects & (1 << bit) != 0 {
            words.push(' ');
            words.push_str(name);
        }
    }
    if affects >> names.len() != 0 {
        words.push_str(&format!(" affects={affects:#x}"));
    }
    words
}

pub fn show_server_as(message: &ServerMessage, detailed: bool) -> String {
    match message {
        ServerMessage::EntitySpawn(s) if detailed => format!(
            "EntitySpawn vid={} ({}, {}) kind={} race={} angle={} speed={}/{}{}{}",
            s.vid,
            s.x,
            s.y,
            s.kind,
            s.race,
            s.angle,
            s.move_speed,
            s.attack_speed,
            show_state(s.state),
            show_affects(s.affects)
        ),
        ServerMessage::EntityMove(m) if detailed => format!(
            "EntityMove kind={} vid={} ({}, {}) angle={} duration={}",
            m.kind, m.vid, m.x, m.y, m.angle, m.duration
        ),
        ServerMessage::Welcome(w) => format!("Welcome protocol={}", w.protocol),
        ServerMessage::Refused(r) => format!(
            "Refused reason={} versions {}-{} {:?}",
            r.reason, r.oldest, r.newest, r.text
        ),
        ServerMessage::Phase(p) => format!("Phase {}", p.phase),
        ServerMessage::SignedIn(s) => format!("SignedIn {} {}", s.account, show_ticket(&s.ticket)),
        ServerMessage::SignInOptions(o) => format!(
            "SignInOptions [{}] nonce={:?}{}",
            o.providers
                .iter()
                .map(|p| p.id.as_str())
                .collect::<Vec<_>>()
                .join(", "),
            o.nonce,
            if o.key_sign_in { " key" } else { "" }
        ),
        ServerMessage::LoginRefused(r) => format!("LoginRefused {}", r.reason),
        ServerMessage::Empire(e) => format!("Empire {}", e.empire),
        ServerMessage::CharacterList(list) => format!(
            "CharacterList [{}]",
            list.characters
                .iter()
                .map(|c| format!(
                    "{}: #{} {} lv{}{}",
                    c.slot,
                    c.id,
                    c.name,
                    c.level,
                    if c.body_part == 0 {
                        String::new()
                    } else {
                        format!(" body={}", c.body_part)
                    }
                ))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        ServerMessage::CharacterCreated(c) => format!(
            "CharacterCreated slot={} #{} {} class={} st={} ht={} dx={} iq={}",
            c.character.slot,
            c.character.id,
            c.character.name,
            c.character.class,
            c.character.st,
            c.character.ht,
            c.character.dx,
            c.character.iq
        ),
        ServerMessage::CharacterCreateFailed(f) => format!("CharacterCreateFailed {}", f.reason),
        ServerMessage::MainCharacter(m) => format!(
            "MainCharacter vid={} {} class={} ({}, {}) empire={} group={} run={}",
            m.vid, m.name, m.class, m.x, m.y, m.empire, m.skill_group, m.run_speed
        ),
        ServerMessage::MoveRefused(r) => format!("MoveRefused ({}, {})", r.x, r.y),
        ServerMessage::Points(p) => format!("Points {}", show_points(&p.points)),
        ServerMessage::PointsChanged(p) => format!("PointsChanged {}", show_points(&p.points)),
        ServerMessage::GameTime(t) => format!("GameTime {}", t.time),
        ServerMessage::Channel(c) => format!("Channel {}", c.channel),
        ServerMessage::EntitySpawn(s) => format!(
            "EntitySpawn vid={} ({}, {}) race={} speed={}/{}{}{}",
            s.vid,
            s.x,
            s.y,
            s.race,
            s.move_speed,
            s.attack_speed,
            show_state(s.state),
            show_affects(s.affects)
        ),
        ServerMessage::EntityInfo(i) => format!(
            "EntityInfo vid={} {} empire={} level={}{}",
            i.vid,
            i.name,
            i.empire,
            i.level,
            if i.parts == [0; 4] {
                String::new()
            } else {
                format!(" parts={:?}", i.parts)
            }
        ),
        ServerMessage::EntityUpdate(u) => format!(
            "EntityUpdate vid={} parts={:?} speed={}/{}{}{}",
            u.vid,
            u.parts,
            u.move_speed,
            u.attack_speed,
            show_state(u.state),
            show_affects(u.affects)
        ),
        ServerMessage::ItemSet(i) if i.vnum == 0 => {
            format!("ItemSet {}:{} empty", i.window, i.slot)
        }
        ServerMessage::ItemSet(i) => {
            format!("ItemSet {}:{} {}x{}", i.window, i.slot, i.vnum, i.count)
        }
        ServerMessage::GroundItemAdd(g) => {
            format!("GroundItemAdd vid={} {} ({}, {})", g.vid, g.vnum, g.x, g.y)
        }
        ServerMessage::GroundItemRemove(g) => format!("GroundItemRemove vid={}", g.vid),
        ServerMessage::ItemOwnership(o) => format!(
            "ItemOwnership vid={} {}",
            o.vid,
            if o.owner.is_empty() {
                "anyone"
            } else {
                &o.owner
            }
        ),
        ServerMessage::QuickSlotSet(q) => format!(
            "QuickSlotSet {} kind={} position={}",
            q.index, q.kind, q.position
        ),
        ServerMessage::QuickSlotCleared(q) => format!("QuickSlotCleared {}", q.index),
        ServerMessage::ShopOpened(opened) => format!(
            "ShopOpened vid={} [{}]",
            opened.vid,
            opened
                .goods
                .iter()
                .map(|good| format!(
                    "{}: {}x{} for {}",
                    good.position, good.vnum, good.count, good.price
                ))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        ServerMessage::ShopRefused(r) => format!("ShopRefused {}", r.reason),
        ServerMessage::SkillGroup(g) => format!("SkillGroup {}", g.group),
        ServerMessage::TeacherOpened(t) => format!(
            "TeacherOpened vid={} job={} group={} level={}",
            t.vid, t.job, t.group, t.level
        ),
        ServerMessage::ChooseSkill(c) => {
            format!("ChooseSkill slot={} skills={:?}", c.slot, c.skills)
        }
        ServerMessage::Affects(a) => {
            let affects: Vec<String> = a
                .affects
                .iter()
                .map(|a| match a.seconds {
                    0 => format!("{}:{}={} on", a.skill, a.point, a.amount),
                    s => format!("{}:{}={} {s}s", a.skill, a.point, a.amount),
                })
                .collect();
            if affects.is_empty() {
                "Affects".to_owned()
            } else {
                format!("Affects {}", affects.join(", "))
            }
        }
        ServerMessage::Skills(skills) => {
            let learned: Vec<String> = skills
                .skills
                .iter()
                .map(|s| match u64::from(s.next_read) {
                    0 => format!("{}={} grade {}", s.skill, s.level, s.grade),
                    // The wait's end, from the epoch.
                    next => format!(
                        "{}={} grade {} next at +{}s",
                        s.skill,
                        s.level,
                        s.grade,
                        next.saturating_sub(EPOCH)
                    ),
                })
                .collect();
            if learned.is_empty() {
                "Skills".to_owned()
            } else {
                format!("Skills {}", learned.join(", "))
            }
        }
        ServerMessage::QuickSlotsSwapped(q) => {
            format!("QuickSlotsSwapped {} {}", q.first, q.second)
        }
        ServerMessage::EntityRemove(r) => format!("EntityRemove vid={}", r.vid),
        ServerMessage::EntityDied(d) => format!("EntityDied vid={}", d.vid),
        ServerMessage::Flight(f) => format!("Flight kind={} from={} to={}", f.kind, f.from, f.to),
        ServerMessage::EntityEffect(e) => format!("EntityEffect kind={} vid={}", e.kind, e.vid),
        ServerMessage::EntitySkills(e) => {
            format!("EntitySkills vid={} skills={:?}", e.vid, e.skills)
        }
        ServerMessage::Damage(d) => {
            format!("Damage vid={} flags={} amount={}", d.vid, d.flags, d.amount)
        }
        ServerMessage::TargetHealth(t) => format!("TargetHealth vid={} {}%", t.vid, t.percent),
        ServerMessage::EntityMove(m) => format!(
            "EntityMove kind={} vid={} ({}, {}) duration={}",
            m.kind, m.vid, m.x, m.y, m.duration
        ),
        ServerMessage::Chat(c) => format!("Chat kind={} vid={} {:?}", c.kind, c.vid, c.text),
        ServerMessage::Whisper(w) => format!(
            "Whisper {} {} {:?}",
            if w.mine { "to" } else { "from" },
            w.name,
            w.text
        ),
        ServerMessage::WhisperRefused(r) => {
            format!("WhisperRefused {} reason={}", r.name, r.reason)
        }
        ServerMessage::Friends(f) => format!(
            "Friends [{}]",
            f.friends
                .iter()
                .map(|friend| format!(
                    "{}{}",
                    friend.name,
                    if friend.online { " (online)" } else { "" }
                ))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        ServerMessage::FriendChanged(c) => format!("FriendChanged {} status={}", c.name, c.status),
        ServerMessage::FriendAsked(a) => format!("FriendAsked {}", a.name),
        ServerMessage::FriendNotice(n) => format!("FriendNotice {} notice={}", n.name, n.notice),
        ServerMessage::EntityMount(m) => format!("EntityMount vid={} look={}", m.vid, m.look),
        ServerMessage::MountState(m) => format!(
            "MountState level={} grade={} look={} stamina={}/{} ride_from={} fights={} skills={} points={} summoned={:?}",
            m.level,
            m.grade,
            m.look,
            m.stamina,
            m.most,
            m.ride_from,
            m.fights,
            m.skills,
            m.skill_points,
            m.summoned
        ),
        ServerMessage::RunSpeed(r) => format!("RunSpeed {}", r.run_speed),
        ServerMessage::StableOpened(s) => match &s.next {
            None => format!("StableOpened vid={} next=none", s.vid),
            Some(n) => format!(
                "StableOpened vid={} next=level {} grade {} look {} speed {} stamina {} rider {} gold {} items [{}] points {}",
                s.vid,
                n.level,
                n.grade,
                n.look,
                n.move_speed,
                n.stamina,
                n.rider_level,
                n.gold,
                n.items
                    .iter()
                    .map(|c| format!("{}x{}", c.vnum, c.count))
                    .collect::<Vec<_>>()
                    .join(", "),
                n.skill_points
            ),
        },
        ServerMessage::UpgradeOffer(o) => format!(
            "UpgradeOffer smith={:?} scroll={:?} {}: {} -> {} gold {} items [{}] chance {} on_fail {}",
            o.smith,
            o.scroll,
            o.slot,
            o.vnum,
            o.next,
            o.gold,
            o.items
                .iter()
                .map(|c| format!("{}x{}", c.vnum, c.count))
                .collect::<Vec<_>>()
                .join(", "),
            o.chance,
            o.on_fail
        ),
        other => other.name().to_owned(),
    }
}

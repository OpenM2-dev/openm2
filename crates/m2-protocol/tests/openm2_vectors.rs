//! Byte-exact vectors for openm2's own protocol.
//!
//! Each vector is the message exactly as it appears on the wire, **frame
//! length included**. `check_c2s`/`check_s2c` test both ways: the message
//! encodes and frames to these bytes, and these bytes deframe and decode to
//! the message. The property tests below check that decoding never panics
//! and that whatever decodes encodes back to the same bytes.
//! Layouts: crates/m2-protocol/packets/openm2.toml.

use m2_protocol::message::{Message, spec_of};
use m2_protocol::profile::openm2::{
    Affect, Affects, AnswerFriend, AskFriend, CLIENT_MESSAGES, Cast, Channel, Character,
    CharacterCreateFailed, CharacterCreated, CharacterList, CharacterMove, Chat, ChooseSkill,
    ClearQuickSlot, ClientMessage, CreateCharacter, Damage, DropGold, DropItem, Empire, EnterGame,
    EntityDied, EntityEffect, EntityInfo, EntityMount, EntityMove, EntityRemove, EntitySkills,
    EntitySpawn, EntityUpdate, Fishing, Flight, Friend, FriendAsked, FriendChanged, FriendNotice,
    Friends, GameTime, GiveItem, GroundItemAdd, GroundItemRemove, Hello, Hit, ItemBonus, ItemBoost,
    ItemBoosts, ItemOwnership, ItemSet, LearnSkillGroup, LeaveGame, LoginRefused, MAX_CLIENT_FRAME,
    MAX_SERVER_FRAME, MainCharacter, MountCost, MountOffer, MountState, MoveItem, MoveRefused,
    Phase, PickUp, Ping, PointValue, Points, PointsChanged, Pong, Projectile, Provider,
    ProviderLogin, QuickSlotCleared, QuickSlotSet, QuickSlotsSwapped, RaiseSkill, RaiseStatus,
    ReelIn, Refused, RemoveFriend, Restart, RideMount, RunSpeed, SERVER_MESSAGES, SelectCharacter,
    SendChat, SendWhisper, ServerMessage, SetQuickSlot, SetTarget, ShopBuy, ShopClose, ShopGood,
    ShopOpened, ShopRefused, ShopSell, SignInOptions, SignedIn, SkillGroup, SkillState, Skills,
    StableOpened, StartSignIn, SwapQuickSlots, Talk, TargetHealth, TeacherOpened, TicketLogin,
    TrainMount, Upgrade, UpgradeCost, UpgradeOffer, UseItem, UseItemOnItem, UseItemOnSkill,
    UseSkill, Welcome, Whisper, WhisperRefused,
};
use m2_protocol::wire::{Deframer, WireError, frame};
use m2_protocol::{DecodeError, EncodeError};
use proptest::prelude::*;

/// The frame limit the vectors use.
const MAX: u32 = 65_536;

fn deframe_one(wire: &[u8]) -> Vec<u8> {
    let mut deframer = Deframer::new(MAX);
    deframer.push(wire);
    let message = deframer.next_frame().unwrap().expect("one whole frame");
    assert_eq!(deframer.buffered(), 0, "bytes left after the frame");
    message
}

fn check_c2s(message: ClientMessage, wire: &[u8]) {
    let framed = frame(&message.to_bytes().unwrap(), MAX);
    assert_eq!(framed.unwrap(), wire, "encode {}", message.name());
    let bytes = deframe_one(wire);
    assert_eq!(ClientMessage::decode(&bytes), Ok(message));
}

fn check_s2c(message: ServerMessage, wire: &[u8]) {
    let framed = frame(&message.to_bytes().unwrap(), MAX);
    assert_eq!(framed.unwrap(), wire, "encode {}", message.name());
    let bytes = deframe_one(wire);
    assert_eq!(ServerMessage::decode(&bytes), Ok(message));
}

fn concat(parts: &[&[u8]]) -> Vec<u8> {
    parts.concat()
}

/// A character for the list vectors.
fn hero() -> Character {
    Character {
        slot: 0,
        id: 1,
        name: "Hero".into(),
        class: 0,
        level: 12,
        playtime: 90,
        st: 6,
        ht: 4,
        dx: 3,
        iq: 3,
        body_part: 1001,
        hair_part: 0,
        guild: String::new(),
    }
}

/// `hero()` as on the wire.
const HERO: &[u8] = &[
    0x00, // slot = 0
    0x01, 0x00, 0x00, 0x00, // id = 1
    0x04, 0x00, b'H', b'e', b'r', b'o', // name = "Hero"
    0x00, // class = 0
    0x0C, // level = 12
    0x5A, 0x00, 0x00, 0x00, // playtime = 90
    0x06, // st = 6
    0x04, // ht = 4
    0x03, // dx = 3
    0x03, // iq = 3
    0xE9, 0x03, // body_part = 1001
    0x00, 0x00, // hair_part = 0
    0x00, 0x00, // guild = ""
];

// ---------------------------------------------------------------------------
// Client → server

#[test]
fn hello() {
    let wire = concat(&[
        &[0x11],       // frame length = 17
        &[0x01],       // id = 1
        &[0x03, 0x00], // protocol = 3
        &[0x0C, 0x00], // build: 12 bytes
        b"openm2 0.1.0",
    ]);
    let hello = Hello {
        protocol: 3,
        build: "openm2 0.1.0".into(),
    };
    check_c2s(ClientMessage::Hello(hello), &wire);
}

#[test]
fn pong() {
    check_c2s(ClientMessage::Pong(Pong), &[0x01, 0x02]);
}

#[test]
fn ticket_login() {
    let wire = concat(&[
        &[0x2A],       // frame length = 42
        &[0x04],       // id = 4
        &[0x05, 0x00], // username: 5 bytes
        b"admin",      //
        &[0x20, 0x00], // ticket: 32 bytes
        &[0xAA; 32],   //
    ]);
    let login = TicketLogin {
        username: "admin".into(),
        ticket: vec![0xAA; 32],
    };
    check_c2s(ClientMessage::TicketLogin(login), &wire);
}

#[test]
fn start_sign_in() {
    check_c2s(ClientMessage::StartSignIn(StartSignIn), &[0x01, 0x1B]);
}

#[test]
fn provider_login() {
    let wire = concat(&[
        &[0x5E],       // frame length = 94
        &[0x1C],       // id = 28
        &[0x06, 0x00], // provider: 6 bytes
        b"google",     //
        &[0x05, 0x00], // code: 5 bytes
        b"4/0Ab",      //
        &[0x2B, 0x00], // verifier: 43 bytes
        b"dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk",
        &[0x1F, 0x00], // redirect: 31 bytes
        b"http://127.0.0.1:52817/callback",
    ]);
    let login = ProviderLogin {
        provider: "google".into(),
        code: "4/0Ab".into(),
        verifier: "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk".into(),
        redirect: "http://127.0.0.1:52817/callback".into(),
    };
    check_c2s(ClientMessage::ProviderLogin(login), &wire);
}

#[test]
fn create_character_with_and_without_an_empire() {
    let wire = concat(&[
        &[0x0B],       // frame length = 11
        &[0x05],       // id = 5
        &[0x01],       // slot = 1
        &[0x04, 0x00], // name: 4 bytes
        b"Hero",       //
        &[0x03],       // class = 3
        &[0x01, 0x02], // empire = Some(2)
    ]);
    let create = CreateCharacter {
        slot: 1,
        name: "Hero".into(),
        class: 3,
        empire: Some(2),
    };
    check_c2s(ClientMessage::CreateCharacter(create.clone()), &wire);
    let wire = concat(&[
        &[0x0A],       // frame length = 10
        &[0x05],       // id = 5
        &[0x01],       // slot = 1
        &[0x04, 0x00], // name: 4 bytes
        b"Hero",       //
        &[0x03],       // class = 3
        &[0x00],       // empire = None
    ]);
    let create = CreateCharacter {
        empire: None,
        ..create
    };
    check_c2s(ClientMessage::CreateCharacter(create), &wire);
}

#[test]
fn select_character() {
    let wire = [
        0x02, // frame length = 2
        0x06, // id = 6
        0x02, // slot = 2
    ];
    check_c2s(
        ClientMessage::SelectCharacter(SelectCharacter { slot: 2 }),
        &wire,
    );
}

#[test]
fn enter_game() {
    check_c2s(ClientMessage::EnterGame(EnterGame), &[0x01, 0x07]);
}

#[test]
fn send_chat() {
    let wire = concat(&[
        &[0x09],       // frame length = 9
        &[0x08],       // id = 8
        &[0x00],       // kind = 0 (talk)
        &[0x05, 0x00], // text: 5 bytes
        b"hello",
    ]);
    let chat = SendChat {
        kind: 0,
        text: "hello".into(),
    };
    check_c2s(ClientMessage::SendChat(chat), &wire);
}

#[test]
fn character_move() {
    let wire = [
        0x11, // frame length = 17
        0x09, // id = 9
        0x01, // kind = 1 (move)
        0x00, // argument = 0
        0x5A, 0x00, // angle = 90
        0xE8, 0x03, 0x00, 0x00, // x = 1000
        0x30, 0xF8, 0xFF, 0xFF, // y = -2000
        0x40, 0xE2, 0x01, 0x00, // time = 123456
    ];
    let moved = CharacterMove {
        kind: 1,
        argument: 0,
        angle: 90,
        x: 1000,
        y: -2000,
        time: 123_456,
    };
    check_c2s(ClientMessage::CharacterMove(moved), &wire);
}

#[test]
fn use_item() {
    let wire = [
        0x04, // frame length = 4
        0x0A, // id = 10
        0x01, // window = 1
        0x5B, 0x00, // slot = 91
    ];
    check_c2s(
        ClientMessage::UseItem(UseItem {
            window: 1,
            slot: 91,
        }),
        &wire,
    );
}

#[test]
fn move_item() {
    let wire = [
        0x08, // frame length = 8
        0x0B, // id = 11
        0x01, // from_window = 1
        0x05, 0x00, // from_slot = 5
        0x01, // to_window = 1
        0x0A, 0x00, // to_slot = 10
        0x03, // count = 3
    ];
    let moved = MoveItem {
        from_window: 1,
        from_slot: 5,
        to_window: 1,
        to_slot: 10,
        count: 3,
    };
    check_c2s(ClientMessage::MoveItem(moved), &wire);
}

#[test]
fn set_quick_slot() {
    let wire = [
        0x04, // frame length = 4
        0x0C, // id = 12
        0x02, // index = 2
        0x01, // kind = 1 (an item)
        0x05, // position = 5
    ];
    let set = SetQuickSlot {
        index: 2,
        kind: 1,
        position: 5,
    };
    check_c2s(ClientMessage::SetQuickSlot(set), &wire);
}

#[test]
fn clear_quick_slot() {
    let wire = [
        0x02, // frame length = 2
        0x0D, // id = 13
        0x02, // index = 2
    ];
    check_c2s(
        ClientMessage::ClearQuickSlot(ClearQuickSlot { index: 2 }),
        &wire,
    );
}

#[test]
fn swap_quick_slots() {
    let wire = [
        0x03, // frame length = 3
        0x0E, // id = 14
        0x01, // first = 1
        0x06, // second = 6
    ];
    let swap = SwapQuickSlots {
        first: 1,
        second: 6,
    };
    check_c2s(ClientMessage::SwapQuickSlots(swap), &wire);
}

#[test]
fn set_target() {
    let wire = [
        0x05, // frame length = 5
        0x0F, // id = 15
        0x04, 0x03, 0x02, 0x01, // vid = 0x01020304
    ];
    check_c2s(
        ClientMessage::SetTarget(SetTarget { vid: 0x0102_0304 }),
        &wire,
    );
}

#[test]
fn hit() {
    let wire = [
        0x06, // frame length = 6
        0x10, // id = 16
        0x07, 0x00, 0x00, 0x00, // vid = 7
        0x03, // skill = 3
    ];
    check_c2s(ClientMessage::Hit(Hit { vid: 7, skill: 3 }), &wire);
}

#[test]
fn pick_up() {
    let wire = [
        0x05, // frame length = 5
        0x11, // id = 17
        0x09, 0x00, 0x00, 0x00, // vid = 9
    ];
    check_c2s(ClientMessage::PickUp(PickUp { vid: 9 }), &wire);
}

#[test]
fn drop_item() {
    let wire = [
        0x05, // frame length = 5
        0x12, // id = 18
        0x01, // window = 1
        0x04, 0x00, // slot = 4
        0x02, // count = 2
    ];
    let drop = DropItem {
        window: 1,
        slot: 4,
        count: 2,
    };
    check_c2s(ClientMessage::DropItem(drop), &wire);
}

#[test]
fn drop_gold() {
    let wire = [
        0x05, // frame length = 5
        0x13, // id = 19
        0xF4, 0x01, 0x00, 0x00, // amount = 500
    ];
    check_c2s(ClientMessage::DropGold(DropGold { amount: 500 }), &wire);
}

#[test]
fn talk() {
    let wire = [
        0x05, // frame length = 5
        0x14, // id = 20
        0x03, 0x00, 0x00, 0x00, // vid = 3
    ];
    check_c2s(ClientMessage::Talk(Talk { vid: 3 }), &wire);
}

#[test]
fn shop_buy() {
    let wire = [
        0x02, // frame length = 2
        0x15, // id = 21
        0x06, // position = 6
    ];
    check_c2s(ClientMessage::ShopBuy(ShopBuy { position: 6 }), &wire);
}

#[test]
fn shop_sell() {
    let wire = [
        0x04, // frame length = 4
        0x16, // id = 22
        0x0C, 0x00, // slot = 12
        0x0A, // count = 10
    ];
    check_c2s(
        ClientMessage::ShopSell(ShopSell {
            slot: 12,
            count: 10,
        }),
        &wire,
    );
}

#[test]
fn shop_close() {
    check_c2s(ClientMessage::ShopClose(ShopClose), &[0x01, 0x17]);
}

#[test]
fn use_skill_with_and_without_a_target() {
    let wire = [
        0x07, // frame length = 7
        0x18, // id = 24
        0x01, // skill = 1
        0x01, 0x05, 0x00, 0x00, 0x00, // target = Some(5)
    ];
    let skill = UseSkill {
        skill: 1,
        target: Some(5),
    };
    check_c2s(ClientMessage::UseSkill(skill), &wire);
    let wire = [
        0x03, // frame length = 3
        0x18, // id = 24
        0x01, // skill = 1
        0x00, // target = None
    ];
    let skill = UseSkill {
        skill: 1,
        target: None,
    };
    check_c2s(ClientMessage::UseSkill(skill), &wire);
}

#[test]
fn raise_skill() {
    let wire = [
        0x02, // frame length = 2
        0x19, // id = 25
        0x03, // skill = 3
    ];
    check_c2s(ClientMessage::RaiseSkill(RaiseSkill { skill: 3 }), &wire);
}

#[test]
fn restart() {
    let wire = [
        0x02, // frame length = 2
        0x1A, // id = 26
        0x01, // town = true
    ];
    check_c2s(ClientMessage::Restart(Restart { town: true }), &wire);
}

#[test]
fn leave_game() {
    let wire = [
        0x01, // frame length = 1
        0x1D, // id = 29
    ];
    check_c2s(ClientMessage::LeaveGame(LeaveGame), &wire);
}

#[test]
fn raise_status() {
    let wire = [
        0x02, // frame length = 2
        0x1E, // id = 30
        0x0D, // point = 13 (vitality)
    ];
    check_c2s(ClientMessage::RaiseStatus(RaiseStatus { point: 13 }), &wire);
}

#[test]
fn learn_skill_group() {
    let wire = [
        0x05, // frame length = 5
        0x1F, // id = 31
        0x2A, 0x00, 0x00, 0x00, // vid = 42
    ];
    check_c2s(
        ClientMessage::LearnSkillGroup(LearnSkillGroup { vid: 42 }),
        &wire,
    );
}

#[test]
fn use_item_on_skill() {
    let wire = [
        0x04, // frame length = 4
        0x20, // id = 32
        0x2D, 0x00, // slot = 45
        0x1F, // skill = 31
    ];
    check_c2s(
        ClientMessage::UseItemOnSkill(UseItemOnSkill {
            slot: 45,
            skill: 31,
        }),
        &wire,
    );
}

#[test]
fn send_whisper() {
    let wire = concat(&[
        &[0x0A],       // frame length = 10
        &[0x21],       // id = 33
        &[0x03, 0x00], // name: 3 bytes
        b"Bob",
        &[0x02, 0x00], // text: 2 bytes
        b"hi",
    ]);
    let whisper = SendWhisper {
        name: "Bob".into(),
        text: "hi".into(),
    };
    check_c2s(ClientMessage::SendWhisper(whisper), &wire);
}

#[test]
fn ask_friend() {
    let wire = concat(&[
        &[0x06],       // frame length = 6
        &[0x22],       // id = 34
        &[0x03, 0x00], // name: 3 bytes
        b"Bob",
    ]);
    let ask = AskFriend { name: "Bob".into() };
    check_c2s(ClientMessage::AskFriend(ask), &wire);
}

#[test]
fn answer_friend() {
    let wire = concat(&[
        &[0x07],       // frame length = 7
        &[0x23],       // id = 35
        &[0x03, 0x00], // name: 3 bytes
        b"Ann",
        &[0x01], // accept = true
    ]);
    let answer = AnswerFriend {
        name: "Ann".into(),
        accept: true,
    };
    check_c2s(ClientMessage::AnswerFriend(answer), &wire);
}

#[test]
fn remove_friend() {
    let wire = concat(&[
        &[0x06],       // frame length = 6
        &[0x24],       // id = 36
        &[0x03, 0x00], // name: 3 bytes
        b"Bob",
    ]);
    let remove = RemoveFriend { name: "Bob".into() };
    check_c2s(ClientMessage::RemoveFriend(remove), &wire);
}

#[test]
fn ride_mount() {
    let wire = concat(&[
        &[0x02], // frame length = 2
        &[0x25], // id = 37
        &[0x01], // ride = true
    ]);
    check_c2s(ClientMessage::RideMount(RideMount { ride: true }), &wire);
}

#[test]
fn train_mount() {
    let wire = concat(&[
        &[0x05],                   // frame length = 5
        &[0x26],                   // id = 38
        &[0xE9, 0x03, 0x00, 0x00], // vid = 1001
    ]);
    check_c2s(ClientMessage::TrainMount(TrainMount { vid: 1001 }), &wire);
}

#[test]
fn give_item() {
    let wire = concat(&[
        &[0x07],                   // frame length = 7
        &[0x27],                   // id = 39
        &[0xE9, 0x03, 0x00, 0x00], // vid = 1001
        &[0x0C, 0x00],             // slot = 12
    ]);
    check_c2s(
        ClientMessage::GiveItem(GiveItem {
            vid: 1001,
            slot: 12,
        }),
        &wire,
    );
}

#[test]
fn use_item_on_item() {
    let wire = [
        0x05, // frame length = 5
        0x28, // id = 40
        0x03, 0x00, // slot = 3
        0x0C, 0x00, // onto = 12
    ];
    check_c2s(
        ClientMessage::UseItemOnItem(UseItemOnItem { slot: 3, onto: 12 }),
        &wire,
    );
}

#[test]
fn upgrade() {
    // At a smith.
    let wire = concat(&[
        &[0x0D],                         // frame length = 13
        &[0x29],                         // id = 41
        &[0x01, 0xE9, 0x03, 0x00, 0x00], // smith = Some(1001)
        &[0x00],                         // scroll = None
        &[0x0C, 0x00],                   // slot = 12
        &[0xFD, 0x08, 0x00, 0x00],       // vnum = 2301
    ]);
    let upgrade = Upgrade {
        smith: Some(1001),
        scroll: None,
        slot: 12,
        vnum: 2301,
    };
    check_c2s(ClientMessage::Upgrade(upgrade), &wire);
    // With a scroll.
    let wire = concat(&[
        &[0x0B],                   // frame length = 11
        &[0x29],                   // id = 41
        &[0x00],                   // smith = None
        &[0x01, 0x03, 0x00],       // scroll = Some(3)
        &[0x0C, 0x00],             // slot = 12
        &[0xFD, 0x08, 0x00, 0x00], // vnum = 2301
    ]);
    let upgrade = Upgrade {
        smith: None,
        scroll: Some(3),
        slot: 12,
        vnum: 2301,
    };
    check_c2s(ClientMessage::Upgrade(upgrade), &wire);
}

// ---------------------------------------------------------------------------
// Server → client

#[test]
fn welcome() {
    let wire = concat(&[
        &[0x18],       // frame length = 24
        &[0x01],       // id = 1
        &[0x01, 0x00], // protocol = 1
        &[0x13, 0x00], // server: 19 bytes
        b"openm2-server 0.1.0",
    ]);
    let welcome = Welcome {
        protocol: 1,
        server: "openm2-server 0.1.0".into(),
    };
    check_s2c(ServerMessage::Welcome(welcome), &wire);
}

#[test]
fn refused() {
    let wire = concat(&[
        &[0x16],       // frame length = 22
        &[0x02],       // id = 2
        &[0x01],       // reason = 1 (another version)
        &[0x01, 0x00], // oldest = 1
        &[0x02, 0x00], // newest = 2
        &[0x0E, 0x00], // text: 14 bytes
        b"Update openm2.",
    ]);
    let refused = Refused {
        reason: 1,
        oldest: 1,
        newest: 2,
        text: "Update openm2.".into(),
    };
    check_s2c(ServerMessage::Refused(refused), &wire);
}

#[test]
fn ping() {
    check_s2c(ServerMessage::Ping(Ping), &[0x01, 0x03]);
}

#[test]
fn phase() {
    let wire = [
        0x02, // frame length = 2
        0x04, // id = 4
        0x02, // phase = 2 (loading)
    ];
    check_s2c(ServerMessage::Phase(Phase { phase: 2 }), &wire);
}

#[test]
fn login_refused() {
    let wire = [
        0x02, // frame length = 2
        0x06, // id = 6
        0x06, // reason = 6 (the provider did not confirm the sign-in)
    ];
    check_s2c(
        ServerMessage::LoginRefused(LoginRefused { reason: 6 }),
        &wire,
    );
}

#[test]
fn empire() {
    let wire = [
        0x02, // frame length = 2
        0x07, // id = 7
        0x02, // empire = 2
    ];
    check_s2c(ServerMessage::Empire(Empire { empire: 2 }), &wire);
}

#[test]
fn character_list() {
    let wire = concat(&[
        &[0x1E],       // frame length = 30
        &[0x08],       // id = 8
        &[0x01, 0x00], // characters: 1
        HERO,          // the character
    ]);
    let list = CharacterList {
        characters: vec![hero()],
    };
    check_s2c(ServerMessage::CharacterList(list), &wire);
}

#[test]
fn character_created() {
    let wire = concat(&[
        &[0x1B], // frame length = 27
        &[0x09], // id = 9
        &[
            0x01, // slot = 1
            0x02, 0x00, 0x00, 0x00, // id = 2
            0x03, 0x00, b'A', b'n', b'n', // name = "Ann"
            0x01, // class = 1
            0x01, // level = 1
            0x00, 0x00, 0x00, 0x00, // playtime = 0
            0x04, // st = 4
            0x05, // ht = 5
            0x06, // dx = 6
            0x03, // iq = 3
            0x00, 0x00, // body_part = 0
            0x00, 0x00, // hair_part = 0
            0x00, 0x00, // guild = ""
        ],
    ]);
    let character = Character {
        slot: 1,
        id: 2,
        name: "Ann".into(),
        class: 1,
        level: 1,
        playtime: 0,
        st: 4,
        ht: 5,
        dx: 6,
        iq: 3,
        body_part: 0,
        hair_part: 0,
        guild: String::new(),
    };
    check_s2c(
        ServerMessage::CharacterCreated(CharacterCreated { character }),
        &wire,
    );
}

#[test]
fn character_create_failed() {
    let wire = [
        0x02, // frame length = 2
        0x0A, // id = 10
        0x00, // reason = 0 (the name is taken)
    ];
    check_s2c(
        ServerMessage::CharacterCreateFailed(CharacterCreateFailed { reason: 0 }),
        &wire,
    );
}

#[test]
fn main_character() {
    let wire = concat(&[
        &[0x1E],                   // frame length = 30
        &[0x0B],                   // id = 11
        &[0x10, 0x00, 0x00, 0x00], // vid = 16
        &[0x00],                   // class = 0
        &[0x04, 0x00],             // name: 4 bytes
        b"Hero",                   //
        &[0x18, 0x73, 0x01, 0x00], // x = 95000
        &[0x48, 0xEE, 0x00, 0x00], // y = 61000
        &[0x00, 0x00, 0x00, 0x00], // z = 0
        &[0x01],                   // empire = 1
        &[0x00],                   // skill_group = 0
        &[0xC2, 0x01, 0x00, 0x00], // run_speed = 450
    ]);
    let main = MainCharacter {
        vid: 16,
        class: 0,
        name: "Hero".into(),
        x: 95_000,
        y: 61_000,
        z: 0,
        empire: 1,
        skill_group: 0,
        run_speed: 450,
    };
    check_s2c(ServerMessage::MainCharacter(main), &wire);
}

#[test]
fn game_time() {
    let wire = [
        0x05, // frame length = 5
        0x0C, // id = 12
        0x40, 0x42, 0x0F, 0x00, // time = 1000000
    ];
    check_s2c(ServerMessage::GameTime(GameTime { time: 1_000_000 }), &wire);
}

#[test]
fn channel() {
    let wire = [
        0x02, // frame length = 2
        0x0D, // id = 13
        0x01, // channel = 1
    ];
    check_s2c(ServerMessage::Channel(Channel { channel: 1 }), &wire);
}

#[test]
fn chat() {
    let wire = concat(&[
        &[0x11],                   // frame length = 17
        &[0x0E],                   // id = 14
        &[0x00],                   // kind = 0 (talk)
        &[0x10, 0x00, 0x00, 0x00], // vid = 16
        &[0x01],                   // empire = 1
        &[0x08, 0x00],             // text: 8 bytes
        b"Hero: hi",
    ]);
    let chat = Chat {
        kind: 0,
        vid: 16,
        empire: 1,
        text: "Hero: hi".into(),
    };
    check_s2c(ServerMessage::Chat(chat), &wire);
}

#[test]
fn entity_spawn() {
    let wire = [
        0x21, // frame length = 33
        0x0F, // id = 15
        0x20, 0x00, 0x00, 0x00, // vid = 32
        0xB4, 0x00, // angle = 180
        0xE8, 0x03, 0x00, 0x00, // x = 1000
        0xD0, 0x07, 0x00, 0x00, // y = 2000
        0x00, 0x00, 0x00, 0x00, // z = 0
        0x00, // kind = 0 (monster)
        0xE9, 0x03, // race = 1001
        0x64, // move_speed = 100
        0x64, // attack_speed = 100
        0x00, // state = 0
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // affects = 0
    ];
    let spawn = EntitySpawn {
        vid: 32,
        angle: 180,
        x: 1000,
        y: 2000,
        z: 0,
        kind: 0,
        race: 1001,
        move_speed: 100,
        attack_speed: 100,
        state: 0,
        affects: 0,
    };
    check_s2c(ServerMessage::EntitySpawn(spawn), &wire);
}

#[test]
fn entity_remove() {
    let wire = [
        0x05, // frame length = 5
        0x10, // id = 16
        0x20, 0x00, 0x00, 0x00, // vid = 32
    ];
    check_s2c(ServerMessage::EntityRemove(EntityRemove { vid: 32 }), &wire);
}

#[test]
fn entity_move() {
    let wire = [
        0x15, // frame length = 21
        0x11, // id = 17
        0x20, 0x00, 0x00, 0x00, // vid = 32
        0x01, // kind = 1 (move)
        0x00, // argument = 0
        0x0E, 0x01, // angle = 270
        0xDC, 0x05, 0x00, 0x00, // x = 1500
        0xD0, 0x07, 0x00, 0x00, // y = 2000
        0xB0, 0x04, 0x00, 0x00, // duration = 1200
    ];
    let moved = EntityMove {
        vid: 32,
        kind: 1,
        argument: 0,
        angle: 270,
        x: 1500,
        y: 2000,
        duration: 1200,
    };
    check_s2c(ServerMessage::EntityMove(moved), &wire);
}

#[test]
fn entity_died() {
    let wire = [
        0x05, // frame length = 5
        0x12, // id = 18
        0x20, 0x00, 0x00, 0x00, // vid = 32
    ];
    check_s2c(ServerMessage::EntityDied(EntityDied { vid: 32 }), &wire);
}

#[test]
fn entity_update() {
    let wire = [
        0x1C, // frame length = 28
        0x13, // id = 19
        0x10, 0x00, 0x00, 0x00, // vid = 16
        0xE9, 0x03, 0xD1, 0x07, 0x00, 0x00, 0x01, 0x00, // parts = [1001, 2001, 0, 1]
        0x6E, // move_speed = 110
        0x64, // attack_speed = 100
        0x00, // state = 0
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // affects = 0
        0x00, 0x00, 0x00, 0x00, // guild = 0
    ];
    let update = EntityUpdate {
        vid: 16,
        parts: [1001, 2001, 0, 1],
        move_speed: 110,
        attack_speed: 100,
        state: 0,
        affects: 0,
        guild: 0,
    };
    check_s2c(ServerMessage::EntityUpdate(update), &wire);
}

#[test]
fn entity_info() {
    let wire = concat(&[
        &[0x19],                                           // frame length = 25
        &[0x14],                                           // id = 20
        &[0x10, 0x00, 0x00, 0x00],                         // vid = 16
        &[0x04, 0x00],                                     // name: 4 bytes
        b"Hero",                                           //
        &[0xE9, 0x03, 0xD1, 0x07, 0x00, 0x00, 0x01, 0x00], // parts = [1001, 2001, 0, 1]
        &[0x01],                                           // empire = 1
        &[0x00, 0x00, 0x00, 0x00],                         // guild = 0
        &[0x0C],                                           // level = 12
    ]);
    let info = EntityInfo {
        vid: 16,
        name: "Hero".into(),
        parts: [1001, 2001, 0, 1],
        empire: 1,
        guild: 0,
        level: 12,
    };
    check_s2c(ServerMessage::EntityInfo(info), &wire);
}

#[test]
fn target_health() {
    let wire = [
        0x06, // frame length = 6
        0x15, // id = 21
        0x20, 0x00, 0x00, 0x00, // vid = 32
        0x4B, // percent = 75
    ];
    let health = TargetHealth {
        vid: 32,
        percent: 75,
    };
    check_s2c(ServerMessage::TargetHealth(health), &wire);
}

#[test]
fn damage() {
    let wire = [
        0x0A, // frame length = 10
        0x16, // id = 22
        0x20, 0x00, 0x00, 0x00, // vid = 32
        0x01, // flags = 1 (normal)
        0x2A, 0x00, 0x00, 0x00, // amount = 42
    ];
    let damage = Damage {
        vid: 32,
        flags: 1,
        amount: 42,
    };
    check_s2c(ServerMessage::Damage(damage), &wire);
}

#[test]
fn points() {
    let wire = [
        0x12, // frame length = 18
        0x17, // id = 23
        0x03, 0x00, // points: 3
        0x01, 0x0C, 0x00, 0x00, 0x00, // level = 12
        0x05, 0xC2, 0x01, 0x00, 0x00, // health = 450
        0x0B, 0x64, 0x00, 0x00, 0x00, // gold = 100
    ];
    let points = Points {
        points: vec![
            PointValue {
                point: 1,
                value: 12,
            },
            PointValue {
                point: 5,
                value: 450,
            },
            PointValue {
                point: 11,
                value: 100,
            },
        ],
    };
    check_s2c(ServerMessage::Points(points), &wire);
}

#[test]
fn points_changed() {
    // A bite: health only, 9 bytes on the wire.
    let wire = [
        0x08, // frame length = 8
        0x18, // id = 24
        0x01, 0x00, // points: 1
        0x05, 0xAE, 0x01, 0x00, 0x00, // health = 430
    ];
    let points = PointsChanged {
        points: vec![PointValue {
            point: 5,
            value: 430,
        }],
    };
    check_s2c(ServerMessage::PointsChanged(points), &wire);
}

#[test]
fn item_set() {
    let wire = [
        0x22, // frame length = 34
        0x19, // id = 25
        0x01, // window = 1
        0x00, 0x00, // slot = 0
        0xE9, 0x03, 0x00, 0x00, // vnum = 1001
        0x01, // count = 1
        0x00, 0x00, 0x00, 0x00, // flags = 0
        0x00, 0x00, 0x00, 0x00, // anti_flags = 0
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, // sockets = [0; 3]
        0x01, 0x00, // bonuses: 1
        0x01, 0x32, 0x00, // kind 1, value 50
    ];
    let item = ItemSet {
        window: 1,
        slot: 0,
        vnum: 1001,
        count: 1,
        flags: 0,
        anti_flags: 0,
        sockets: [0; 3],
        bonuses: vec![ItemBonus { kind: 1, value: 50 }],
    };
    check_s2c(ServerMessage::ItemSet(item), &wire);
}

#[test]
fn ground_item_add() {
    let wire = [
        0x11, // frame length = 17
        0x1A, // id = 26
        0x30, 0x00, 0x00, 0x00, // vid = 48
        0x01, 0x00, 0x00, 0x00, // vnum = 1 (gold)
        0xB0, 0x04, 0x00, 0x00, // x = 1200
        0x34, 0x08, 0x00, 0x00, // y = 2100
    ];
    let item = GroundItemAdd {
        vid: 48,
        vnum: 1,
        x: 1200,
        y: 2100,
    };
    check_s2c(ServerMessage::GroundItemAdd(item), &wire);
}

#[test]
fn ground_item_remove() {
    let wire = [
        0x05, // frame length = 5
        0x1B, // id = 27
        0x30, 0x00, 0x00, 0x00, // vid = 48
    ];
    check_s2c(
        ServerMessage::GroundItemRemove(GroundItemRemove { vid: 48 }),
        &wire,
    );
}

#[test]
fn item_ownership() {
    let wire = concat(&[
        &[0x0B],                   // frame length = 11
        &[0x1C],                   // id = 28
        &[0x30, 0x00, 0x00, 0x00], // vid = 48
        &[0x04, 0x00],             // owner: 4 bytes
        b"Hero",
    ]);
    let owner = ItemOwnership {
        vid: 48,
        owner: "Hero".into(),
    };
    check_s2c(ServerMessage::ItemOwnership(owner), &wire);
}

#[test]
fn shop_opened() {
    let wire = [
        0x1B, // frame length = 27
        0x1D, // id = 29
        0x03, 0x00, 0x00, 0x00, // vid = 3
        0x02, 0x00, // goods: 2
        0x00, // position = 0
        0xE9, 0x03, 0x00, 0x00, // vnum = 1001
        0x3C, 0x00, 0x00, 0x00, // price = 60
        0x01, // count = 1
        0x05, // position = 5
        0xEA, 0x03, 0x00, 0x00, // vnum = 1002
        0x28, 0x00, 0x00, 0x00, // price = 40
        0x0A, // count = 10
    ];
    let shop = ShopOpened {
        vid: 3,
        goods: vec![
            ShopGood {
                position: 0,
                vnum: 1001,
                price: 60,
                count: 1,
            },
            ShopGood {
                position: 5,
                vnum: 1002,
                price: 40,
                count: 10,
            },
        ],
    };
    check_s2c(ServerMessage::ShopOpened(shop), &wire);
}

#[test]
fn shop_refused() {
    let wire = [
        0x02, // frame length = 2
        0x1E, // id = 30
        0x02, // reason = 2 (no room)
    ];
    check_s2c(ServerMessage::ShopRefused(ShopRefused { reason: 2 }), &wire);
}

#[test]
fn quick_slot_set() {
    let wire = [
        0x04, // frame length = 4
        0x1F, // id = 31
        0x00, // index = 0
        0x02, // kind = 2 (a skill)
        0x01, // position = 1
    ];
    let set = QuickSlotSet {
        index: 0,
        kind: 2,
        position: 1,
    };
    check_s2c(ServerMessage::QuickSlotSet(set), &wire);
}

#[test]
fn quick_slot_cleared() {
    let wire = [
        0x02, // frame length = 2
        0x20, // id = 32
        0x00, // index = 0
    ];
    check_s2c(
        ServerMessage::QuickSlotCleared(QuickSlotCleared { index: 0 }),
        &wire,
    );
}

#[test]
fn quick_slots_swapped() {
    let wire = [
        0x03, // frame length = 3
        0x21, // id = 33
        0x00, // first = 0
        0x03, // second = 3
    ];
    let swapped = QuickSlotsSwapped {
        first: 0,
        second: 3,
    };
    check_s2c(ServerMessage::QuickSlotsSwapped(swapped), &wire);
}

#[test]
fn skills() {
    let wire = [
        0x0A, // frame length = 10
        0x22, // id = 34
        0x01, 0x00, // skills: 1
        0x01, // skill = 1
        0x14, // level = 20 (M1)
        0x01, // grade = 1 (master)
        0x00, 0x00, 0x00, 0x00, // next_read = 0
    ];
    let skills = Skills {
        skills: vec![SkillState {
            skill: 1,
            level: 20,
            grade: 1,
            next_read: 0,
        }],
    };
    check_s2c(ServerMessage::Skills(skills), &wire);
}

#[test]
fn skill_group() {
    let wire = [
        0x02, // frame length = 2
        0x23, // id = 35
        0x01, // group = 1
    ];
    check_s2c(ServerMessage::SkillGroup(SkillGroup { group: 1 }), &wire);
}

#[test]
fn sign_in_options() {
    let wire = concat(&[
        &[0x72],       // frame length = 114
        &[0x24],       // id = 36
        &[0x01, 0x00], // providers: 1
        &[0x06, 0x00], //   id: 6 bytes
        b"google",     //
        &[0x06, 0x00], //   name: 6 bytes
        b"Google",     //
        &[0x2C, 0x00], //   authorize: 44 bytes
        b"https://accounts.google.com/o/oauth2/v2/auth",
        &[0x0D, 0x00], //   client_id: 13 bytes
        b"openm2-client",
        &[0x20, 0x00], // nonce: 32 bytes
        b"0123456789abcdef0123456789abcdef",
    ]);
    let options = SignInOptions {
        providers: vec![Provider {
            id: "google".into(),
            name: "Google".into(),
            authorize: "https://accounts.google.com/o/oauth2/v2/auth".into(),
            client_id: "openm2-client".into(),
        }],
        nonce: "0123456789abcdef0123456789abcdef".into(),
    };
    check_s2c(ServerMessage::SignInOptions(options), &wire);
    // A server without providers.
    let none = SignInOptions {
        providers: Vec::new(),
        nonce: String::new(),
    };
    check_s2c(
        ServerMessage::SignInOptions(none),
        &[
            0x05, // frame length = 5
            0x24, // id = 36
            0x00, 0x00, // providers: 0
            0x00, 0x00, // nonce = ""
        ],
    );
}

#[test]
fn signed_in() {
    let wire = concat(&[
        &[0x39],       // frame length = 57
        &[0x25],       // id = 37
        &[0x14, 0x00], // account: 20 bytes
        b"google:2fh4kq7m3xw9a",
        &[0x20, 0x00], // ticket: 32 bytes
        &[0xAB; 32],   //
    ]);
    let signed_in = SignedIn {
        account: "google:2fh4kq7m3xw9a".into(),
        ticket: vec![0xAB; 32],
    };
    check_s2c(ServerMessage::SignedIn(signed_in), &wire);
}

#[test]
fn move_refused() {
    let wire = concat(&[
        &[0x09],                   // frame length = 9
        &[0x26],                   // id = 38
        &[0x18, 0x73, 0x01, 0x00], // x = 95000
        &[0xB8, 0x11, 0xFF, 0xFF], // y = -61000
    ]);
    let refused = MoveRefused {
        x: 95_000,
        y: -61_000,
    };
    check_s2c(ServerMessage::MoveRefused(refused), &wire);
}

#[test]
fn projectile() {
    let wire = concat(&[
        &[0x11],                   // frame length = 17
        &[0x27],                   // id = 39
        &[0x07, 0x00, 0x00, 0x00], // shooter = 7
        &[0x2A, 0x00, 0x00, 0x00], // target = 42
        &[0x18, 0x73, 0x01, 0x00], // x = 95000
        &[0xB8, 0x11, 0xFF, 0xFF], // y = -61000
    ]);
    let shot = Projectile {
        shooter: 7,
        target: 42,
        x: 95_000,
        y: -61_000,
    };
    check_s2c(ServerMessage::Projectile(shot), &wire);
}

#[test]
fn teacher_opened() {
    let wire = concat(&[
        &[0x08],                   // frame length = 8
        &[0x28],                   // id = 40
        &[0x07, 0x00, 0x00, 0x00], // vid = 7
        &[0x02],                   // job = 2 (sura)
        &[0x01],                   // group = 1
        &[0x05],                   // level = 5
    ]);
    let opened = TeacherOpened {
        vid: 7,
        job: 2,
        group: 1,
        level: 5,
    };
    check_s2c(ServerMessage::TeacherOpened(opened), &wire);
}

#[test]
fn choose_skill() {
    let wire = [
        0x07, // frame length = 7
        0x29, // id = 41
        0x03, 0x00, // slot = 3
        0x02, 0x00, // skills: 2
        0x01, // skill 1
        0x05, // skill 5
    ];
    let choose = ChooseSkill {
        slot: 3,
        skills: vec![1, 5],
    };
    check_s2c(ServerMessage::ChooseSkill(choose), &wire);
}

#[test]
fn whisper() {
    let wire = concat(&[
        &[0x0E],       // frame length = 14
        &[0x2A],       // id = 42
        &[0x03, 0x00], // name: 3 bytes
        b"Ann",
        &[0x00],       // mine = false: Ann whispered to us
        &[0x05, 0x00], // text: 5 bytes
        b"hello",
    ]);
    let whisper = Whisper {
        name: "Ann".into(),
        mine: false,
        text: "hello".into(),
    };
    check_s2c(ServerMessage::Whisper(whisper), &wire);
}

#[test]
fn whisper_refused() {
    let wire = concat(&[
        &[0x07],       // frame length = 7
        &[0x2B],       // id = 43
        &[0x03, 0x00], // name: 3 bytes
        b"zed",
        &[0x01], // reason = 1 (nobody of that name in the game)
    ]);
    let refused = WhisperRefused {
        name: "zed".into(),
        reason: 1,
    };
    check_s2c(ServerMessage::WhisperRefused(refused), &wire);
}

#[test]
fn friends() {
    let wire = concat(&[
        &[0x0F],       // frame length = 15
        &[0x2C],       // id = 44
        &[0x02, 0x00], // friends: 2
        &[0x03, 0x00], // name: 3 bytes
        b"Ann",
        &[0x01],       // online = true
        &[0x03, 0x00], // name: 3 bytes
        b"Bob",
        &[0x00], // online = false
    ]);
    let friends = Friends {
        friends: vec![
            Friend {
                name: "Ann".into(),
                online: true,
            },
            Friend {
                name: "Bob".into(),
                online: false,
            },
        ],
    };
    check_s2c(ServerMessage::Friends(friends), &wire);
}

#[test]
fn friend_changed() {
    let wire = concat(&[
        &[0x07],       // frame length = 7
        &[0x2D],       // id = 45
        &[0x03, 0x00], // name: 3 bytes
        b"Ann",
        &[0x02], // status = 2 (in the game)
    ]);
    let changed = FriendChanged {
        name: "Ann".into(),
        status: 2,
    };
    check_s2c(ServerMessage::FriendChanged(changed), &wire);
}

#[test]
fn friend_asked() {
    let wire = concat(&[
        &[0x06],       // frame length = 6
        &[0x2E],       // id = 46
        &[0x03, 0x00], // name: 3 bytes
        b"Ann",
    ]);
    let asked = FriendAsked { name: "Ann".into() };
    check_s2c(ServerMessage::FriendAsked(asked), &wire);
}

#[test]
fn friend_notice() {
    let wire = concat(&[
        &[0x07],       // frame length = 7
        &[0x2F],       // id = 47
        &[0x03, 0x00], // name: 3 bytes
        b"Ann",
        &[0x07], // notice = 7 (declined)
    ]);
    let notice = FriendNotice {
        name: "Ann".into(),
        notice: 7,
    };
    check_s2c(ServerMessage::FriendNotice(notice), &wire);
}

#[test]
fn entity_mount() {
    let wire = concat(&[
        &[0x07],                   // frame length = 7
        &[0x30],                   // id = 48
        &[0x10, 0x27, 0x00, 0x00], // vid = 10000
        &[0xB1, 0x04],             // look = 1201
    ]);
    let mount = EntityMount {
        vid: 10_000,
        look: 1201,
    };
    check_s2c(ServerMessage::EntityMount(mount), &wire);
}

#[test]
fn mount_state() {
    let wire = concat(&[
        &[0x13],                         // frame length = 19
        &[0x31],                         // id = 49
        &[0x02],                         // level = 2
        &[0x01],                         // grade = 1
        &[0xB1, 0x04],                   // look = 1201
        &[0x4B, 0x00],                   // stamina = 75
        &[0x78, 0x00],                   // most = 120
        &[0x18, 0x00],                   // ride_from = 24
        &[0x01],                         // fights = true
        &[0x00],                         // skills = false
        &[0x03],                         // skill_points = 3
        &[0x01, 0x11, 0x27, 0x00, 0x00], // summoned = Some(10001)
    ]);
    let state = MountState {
        level: 2,
        grade: 1,
        look: 1201,
        stamina: 75,
        most: 120,
        ride_from: 24,
        fights: true,
        skills: false,
        skill_points: 3,
        summoned: Some(10_001),
    };
    check_s2c(ServerMessage::MountState(state), &wire);
}

#[test]
fn run_speed() {
    let wire = concat(&[
        &[0x05],                   // frame length = 5
        &[0x32],                   // id = 50
        &[0x80, 0x02, 0x00, 0x00], // run_speed = 640
    ]);
    check_s2c(ServerMessage::RunSpeed(RunSpeed { run_speed: 640 }), &wire);
}

#[test]
fn stable_opened() {
    let wire = concat(&[
        &[0x1A],                   // frame length = 26
        &[0x33],                   // id = 51
        &[0xD1, 0x07, 0x00, 0x00], // vid = 2001
        &[0x01],                   // next = Some(
        &[0x03],                   //   level = 3
        &[0x02],                   //   grade = 2
        &[0xB2, 0x04],             //   look = 1202
        &[0xDC],                   //   move_speed = 220
        &[0x96, 0x00],             //   stamina = 150
        &[0x14],                   //   rider_level = 20
        &[0x88, 0x13, 0x00, 0x00], //   gold = 5000
        &[0x01, 0x00],             //   items: 1
        &[0x4E, 0x04, 0x00, 0x00], //     vnum = 1102
        &[0x02],                   //     count = 2
        &[0x01],                   //   skill_points = 1)
    ]);
    let opened = StableOpened {
        vid: 2001,
        next: Some(MountOffer {
            level: 3,
            grade: 2,
            look: 1202,
            move_speed: 220,
            stamina: 150,
            rider_level: 20,
            gold: 5000,
            items: vec![MountCost {
                vnum: 1102,
                count: 2,
            }],
            skill_points: 1,
        }),
    };
    check_s2c(ServerMessage::StableOpened(opened), &wire);
    // At the last level: nothing to train to.
    let wire = concat(&[
        &[0x06],                   // frame length = 6
        &[0x33],                   // id = 51
        &[0xD1, 0x07, 0x00, 0x00], // vid = 2001
        &[0x00],                   // next = None
    ]);
    let opened = StableOpened {
        vid: 2001,
        next: None,
    };
    check_s2c(ServerMessage::StableOpened(opened), &wire);
}

#[test]
fn upgrade_offer() {
    let wire = concat(&[
        &[0x1E],                         // frame length = 30
        &[0x35],                         // id = 53
        &[0x01, 0xE9, 0x03, 0x00, 0x00], // smith = Some(1001)
        &[0x00],                         // scroll = None
        &[0x0C, 0x00],                   // slot = 12
        &[0xFD, 0x08, 0x00, 0x00],       // vnum = 2301
        &[0xFE, 0x08, 0x00, 0x00],       // next = 2302
        &[0xB0, 0x04, 0x00, 0x00],       // gold = 1200
        &[0x01, 0x00],                   // items: 1
        &[0x07, 0x09, 0x00, 0x00],       //   vnum = 2311
        &[0x02],                         //   count = 2
        &[0x32],                         // chance = 50
        &[0x00],                         // on_fail = 0 (the item is gone)
    ]);
    let offer = UpgradeOffer {
        smith: Some(1001),
        scroll: None,
        slot: 12,
        vnum: 2301,
        next: 2302,
        gold: 1200,
        items: vec![UpgradeCost {
            vnum: 2311,
            count: 2,
        }],
        chance: 50,
        on_fail: 0,
    };
    check_s2c(ServerMessage::UpgradeOffer(offer), &wire);
}

#[test]
fn flight() {
    let wire = [
        0x0A, // frame length = 10
        0x36, // id = 54
        0x00, // kind = 0 (experience)
        0x07, 0x00, 0x00, 0x00, // from = 7
        0x2A, 0x00, 0x00, 0x00, // to = 42
    ];
    let flight = Flight {
        kind: 0,
        from: 7,
        to: 42,
    };
    check_s2c(ServerMessage::Flight(flight), &wire);
}

#[test]
fn entity_effect() {
    let wire = [
        0x06, // frame length = 6
        0x37, // id = 55
        0x02, // kind = 2 (a level gained)
        0x2A, 0x00, 0x00, 0x00, // vid = 42
    ];
    let effect = EntityEffect { kind: 2, vid: 42 };
    check_s2c(ServerMessage::EntityEffect(effect), &wire);
}

#[test]
fn entity_skills() {
    let wire = [
        0x09, // frame length = 9
        0x38, // id = 56
        0x2A, 0x00, 0x00, 0x00, // vid = 42
        0x02, 0x00, // skills: 2
        0x04, // skill 4
        0x13, // skill 19
    ];
    let skills = EntitySkills {
        vid: 42,
        skills: vec![4, 19],
    };
    check_s2c(ServerMessage::EntitySkills(skills), &wire);
}

#[test]
fn cast() {
    let wire = [
        0x03, // frame length = 3
        0x2A, // id = 42
        0x5A, 0x00, // angle = 90 (east)
    ];
    check_c2s(ClientMessage::Cast(Cast { angle: 90 }), &wire);
}

#[test]
fn reel_in() {
    let wire = [
        0x01, // frame length = 1
        0x2B, // id = 43
    ];
    check_c2s(ClientMessage::ReelIn(ReelIn), &wire);
}

#[test]
fn fishing() {
    let wire = [
        0x0E, // frame length = 14
        0x39, // id = 57
        0x2A, 0x00, 0x00, 0x00, // vid = 42
        0x02, // step = 2 (a bite)
        0x10, 0x27, 0x00, 0x00, // x = 10000
        0xF0, 0xD8, 0xFF, 0xFF, // y = -10000
    ];
    let fishing = Fishing {
        vid: 42,
        step: 2,
        x: 10_000,
        y: -10_000,
    };
    check_s2c(ServerMessage::Fishing(fishing), &wire);
}

#[test]
fn item_boosts() {
    let wire = [
        0x10, // frame length = 16
        0x3A, // id = 58
        0x01, 0x00, // boosts: 1
        0xE6, 0x6C, 0x00, 0x00, // item = 27878
        0x0D, // point = 13 (strength)
        0x0A, 0x00, 0x00, 0x00, // amount = 10
        0x58, 0x02, 0x00, 0x00, // seconds = 600
    ];
    let boosts = ItemBoosts {
        boosts: vec![ItemBoost {
            item: 27_878,
            point: 13,
            amount: 10,
            seconds: 600,
        }],
    };
    check_s2c(ServerMessage::ItemBoosts(boosts), &wire);
}

#[test]
fn affects() {
    let wire = [
        0x17, // frame length = 23
        0x34, // id = 52
        0x02, 0x00, // affects: 2
        0x03, // skill 3
        0x01, // point 1 (attack speed)
        0x19, 0x00, 0x00, 0x00, // amount = 25
        0x5A, 0x00, 0x00, 0x00, // seconds = 90
        0x13, // skill 19
        0x02, // point 2 (movement speed)
        0xF6, 0xFF, 0xFF, 0xFF, // amount = -10
        0x78, 0x00, 0x00, 0x00, // seconds = 120
    ];
    let affects = Affects {
        affects: vec![
            Affect {
                skill: 3,
                point: 1,
                amount: 25,
                seconds: 90,
            },
            Affect {
                skill: 19,
                point: 2,
                amount: -10,
                seconds: 120,
            },
        ],
    };
    check_s2c(ServerMessage::Affects(affects), &wire);
    let wire = [0x03, 0x34, 0x00, 0x00]; // the last one ended
    check_s2c(ServerMessage::Affects(Affects { affects: vec![] }), &wire);
}

#[test]
fn a_heal_is_damage_with_flag_64() {
    let wire = [
        0x0A, // frame length = 10
        0x16, // id = 22
        0x07, 0x00, 0x00, 0x00, // vid = 7
        0x40, // flags = 64 (a heal)
        0x2C, 0x01, 0x00, 0x00, // amount = 300, given
    ];
    let heal = Damage {
        vid: 7,
        flags: 64,
        amount: 300,
    };
    check_s2c(ServerMessage::Damage(heal), &wire);
}

// ---------------------------------------------------------------------------
// Tables, refusals and frames

#[test]
fn ids_skip_only_the_retired_ones_and_the_hello_never_moves() {
    let ids = |table: &[m2_protocol::MessageSpec]| table.iter().map(|m| m.id).collect::<Vec<_>>();
    // c2s 3 and s2c 5 are reserved: no message has them.
    assert_eq!(
        ids(CLIENT_MESSAGES),
        (1..=43).filter(|id| *id != 3).collect::<Vec<_>>()
    );
    assert_eq!(
        ids(SERVER_MESSAGES),
        (1..=58).filter(|id| *id != 5).collect::<Vec<_>>()
    );
    assert_eq!(
        ClientMessage::decode(&[0x03, 0x00, 0x00, 0x00, 0x00]),
        Err(DecodeError::UnknownId { id: 3 })
    );
    // These three must keep their ids in every version.
    assert_eq!(Hello::ID, 1);
    assert_eq!(Welcome::ID, 1);
    assert_eq!(Refused::ID, 2);
    assert_eq!(
        spec_of(CLIENT_MESSAGES, 4).map(|m| m.secrets),
        Some(&["ticket"][..])
    );
    assert_eq!(
        spec_of(CLIENT_MESSAGES, 28).map(|m| m.secrets),
        Some(&["code", "verifier"][..])
    );
    assert_eq!(
        spec_of(SERVER_MESSAGES, 37).map(|m| m.secrets),
        Some(&["ticket"][..])
    );
}

#[test]
fn secrets_never_show_and_are_blanked() {
    let login = ClientMessage::TicketLogin(TicketLogin {
        username: "admin".into(),
        ticket: vec![0xBE; 32],
    });
    let shown = format!("{login:?}");
    assert!(
        shown.contains("admin") && shown.contains("<redacted>"),
        "{shown}"
    );
    assert!(!shown.contains("190"), "{shown}");
    let ClientMessage::TicketLogin(blank) = login.redacted() else {
        panic!("redacted() keeps the message");
    };
    assert_eq!(
        (blank.username.as_str(), blank.ticket.as_slice()),
        ("admin", &[][..])
    );
    let provider = ClientMessage::ProviderLogin(ProviderLogin {
        provider: "google".into(),
        code: "the-code".into(),
        verifier: "the-verifier".into(),
        redirect: "http://127.0.0.1:52817/callback".into(),
    });
    let shown = format!("{provider:?}");
    assert!(
        shown.contains("google") && shown.contains("52817"),
        "{shown}"
    );
    assert!(
        !shown.contains("the-code") && !shown.contains("the-verifier"),
        "{shown}"
    );
    let ClientMessage::ProviderLogin(blank) = provider.redacted() else {
        panic!("redacted() keeps the message");
    };
    assert_eq!((blank.code.as_str(), blank.verifier.as_str()), ("", ""));
    assert_eq!(blank.provider, "google");
    let signed_in = ServerMessage::SignedIn(SignedIn {
        account: "google:2fh4kq7m3xw9a".into(),
        ticket: vec![0xBE; 32],
    });
    assert!(!format!("{signed_in:?}").contains("190"));
    assert_eq!(
        signed_in.redacted(),
        ServerMessage::SignedIn(SignedIn {
            account: "google:2fh4kq7m3xw9a".into(),
            ticket: vec![],
        })
    );
    // Messages without secrets come back the same.
    let pong = ClientMessage::Pong(Pong);
    assert_eq!(pong.redacted(), pong);
}

#[test]
fn bad_messages_are_errors() {
    // An id nobody sends.
    assert_eq!(
        ServerMessage::decode(&[0x7F]),
        Err(DecodeError::UnknownId { id: 127 })
    );
    assert_eq!(
        ClientMessage::decode(&[0x80, 0x01]),
        Err(DecodeError::UnknownId { id: 128 })
    );
    // Another message's bytes.
    assert_eq!(
        TicketLogin::from_bytes(&[0x02]),
        Err(DecodeError::WrongId {
            expected: 4,
            found: 2
        })
    );
    // A name longer than its field: 65 bytes for string<64>.
    let mut long = vec![0x0B, 0x10, 0, 0, 0, 0x00, 0x41, 0x00];
    long.extend([b'a'; 65]);
    assert!(matches!(
        ServerMessage::decode(&long),
        Err(DecodeError::TooLong {
            len: 65,
            max: 64,
            ..
        })
    ));
    // Not UTF-8.
    assert!(matches!(
        ClientMessage::decode(&[0x08, 0x00, 0x01, 0x00, 0xFF]),
        Err(DecodeError::BadUtf8 { .. })
    ));
    // Five characters for a list of at most four.
    assert!(matches!(
        ServerMessage::decode(&[0x08, 0x05, 0x00]),
        Err(DecodeError::TooMany {
            count: 5,
            max: 4,
            ..
        })
    ));
    // An option's tag and a boolean that are not 0 or 1.
    assert_eq!(
        ClientMessage::decode(&[0x18, 0x01, 0x02]),
        Err(DecodeError::BadTag { at: 2, value: 2 })
    );
    assert_eq!(
        ClientMessage::decode(&[0x1A, 0x02]),
        Err(DecodeError::BadTag { at: 1, value: 2 })
    );
    // Cut short, and too long.
    assert!(matches!(
        ServerMessage::decode(&[0x16, 0x20, 0x00]),
        Err(DecodeError::UnexpectedEnd { .. })
    ));
    assert_eq!(
        ServerMessage::decode(&[0x03, 0x00]),
        Err(DecodeError::TrailingBytes { count: 1 })
    );
    // An id in more bytes than it needs.
    assert_eq!(
        ServerMessage::decode(&[0x83, 0x00]),
        Err(DecodeError::BadVarint { at: 0 })
    );
    // A string too long to send.
    let chat = ClientMessage::SendChat(SendChat {
        kind: 0,
        text: "a".repeat(513),
    });
    assert_eq!(
        chat.to_bytes(),
        Err(EncodeError::TooLong { len: 513, max: 512 })
    );
}

#[test]
fn frames_split_join_and_refuse() {
    let chat = ServerMessage::Chat(Chat {
        kind: 0,
        vid: 0,
        empire: 0,
        text: "x".repeat(200),
    });
    let long = frame(&chat.to_bytes().unwrap(), MAX).unwrap();
    // 209 bytes of message (id, kind, vid, empire, length, text): a
    // two-byte length.
    assert_eq!(&long[..3], &[0xD1, 0x01, 0x0E]);
    let ping = frame(&ServerMessage::Ping(Ping).to_bytes().unwrap(), MAX).unwrap();
    let stream = [ping.as_slice(), &long, &ping].concat();
    for cut in 1..stream.len() {
        let mut deframer = Deframer::new(MAX);
        let mut got = Vec::new();
        for part in [&stream[..cut], &stream[cut..]] {
            deframer.push(part);
            while let Some(message) = deframer.next_frame().unwrap() {
                got.push(ServerMessage::decode(&message).unwrap());
            }
        }
        assert_eq!(
            got,
            [
                ServerMessage::Ping(Ping),
                chat.clone(),
                ServerMessage::Ping(Ping)
            ],
            "cut at {cut}"
        );
        assert_eq!(deframer.buffered(), 0);
    }

    // Too large: refused from its length alone, and for good.
    let mut deframer = Deframer::new(100);
    deframer.push(&[0xD0, 0x01]);
    let error = WireError::TooLarge {
        offset: 0,
        len: 208,
        max: 100,
    };
    assert_eq!(deframer.next_frame(), Err(error.clone()));
    deframer.push(&[0x01, 0x03]);
    assert_eq!(deframer.next_frame(), Err(error));
    // Empty, and a length in too many bytes.
    let mut deframer = Deframer::new(MAX);
    deframer.push(&[0x01, 0x03, 0x00]);
    assert_eq!(deframer.next_frame(), Ok(Some(vec![0x03])));
    assert_eq!(deframer.next_frame(), Err(WireError::Empty { offset: 2 }));
    let mut deframer = Deframer::new(MAX);
    deframer.push(&[0x81, 0x00]);
    assert_eq!(
        deframer.next_frame(),
        Err(WireError::BadLength { offset: 0 })
    );
    // A length cut short waits for its next byte.
    let mut deframer = Deframer::new(MAX);
    deframer.push(&[0x81]);
    assert_eq!(deframer.next_frame(), Ok(None));
    // Too large to send.
    assert_eq!(
        frame(&[0; 101], 100),
        Err(WireError::MessageTooLarge { len: 101, max: 100 })
    );
}

#[test]
fn the_biggest_messages_fit_their_frame_limits() {
    // Every field at its maximum.
    let chat = ClientMessage::SendChat(SendChat {
        kind: 0,
        text: "a".repeat(512),
    })
    .to_bytes()
    .unwrap();
    assert_eq!(chat.len(), 516);
    let provider = ClientMessage::ProviderLogin(ProviderLogin {
        provider: "a".repeat(16),
        code: "a".repeat(4096),
        verifier: "a".repeat(128),
        redirect: "a".repeat(64),
    })
    .to_bytes()
    .unwrap();
    assert_eq!(provider.len(), 4_313);
    let whisper = ClientMessage::SendWhisper(SendWhisper {
        name: "a".repeat(24),
        text: "a".repeat(512),
    })
    .to_bytes()
    .unwrap();
    assert_eq!(whisper.len(), 541);
    assert!(chat.len() < provider.len());
    assert!(whisper.len() < provider.len());
    assert!(frame(&provider, MAX_CLIENT_FRAME).is_ok());

    let skill = |skill| SkillState {
        skill,
        level: 40,
        grade: 3,
        next_read: u32::MAX,
    };
    let skills = ServerMessage::Skills(Skills {
        skills: (0..=254).map(skill).collect(),
    })
    .to_bytes()
    .unwrap();
    assert_eq!(skills.len(), 1_788);
    let points = ServerMessage::Points(Points {
        points: (0..=254)
            .map(|point| PointValue {
                point,
                value: u32::MAX,
            })
            .collect(),
    })
    .to_bytes()
    .unwrap();
    let chat = ServerMessage::Chat(Chat {
        kind: 0,
        vid: 0,
        empire: 0,
        text: "a".repeat(1024),
    })
    .to_bytes()
    .unwrap();
    let list = ServerMessage::CharacterList(CharacterList {
        characters: (0..4)
            .map(|slot| Character {
                slot,
                name: "a".repeat(64),
                guild: "a".repeat(64),
                ..hero()
            })
            .collect(),
    })
    .to_bytes()
    .unwrap();
    // Names are 1 to 24 ASCII letters and digits (the character list's
    // rules), so a full friends list stays under Skills.
    let friends = ServerMessage::Friends(Friends {
        friends: (0..64)
            .map(|_| Friend {
                name: "a".repeat(24),
                online: true,
            })
            .collect(),
    })
    .to_bytes()
    .unwrap();
    assert_eq!(friends.len(), 1_731);
    let whisper = ServerMessage::Whisper(Whisper {
        name: "a".repeat(24),
        mine: false,
        text: "a".repeat(512),
    })
    .to_bytes()
    .unwrap();
    for other in [&points, &chat, &list, &friends, &whisper] {
        assert!(other.len() < skills.len());
    }
    assert!(frame(&skills, MAX_SERVER_FRAME).is_ok());
}

// ---------------------------------------------------------------------------
// Properties

/// Strings of at most `max` bytes of UTF-8, any characters.
fn text(max: usize) -> impl Strategy<Value = String> {
    proptest::collection::vec(any::<char>(), 0..max).prop_map(move |chars| {
        let mut text = String::new();
        for c in chars {
            if text.len() + c.len_utf8() > max {
                break;
            }
            text.push(c);
        }
        text
    })
}

fn character() -> impl Strategy<Value = Character> {
    (
        (
            any::<u8>(),
            any::<u32>(),
            text(64),
            any::<u8>(),
            any::<u8>(),
        ),
        (any::<u32>(), any::<[u8; 4]>(), any::<u16>(), any::<u16>()),
        text(64),
    )
        .prop_map(
            |((slot, id, name, class, level), (playtime, stats, body_part, hair_part), guild)| {
                Character {
                    slot,
                    id,
                    name,
                    class,
                    level,
                    playtime,
                    st: stats[0],
                    ht: stats[1],
                    dx: stats[2],
                    iq: stats[3],
                    body_part,
                    hair_part,
                    guild,
                }
            },
        )
}

fn server_message() -> impl Strategy<Value = ServerMessage> {
    prop_oneof![
        (any::<u8>(), any::<u32>(), any::<u8>(), text(1024)).prop_map(
            |(kind, vid, empire, text)| ServerMessage::Chat(Chat {
                kind,
                vid,
                empire,
                text,
            })
        ),
        proptest::collection::vec(character(), 0..=4)
            .prop_map(|characters| { ServerMessage::CharacterList(CharacterList { characters }) }),
        proptest::collection::vec((any::<u8>(), any::<u32>()), 0..=255).prop_map(|points| {
            let points = points
                .into_iter()
                .map(|(point, value)| PointValue { point, value })
                .collect();
            ServerMessage::PointsChanged(PointsChanged { points })
        }),
        (
            any::<(u8, u16, u32, u8, u32, u32)>(),
            any::<[u32; 3]>(),
            proptest::collection::vec(any::<(u8, u16)>(), 0..=7)
        )
            .prop_map(
                |((window, slot, vnum, count, flags, anti_flags), sockets, bonuses)| {
                    ServerMessage::ItemSet(ItemSet {
                        window,
                        slot,
                        vnum,
                        count,
                        flags,
                        anti_flags,
                        sockets,
                        bonuses: bonuses
                            .into_iter()
                            .map(|(kind, value)| ItemBonus { kind, value })
                            .collect(),
                    })
                }
            ),
        (
            any::<u32>(),
            proptest::collection::vec(any::<(u8, u32, u32, u8)>(), 0..=40)
        )
            .prop_map(|(vid, goods)| ServerMessage::ShopOpened(ShopOpened {
                vid,
                goods: goods
                    .into_iter()
                    .map(|(position, vnum, price, count)| ShopGood {
                        position,
                        vnum,
                        price,
                        count,
                    })
                    .collect(),
            })),
        proptest::collection::vec(any::<(u8, u8, u8, u32)>(), 0..=255).prop_map(|skills| {
            ServerMessage::Skills(Skills {
                skills: skills
                    .into_iter()
                    .map(|(skill, level, grade, next_read)| SkillState {
                        skill,
                        level,
                        grade,
                        next_read,
                    })
                    .collect(),
            })
        }),
        (any::<u8>(), any::<u16>(), any::<u16>(), text(256)).prop_map(
            |(reason, oldest, newest, text)| ServerMessage::Refused(Refused {
                reason,
                oldest,
                newest,
                text,
            })
        ),
        (
            proptest::collection::vec((text(16), text(64), text(512), text(256)), 0..=8),
            text(64)
        )
            .prop_map(
                |(providers, nonce)| ServerMessage::SignInOptions(SignInOptions {
                    providers: providers
                        .into_iter()
                        .map(|(id, name, authorize, client_id)| Provider {
                            id,
                            name,
                            authorize,
                            client_id,
                        })
                        .collect(),
                    nonce,
                })
            ),
        (text(64), proptest::collection::vec(any::<u8>(), 0..=32))
            .prop_map(|(account, ticket)| ServerMessage::SignedIn(SignedIn { account, ticket })),
        (any::<u32>(), any::<u32>(), any::<i32>(), any::<i32>()).prop_map(
            |(shooter, target, x, y)| ServerMessage::Projectile(Projectile {
                shooter,
                target,
                x,
                y,
            })
        ),
        (any::<u32>(), any::<u8>(), any::<u8>(), any::<u8>()).prop_map(
            |(vid, job, group, level)| ServerMessage::TeacherOpened(TeacherOpened {
                vid,
                job,
                group,
                level,
            })
        ),
        (any::<u16>(), proptest::collection::vec(any::<u8>(), 0..=12))
            .prop_map(|(slot, skills)| ServerMessage::ChooseSkill(ChooseSkill { slot, skills })),
        (text(24), any::<bool>(), text(512))
            .prop_map(|(name, mine, text)| ServerMessage::Whisper(Whisper { name, mine, text })),
        (text(24), any::<u8>()).prop_map(|(name, reason)| ServerMessage::WhisperRefused(
            WhisperRefused { name, reason }
        )),
        proptest::collection::vec((text(24), any::<bool>()), 0..=64).prop_map(|friends| {
            let friends = friends
                .into_iter()
                .map(|(name, online)| Friend { name, online })
                .collect();
            ServerMessage::Friends(Friends { friends })
        }),
        (text(24), any::<u8>()).prop_map(|(name, status)| ServerMessage::FriendChanged(
            FriendChanged { name, status }
        )),
        text(24).prop_map(|name| ServerMessage::FriendAsked(FriendAsked { name })),
        (text(24), any::<u8>())
            .prop_map(|(name, notice)| ServerMessage::FriendNotice(FriendNotice { name, notice })),
        (any::<u32>(), any::<u16>())
            .prop_map(|(vid, look)| ServerMessage::EntityMount(EntityMount { vid, look })),
        (
            (any::<u8>(), any::<u8>(), any::<u16>()),
            (any::<u16>(), any::<u16>(), any::<u16>()),
            (
                any::<bool>(),
                any::<bool>(),
                any::<u8>(),
                any::<Option<u32>>()
            ),
        )
            .prop_map(
                |(
                    (level, grade, look),
                    (stamina, most, ride_from),
                    (fights, skills, skill_points, summoned),
                )| {
                    ServerMessage::MountState(MountState {
                        level,
                        grade,
                        look,
                        stamina,
                        most,
                        ride_from,
                        fights,
                        skills,
                        skill_points,
                        summoned,
                    })
                }
            ),
        any::<u32>().prop_map(|run_speed| ServerMessage::RunSpeed(RunSpeed { run_speed })),
        (any::<u32>(), proptest::option::of(mount_offer()))
            .prop_map(|(vid, next)| { ServerMessage::StableOpened(StableOpened { vid, next }) }),
        proptest::collection::vec(
            (any::<u8>(), any::<u8>(), any::<i32>(), any::<u32>()).prop_map(
                |(skill, point, amount, seconds)| Affect {
                    skill,
                    point,
                    amount,
                    seconds,
                }
            ),
            0..=64
        )
        .prop_map(|affects| ServerMessage::Affects(Affects { affects })),
        upgrade_offer_message(),
        (any::<u8>(), any::<u32>(), any::<u32>())
            .prop_map(|(kind, from, to)| ServerMessage::Flight(Flight { kind, from, to })),
        (any::<u8>(), any::<u32>())
            .prop_map(|(kind, vid)| ServerMessage::EntityEffect(EntityEffect { kind, vid })),
        (any::<u32>(), proptest::collection::vec(any::<u8>(), 0..=16))
            .prop_map(|(vid, skills)| ServerMessage::EntitySkills(EntitySkills { vid, skills })),
        (any::<u32>(), any::<u8>(), any::<i32>(), any::<i32>())
            .prop_map(|(vid, step, x, y)| { ServerMessage::Fishing(Fishing { vid, step, x, y }) }),
        proptest::collection::vec(
            (any::<u32>(), any::<u8>(), any::<i32>(), any::<u32>()).prop_map(
                |(item, point, amount, seconds)| ItemBoost {
                    item,
                    point,
                    amount,
                    seconds,
                }
            ),
            0..=16
        )
        .prop_map(|boosts| ServerMessage::ItemBoosts(ItemBoosts { boosts })),
    ]
}

fn upgrade_offer_message() -> impl Strategy<Value = ServerMessage> {
    (
        (
            proptest::option::of(any::<u32>()),
            proptest::option::of(any::<u16>()),
            any::<u16>(),
            any::<u32>(),
            any::<u32>(),
        ),
        (
            any::<u32>(),
            proptest::collection::vec((any::<u32>(), any::<u8>()), 0..=4),
            any::<u8>(),
            any::<u8>(),
        ),
    )
        .prop_map(
            |((smith, scroll, slot, vnum, next), (gold, items, chance, on_fail))| {
                ServerMessage::UpgradeOffer(UpgradeOffer {
                    smith,
                    scroll,
                    slot,
                    vnum,
                    next,
                    gold,
                    items: items
                        .into_iter()
                        .map(|(vnum, count)| UpgradeCost { vnum, count })
                        .collect(),
                    chance,
                    on_fail,
                })
            },
        )
}

fn mount_offer() -> impl Strategy<Value = MountOffer> {
    (
        (
            any::<u8>(),
            any::<u8>(),
            any::<u16>(),
            any::<u8>(),
            any::<u16>(),
        ),
        (any::<u8>(), any::<u32>(), any::<u8>()),
        proptest::collection::vec((any::<u32>(), any::<u8>()), 0..=8),
    )
        .prop_map(
            |(
                (level, grade, look, move_speed, stamina),
                (rider_level, gold, skill_points),
                items,
            )| {
                MountOffer {
                    level,
                    grade,
                    look,
                    move_speed,
                    stamina,
                    rider_level,
                    gold,
                    items: items
                        .into_iter()
                        .map(|(vnum, count)| MountCost { vnum, count })
                        .collect(),
                    skill_points,
                }
            },
        )
}

fn client_message() -> impl Strategy<Value = ClientMessage> {
    prop_oneof![
        (any::<u16>(), text(64))
            .prop_map(|(protocol, build)| ClientMessage::Hello(Hello { protocol, build })),
        (text(64), proptest::collection::vec(any::<u8>(), 0..=32)).prop_map(
            |(username, ticket)| ClientMessage::TicketLogin(TicketLogin { username, ticket })
        ),
        (any::<u8>(), text(64), any::<u8>(), any::<Option<u8>>()).prop_map(
            |(slot, name, class, empire)| ClientMessage::CreateCharacter(CreateCharacter {
                slot,
                name,
                class,
                empire,
            })
        ),
        (any::<u8>(), text(512))
            .prop_map(|(kind, text)| ClientMessage::SendChat(SendChat { kind, text })),
        (any::<u8>(), any::<Option<u32>>())
            .prop_map(|(skill, target)| ClientMessage::UseSkill(UseSkill { skill, target })),
        any::<bool>().prop_map(|town| ClientMessage::Restart(Restart { town })),
        Just(ClientMessage::StartSignIn(StartSignIn)),
        Just(ClientMessage::LeaveGame(LeaveGame)),
        any::<u8>().prop_map(|point| ClientMessage::RaiseStatus(RaiseStatus { point })),
        any::<u32>().prop_map(|vid| ClientMessage::LearnSkillGroup(LearnSkillGroup { vid })),
        (any::<u16>(), any::<u8>()).prop_map(|(slot, skill)| ClientMessage::UseItemOnSkill(
            UseItemOnSkill { slot, skill }
        )),
        (text(24), text(512))
            .prop_map(|(name, text)| ClientMessage::SendWhisper(SendWhisper { name, text })),
        text(24).prop_map(|name| ClientMessage::AskFriend(AskFriend { name })),
        (text(24), any::<bool>())
            .prop_map(|(name, accept)| ClientMessage::AnswerFriend(AnswerFriend { name, accept })),
        text(24).prop_map(|name| ClientMessage::RemoveFriend(RemoveFriend { name })),
        any::<bool>().prop_map(|ride| ClientMessage::RideMount(RideMount { ride })),
        any::<u32>().prop_map(|vid| ClientMessage::TrainMount(TrainMount { vid })),
        (any::<u32>(), any::<u16>())
            .prop_map(|(vid, slot)| ClientMessage::GiveItem(GiveItem { vid, slot })),
        (any::<u16>(), any::<u16>())
            .prop_map(|(slot, onto)| ClientMessage::UseItemOnItem(UseItemOnItem { slot, onto })),
        any::<u16>().prop_map(|angle| ClientMessage::Cast(Cast { angle })),
        Just(ClientMessage::ReelIn(ReelIn)),
        (
            proptest::option::of(any::<u32>()),
            proptest::option::of(any::<u16>()),
            any::<u16>(),
            any::<u32>()
        )
            .prop_map(
                |(smith, scroll, slot, vnum)| ClientMessage::Upgrade(Upgrade {
                    smith,
                    scroll,
                    slot,
                    vnum
                })
            ),
        (text(16), text(4096), text(128), text(64)).prop_map(
            |(provider, code, verifier, redirect)| ClientMessage::ProviderLogin(ProviderLogin {
                provider,
                code,
                verifier,
                redirect,
            })
        ),
        (any::<(u8, u8, u16, i32, i32, u32)>()).prop_map(|(kind, argument, angle, x, y, time)| {
            ClientMessage::CharacterMove(CharacterMove {
                kind,
                argument,
                angle,
                x,
                y,
                time,
            })
        }),
    ]
}

proptest! {
    #[test]
    fn server_messages_round_trip(message in server_message()) {
        let bytes = message.to_bytes().unwrap();
        prop_assert_eq!(ServerMessage::decode(&bytes), Ok(message));
    }

    #[test]
    fn client_messages_round_trip(message in client_message()) {
        let bytes = message.to_bytes().unwrap();
        prop_assert_eq!(ClientMessage::decode(&bytes), Ok(message));
    }

    /// Any bytes: no panic, and what decodes encodes back to the same bytes
    /// (one encoding per message, so nothing can hide in a second one).
    #[test]
    fn decoding_is_safe_and_exact(bytes in proptest::collection::vec(any::<u8>(), 0..64)) {
        if let Ok(message) = ServerMessage::decode(&bytes) {
            prop_assert_eq!(message.to_bytes().unwrap(), bytes.clone());
        }
        if let Ok(message) = ClientMessage::decode(&bytes) {
            prop_assert_eq!(message.to_bytes().unwrap(), bytes);
        }
    }

    /// Valid messages with bytes changed: no panic, and the same rule.
    #[test]
    fn damaged_messages_are_safe(
        message in server_message(),
        changes in proptest::collection::vec((any::<prop::sample::Index>(), any::<u8>()), 1..4),
    ) {
        let mut bytes = message.to_bytes().unwrap();
        for (at, value) in changes {
            let at = at.index(bytes.len());
            bytes[at] = value;
        }
        if let Ok(message) = ServerMessage::decode(&bytes) {
            prop_assert_eq!(message.to_bytes().unwrap(), bytes);
        }
    }

    /// A stream of frames cut anywhere comes out whole.
    #[test]
    fn streams_deframe_wherever_they_are_cut(
        messages in proptest::collection::vec(server_message(), 1..6),
        cuts in proptest::collection::vec(1usize..500, 0..20),
    ) {
        let stream: Vec<u8> = messages
            .iter()
            .flat_map(|m| frame(&m.to_bytes().unwrap(), MAX).unwrap())
            .collect();
        let mut deframer = Deframer::new(MAX);
        let mut got = Vec::new();
        let mut rest = stream.as_slice();
        for cut in cuts {
            let (part, tail) = rest.split_at(cut.min(rest.len()));
            deframer.push(part);
            rest = tail;
            while let Some(bytes) = deframer.next_frame().unwrap() {
                got.push(ServerMessage::decode(&bytes).unwrap());
            }
        }
        deframer.push(rest);
        while let Some(bytes) = deframer.next_frame().unwrap() {
            got.push(ServerMessage::decode(&bytes).unwrap());
        }
        prop_assert_eq!(got, messages);
    }
}

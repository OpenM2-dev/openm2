# Server: the session

What openm2's own server (`m2-server-core`) sends and accepts, from the first byte of a connection to leaving the game; its monsters and NPCs, fights, deaths and points are in [`world.md`](world.md), its content in [`content.md`](content.md). It speaks openm2's own protocol ([`../protocol/openm2/`](../protocol/openm2/README.md)) and serves the openm2 client only.

Each rule says where it comes from:
- **QCX:** Quantum Core X `007c7a6` does the same (its source and captures of sessions against it), so the game plays as players know it;
- **openm2:** our own choice, where QCX does something else or nothing.

**Checked by:** the scenario tests (`crates/m2-server-core/tests/scenarios.rs`, `sign_in.rs`, snapshots beside them), random sessions (`random_sessions.rs`), real clients against the running server (`crates/m2-server/tests/clients.rs`), and the `openm2 bot` binary on our server (`crates/m2-cli/tests/own_server.rs`), whose messages on entering come in the order below.

## Connections
- Two listeners in one process: auth (11002) and game (13001), on 127.0.0.1 unless the operator says otherwise. QCX runs both in one process too (its `Single` build).
- **TLS first** (openm2; [`../protocol/openm2/README.md`](../protocol/openm2/README.md), "TLS"): every connection starts with a TLS 1.3 handshake with ALPN `openm2/1`, on the server's certificate (`docs/server.md`, "TLS"). The core hears of a connection only once its handshake is done:
  - a handshake that fails, ends without `openm2/1` or takes more than 10 s closes the connection (logged at debug);
  - at most 256 handshakes are under way at once, and 16 from one address; past them a new connection is closed at once, so connections that never finish their handshakes cannot hold the server.
- Every connection that finished its handshake gets a number; the core handles one input at a time.
- **Limits** (openm2; QCX has none):
  - at most 1,000 connections, and 16 from one address; past them a connection gets Refused (2 full, 3 too many from the address) and is closed;
  - a client that sends more than 100 messages in a second, a frame over 8,192 bytes, or a message that does not decode, is closed; a message of an id the server does not know is skipped ([`../protocol/openm2/README.md`](../protocol/openm2/README.md), "Frames");
  - a client that does not read what it is sent (2,048 messages queued) is closed;
  - a connection silent for 60 s is closed (the game server's pings keep a live client talking, and the auth server's while a player signs in with a provider).
- What is queued for a connection goes out in one write: one TLS record.

## Hello
On every connection ([`../protocol/openm2/session.md`](../protocol/openm2/session.md), "Versions"; openm2):
1. The client speaks first: Hello {the minor version of the protocol, its build}. The server sends nothing before it.
2. A version the server speaks (3 only) gets Welcome {its version, `openm2-server` and its version}. The client's build goes to the log with control characters as `?`.
3. Any other version gets Refused {1, the oldest and newest versions it speaks, a line for the player}, and the connection is closed.
4. Any other message before Hello, a second Hello, or no Hello within 10 s of the handshake closes the connection.
5. Done: the game server pings every 5 s from then on.

## Logging in
### Signing in with a provider (auth server)
There are no passwords (openm2; [`../protocol/openm2/session.md`](../protocol/openm2/session.md), "Signing in with a provider"; `docs/server.md`, "Sign-in with a provider"). Players sign in with a provider, or with a device's key when the operator allows it ("Signing in with a device key" below). `openm2-server serve` does not start without a way in.
- **StartSignIn**, after Hello and before a login only: SignInOptions {the providers, in the operator's order, at most 8; a nonce: 16 bytes from the tickets' ChaCha20, in hex; whether devices' keys are taken (`Config::key_sign_in`)}. With no way in, an empty list, an empty nonce and false. A newer StartSignIn replaces the nonce. With a way in, the connection is pinged every 5 s from the first one, and closed if it has not signed in 10 minutes after it.
- **ProviderLogin**, after Hello and before a login only. The nonce is used up, whatever follows. Refused at once (LoginRefused **6**, the connection closed, a failure of the address) without a nonce, for a provider the server does not have (never `key`, reserved for devices' keys), or unless the redirect is `http://127.0.0.1:<port>/callback` (the port 1 to 65535, no leading zero), the verifier 43 to 128 of `A–Z a–z 0–9 - . _ ~` (RFC 7636) and the code 1 to 4,096 printable ASCII characters (RFC 6749).
- **Throttling** (openm2): after 5 failures in a minute from one address, every ProviderLogin and KeyLogin from it gets LoginRefused **4** unchecked, the connection closed, until the minute is over. Players behind one address share its count.
- **The edge checks it** (`m2_server::oidc`), off the core's task:
  - it exchanges the code at the provider's token endpoint: the code, the redirect, the client id, the verifier and the client secret if the provider has one (`client_secret_post`);
  - the ID token must be signed with RS256 or ES256 by one of the provider's keys, the algorithm taken from the key (no `none`, no HMAC); name a key (`kid`) unless the provider has one key of that algorithm; have `iss` exactly the configured issuer, the client id in `aud` (and `azp` the client id when `aud` has several or `azp` is there), `exp` not past and `iat` not ahead of now by more than 60 s, `nonce` the connection's, and `sub` of 1 to 255 bytes;
  - keys are fetched again when 15 minutes old or when a token names an unknown key, at most once a minute; a failed fetch keeps the old keys;
  - requests are HTTPS (plain HTTP to a provider on the loopback address only), 10 s each, without redirects, answers of 64 KiB at most; at most 32 sign-ins are checked at once.
- **Answers:**
  - SignedIn {the account's name, a ticket}: storage finds the account by the issuer and subject, or makes it ([`storage.md`](storage.md), "Accounts"). The ticket is 32 random bytes from the server's ChaCha20 (seeded by the operating system), used once, valid for 30 s, bound to the account; a newer sign-in of the account replaces its older ticket (openm2).
  - LoginRefused **6**: the provider refused the code, or the token failed a check; a failure of the address; the connection closed.
  - LoginRefused **7**: the provider did not answer (or answered with a server error), or 32 sign-ins were under way; the connection closed, not counted as a failure.
  - LoginRefused **2**: the account is on the game server, **always**, however often it is tried (openm2: QCX let a second login within a minute in and dropped the first session, a bug); the connection closed.
  - Storage failing closes the connection.
- LoginRefused **1** is reserved and never sent.
- **Logs and metrics:** a sign-in is logged as `account N signed in with ID`; refusals with their reason, and the edge's reason at debug, never the code, the verifier or a token; `openm2.sign_ins` counts them by provider and result.

### Signing in with a device key (auth server)
openm2's own design ([`../protocol/openm2/session.md`](../protocol/openm2/session.md), "Signing in with a device key"), off unless the operator turns it on (`--key-sign-in`, `OPENM2_KEY_SIGN_IN`; [`../../server.md`](../../server.md), "Sign-in with a device key").
- **KeyLogin**, after Hello and before a login only, as ProviderLogin. The nonce is used up, whatever follows. Refused at once (LoginRefused **8**, the connection closed, a failure of the address) when the server does not take keys or no nonce was given out; after 5 failures in a minute from the address, **4** unchecked.
- **The edge checks it** (`m2_server::key_sign_in`): the TLS exporter of the connection the KeyLogin came on (RFC 9266's `tls-exporter`, 32 bytes, taken once its handshake is done) and the nonce make the signed bytes; the signature must be the key's over them (Ed25519, aws-lc-rs). A connection without the binding, a signature of another connection or nonce, a key that is not one: refused.
- **Answers:**
  - SignedIn {the account's name, a ticket}, as for a provider: storage finds the account by the issuer `openm2:key` and the key, written `ed25519:` and 64 hex digits, as the subject, or makes it ([`storage.md`](storage.md), "Accounts"). Its name is `key:` and 13 characters. KeyLogin names no account: a key reaches its own and nobody else's.
  - LoginRefused **8**: the signature did not verify (a failure of the address), or could not be checked (not one).
  - LoginRefused **2**: the account is on the game server, as for a provider.
- **Who may:** anyone who reaches the auth server makes an account with a key of their own; a key gives no other right. A game master is still only an account the operator names (`--game-masters`, or `--owner-key`: "Game masters").
- **Logs and metrics:** `account N signed in with key`; `openm2.sign_ins` counts them under the provider `key`. The public key and the signature are not secrets; the device's private key never reaches the server.

### Ticket (game server)
- TicketLogin, after Hello only.
- The ticket must be the one given out to the account named (compared without case), not expired or used. It is compared in a time that does not depend on where it differs.
- Anything else gets LoginRefused **5** and the connection is closed. Wrong tickets count as failures of the address; after 5 in a minute the address's ticket logins get LoginRefused **4** unchecked (openm2). A wrong ticket does not cancel the right one, so nobody can lock a player out by guessing.
- One game connection per account; a second gets LoginRefused **2** and is closed.
- Then storage loads the account's characters, once no save of the account is under way: the save made when it last left lands first (openm2, [`storage.md`](storage.md)).

## Characters
### The list
- Empire, then Phase 1, then CharacterList (QCX order):
  - Empire: the account's, else its first character's, else **0** for an account without one (openm2); **0** for a game master's account too, whose new characters each pick their empire.
  - CharacterList: the account's characters by slot (0 to 3); free slots are left out. Each has its slot, id, name, class, level, play time **in minutes** and stats (QCX); the body part the worn armour's number, hair 0 ([`world.md`](world.md), "Items"). Guild names empty.

### Creating
- CreateCharacter, on the list only, one at a time.
- **The empire:** an account without characters and without an empire takes the one in the message, 1 to 3. On any other account the message's empire is not read (openm2: QCX would move the account's first character to a new empire), but on a game master's: its character takes the message's empire, 1 to 3, else the account's, the account keeps its first character's, and Empire 0 follows CharacterCreated, so its client lets the next one pick again (openm2).
- Refused with CharacterCreateFailed **1** (openm2; QCX closes the connection for most of these):
  - no empire for the account and none in the message;
  - a name that is not 1 to 24 ASCII letters and digits (QCX checks nothing but that the name is free);
  - a class past 7;
  - four characters already.
- **The slot:** the one asked for if it is free, else the first free one (openm2: QCX ignores it and takes the next).
- **A new character** (the values are QCX's defaults; an operator can change them in `m2_server_core::Config`):
  - level 1, or `start.toml`'s level with the status and skill points of the level-ups skipped ([`content.md`](content.md)); no play time, skill group 0;
  - stats by job (warrior, ninja, sura, shaman = class modulo 4): 6/4/3/3, 4/3/6/3, 5/3/3/5, 3/4/3/6 (strength, vitality, dexterity, intelligence);
  - where it starts, by empire: Shinsoo (475000, 966100), Chunjo (60000, 156000), Jinno (963400, 278200). Each must lie on a map of the client's atlas, or `openm2 play` cannot draw the character.
- Storage stores it if the name is free (compared without case) and the slot too, and gives it its id. The account takes the character's empire if it had none.
- **Answers:** CharacterCreated {the character as in the list} (QCX), or CharacterCreateFailed **0** for a name that is taken.

### Selecting
- SelectCharacter with a slot that holds a character; an empty slot closes the connection (QCX).
- The character gets a vid: the next number not in use, from 1 (QCX counts the same way, shared with monsters and items).
- **Phase 2 first**, then MainCharacter {vid, class, name, position, empire, skill group, run speed (as "Moving" times walks; openm2)}, then Points, then a QuickSlotSet for each quickslot that holds something (QCX order; the client sends EnterGame only after MainCharacter in phase 2).
- **Points** ([`world.md`](world.md), "Points"): every point that is not 0: level, experience and what the next level needs, health and mana with their maxima, gold, stats, defence, attack grade, speeds, status and skill points, play time. From then on the player gets PointsChanged with only the points that changed since it was last told (openm2). Health and mana are the stored ones, at most their maxima (openm2: QCX fills them on every selection); a new character starts with both full.

## In game
### Entering
- EnterGame after the selection only.
- Phase 3, GameTime (milliseconds since the server started, the low 32 bits), Channel 1, our EntitySpawn {vid, angle, position, kind 6, race = class, movement and attack speed (150 and 100, changed by what it wears), state 1 if it left the game dead, else 0} and EntityInfo {vid, name, its armour and weapon, empire, level}, then an ItemSet for each item it holds, the bag's then the worn ones, then Skills (QCX order; [`world.md`](world.md), "Items" and "Skills"). A character that enters dead gets the restart window ([`world.md`](world.md), "Dying").
- Then every player within **10,000 units** (100 m, QCX's view distance) gets our EntitySpawn and EntityInfo, and we get theirs; then we get the monsters and NPCs around, then the items on the ground ([`world.md`](world.md)).

### Moving
- CharacterMove in game only; a kind past 4 without the skill bit `0x80` closes the connection (QCX). A dead character's moves are ignored (openm2).
- **Checked from where the server has the character** (openm2: QCX trusts positions). The move's `time` is read as the server's clock (the nearest time with its 32 bits), then held to no earlier than the character's last move nor **500 ms** before now, and no later than now. Where the character is at that time, along its walk, is where the move is checked from ([`movement.md`](../protocol/openm2/movement.md), "Our character walks"):
  - a walk (kind 1) must stay on the map it starts on and cross no blocked square of the map's walk mask ([`world.md`](world.md), "Walls"). It then takes the character in a straight line from there to the move's point over the walk's duration (below), counted from the move's time, as QCX moves it; it is saved where it is along the way;
  - other kinds must be on the map, within **2 m** of that point, with no blocked square between; they put the character at the move's point at once;
  - a stop (kind 0) at the end of the walk the server is still timing changes only the direction: the walk goes on, and the players watching get nothing (the client arrived first).
- **A refused move** stops the character where the server has it now. The players that see it get EntityMove {vid, kind 0, argument 0, its angle, that point, 0}, the mover gets **MoveRefused** {that point}, and the log an event (`move refused: through a wall`, `off its map`, `out of reach`). A later move is checked from there.
- The direction is the move's angle, in degrees.
- Who sees whom is worked out from where the walker is: at each move, then on the server's tick every 5 m of a walk and where it ends (QCX: on each tick; [`world.md`](world.md), "View").
- Every move goes to the players that see the mover as EntityMove {vid, kind, argument, angle, point, duration: what is left of the walk}, never back to the mover (QCX).
- **Duration** of a walk (kind 1): distance / run speed × 1000, whole milliseconds, × the movement speed's percent / 100 ([`movement.md`](../protocol/openm2/movement.md), "Speed"). The run speed is the race's `run_speed` from the content ([`content.md`](content.md), "Race"; the import takes it from the race's run motion, as QCX reads each race's motion data), else **450** units a second (openm2: the warrior's run motion; QCX sends 0 without motion data). MainCharacter tells the client the same speed, so both time walks alike (openm2). Other kinds have duration 0.
- After a move, and as a walker passes each 5 m and arrives, players who came within 100 m get each other's EntitySpawn and EntityInfo, and players now farther than **120 m** get each other's EntityRemove (QCX recomputes who sees whom on each tick, with no margin; openm2's margin keeps players at the edge from flickering, [`world.md`](world.md), "View"); monsters and NPCs come and go the same way ([`world.md`](world.md)).

### Chat
- SendChat in game only; empty lines, lines over 512 bytes (what SendChat carries) and lines with control characters are dropped (openm2).
- **Talk** (kind 0): `Name: text`, kind 0, the speaker's vid and empire, to the speaker and every player that sees it (QCX).
- **Shout** (kind 6): the same line to every player in game, with the speaker's vid and empire (openm2: QCX sends vid 0 and Shinsoo's empire, so the client cannot mark a shout as ours).
- Other kinds are ignored (QCX).
- **Commands:** a talk line starting with `/`.
  - `/logout` and `/quit`: the info line `Logging out.`, then the character is saved and the connection closed (QCX does the same, with other words).
  - `/skillgroup 1` or `2`: the skill group ([`world.md`](world.md), "Skills"), for every player in a world without teachers; in one with them a game master's command (below), and `Unknown command skillgroup` for the others.
  - `/account`: the info line `Your account: <name>`, the account's name (openm2), the one an operator lists to make it a game master (below).
  - `/where` (openm2): where the character stands at that moment, along a walk under way, as content files write it ([`content.md`](content.md), "The folder"), for an operator writing spawns and warps. On a map of the content (`Content::map_at`: of maps that overlap, the first by name): the info line `map = "<name>", at = [X, Y], rotation = R; world [WX, WY]`, the point in metres from the map's top-left corner, the way it faces in degrees (0 south, 90 east; the angle of its last move, 0 before one) and its point of the world in metres. On none, and in an empty world: `On no map; world [WX, WY]`. Metres to the centimetre, with at least one decimal (`244.53`, `61.0`), so the numbers are TOML's floats. The dead get it too.
  - A game master's commands (below), for the accounts the operator names.
  - Anything else: the info line `Unknown command <name>` (QCX), a game master's commands too for anyone who is not one. Restarting and spending skill and status points are messages of their own (Restart, RaiseSkill, RaiseStatus).
  - Info lines are kind 1, vid 0 and empire 0, control characters as `?` (openm2: QCX sends the player's empire).

### Whispers
openm2's own rules ([`../protocol/openm2/chat.md`](../protocol/openm2/chat.md), "Whispers").
- SendWhisper in the game only. Its text is checked as chat's: empty, over 512 bytes or with control characters, it is dropped without an answer.
- A name no character could have (1 to 24 ASCII letters and digits), or our own in any case, is ignored (an `Ignored` event).
- Every other whisper counts towards `Config::whisper_burst` (10) in `Config::whisper_window` (10 s), whether or not it arrives; past it, WhisperRefused reason 2.
- The name is looked up among the players in the game only, by name in lowercase (`Server::names`, kept at entering and leaving the game): never in storage. Nobody of that name in the game, whether a character has it or not, gets WhisperRefused reason 1.
- Otherwise the other player gets Whisper from us (`mine` false, our character's name) and we get it back as ours (`mine` true, the other's name as it was created).
- Nothing of a whisper is kept, and its text goes into no event or log.

### Friends
openm2's own rules ([`../protocol/openm2/friends.md`](../protocol/openm2/friends.md)).
- **AskFriend**, in the game only; a name no character could have, or our own, is ignored. Then in turn: past `Config::friend_ask_burst` (5) in `Config::friend_ask_window` (60 s), notice 6; nobody of that name in the game, 2; friends already, 3; our list full (`Config::max_friends`, 50, never more than 64), 4; theirs full, 5. A request of theirs to us waiting: asking back accepts it. Ours to them waiting: notice 1 again, they are not asked twice. More than `Config::max_friend_asks` (5) waiting for them: 6. Otherwise the request waits `Config::friend_ask_time` (60 s), they get FriendAsked and we get notice 1.
- **AnswerFriend**: a request from that player to us must be waiting, else notice 9. No: the asker gets notice 7. Yes: both lists are checked again (full: 4 to the full side, 5 to the other); then each list gains the other, the friendship is written, and both get FriendChanged 2. The event names the two characters' numbers only.
- **RemoveFriend**: the name must be in our list, in any case, else it is ignored. The friendship ends on both sides at once, whether or not the friend is in the game: our list loses it and we get FriendChanged 0; the friend's list loses us wherever the server holds it (in the game, picked, or the character list), and it gets FriendChanged 0 if it is in the game with that character.
- **Entering:** Friends, each friend marked in the game or not (in the game with that character), only when there are some; each friend in the game whose list holds us gets FriendChanged 2. **Leaving:** they get FriendChanged 1, and the requests we made or were asked end (the other side gets 8 or 9). A request whose time is up ends at a tick the same way.
- **Written** on its own, never with a save (`storage.md`, "Friends"); a failed write is logged and the lists in memory stay as they are. Each write counts among both accounts' saves under way, so a login of either waits for it; a login whose characters are loading when a friendship of theirs changes loads them again after the write.

### Game masters
openm2's own design (QCX has commands of its own for its operators).
- **Who:** the accounts the operator names in the server's setting (`--game-masters`, `OPENM2_GAME_MASTERS`: account names separated by commas, as `/account` shows them; [`../../server.md`](../../server.md), "Game masters"), compared without case, and the account of the owner's device key (`--owner-key`, `OPENM2_OWNER_KEY`: the key as `openm2 key public` prints it, with `--key-sign-in`); nobody without them. The server knows a connection's account from the ticket it gave out, never from what the client says. A name is the provider's id and a hash of the player's issuer and subject (`storage.md`, "Accounts"), so it stays the same when the database is wiped and made again; an account's number does not.
- **The commands**, talk lines like the others; each answers with an info line, and a wrong use changes nothing:
  - `/level N`: the character's level becomes N, from 1 to the highest (one past the last of `levels.toml`, else `Config::max_level`, 99). Each level gained gives the status and skill points of a level-up; going down takes none back. Its experience starts again from 0 and its health and mana are filled; it gets PointsChanged, and it and the players that see it get its EntityInfo, as after a level-up, and before it EntityEffect of a level gained when the level went up. `Level N.`
  - `/gold N`: N more gold, 1 to `Config::max_gold` (2,000,000,000), the whole at most that too, else `You cannot carry more gold.` PointsChanged; `N gold.`
  - `/item NUMBER [COUNT]`: COUNT of the item (1 without it; 1 to its stack: 200 if it stacks, else 1) into the bag, onto stacks of it first, as a shop gives it; its ItemSets. Not item 1 (gold in the client's tables) or one the content lacks: `No item NUMBER.` No room: `No room in the bag.`, nothing given. `Given COUNT of NAME.`
  - `/heal`: health and mana filled; PointsChanged; `Healed.`
  - `/mount N`: the character's mount at level N, 0 (none) to the last of `mounts.toml`, with all its stamina and the horseback skill points the levels up to it give; a rider gets off and the mount goes away first. With a mount, the call goes into the bag when it holds none. MountState; `Mount level N.` A world without mounts: `This world has no mounts.`; another number: `Choose a mount level from 0 to LAST: /mount 1`.
  - `/speed N` (openm2): the character moves at speed N, 1 to 1,000, whatever its items, skills and mount give: a walk at N above 100 takes 100/N of its time at 100, so 1,000 runs 6.7 times as far as the server's own 150 (45 m/s on foot at the races' 4.5 m/s). Up to 255, the most EntityUpdate's `move_speed` carries (the original's server keeps players to 200), it is that speed; past it, `move_speed` is 255 and the run speed rises by N/255 ([`world.md`](world.md), "Players' walks"), as RunSpeed tells the client ([`../protocol/openm2/mounts.md`](../protocol/openm2/mounts.md)). The server times its walks by both; its EntityUpdate to it and the players that see it (each of its walks their EntityMove's time), PointsChanged and RunSpeed. Not saved: it lasts until the character leaves the game. `Speed N, your own OWN: /speed alone gives it back.`; `/speed` alone gives the character its own speed back: `Your own speed again: OWN.`
  - `/gm off` and `/gm on`: the mark over the character is hidden, or shows again (below); its EntityUpdate to it and the players that see it, a save at once; `Your GM mark is hidden now.`, `Your GM mark shows now.`, or `… already.` with nothing changed. `/gm` says which: `Your GM mark shows: /gm off hides it.` or `Your GM mark is hidden: /gm on shows it.`
  - A dead character gets `Restart first.` and nothing changes; `/account` and `/where` still answer.
  - Wrong numbers get a line with the use: `Use: /level 1 to 99`, `Use: /gold 1 to 2000000000`, `Use: /item NUMBER [COUNT]`, `NAME comes 1 to STACK at a time.`, `Use: /speed 1 to 1000, or /speed alone for your own`, `Use: /gm on or /gm off`.
- **The mark**: a game master's character has flag 2 in its EntitySpawn's and EntityUpdate's `state` while its mark shows, to everyone that sees it and to itself ([`../protocol/openm2/entities.md`](../protocol/openm2/entities.md), "State and parts"), from the first time it enters. After `/gm off` it has none, as any player, until `/gm on`: the character keeps the choice (`incognito`, [`storage.md`](storage.md)), and an account the operator no longer names has no mark either way.
- **Recorded:** each command carried out is an event in the log: the connection, the account's and the character's numbers, and the command as the server read it (`/item 3004 200`). Logs still never hold chat.
- **Saved** with the character's next save, as a shop's gold and items are: every few minutes and when it leaves; `/gm`'s choice at once.
- Not yet: moving to another place or map by a command (warps take characters to other maps: [`world.md`](world.md), "Warps"), and commands on other players.

### Pings
- Ping every 5 s on a game connection from Welcome on (QCX). Pong is accepted and only shows the client is alive; the server does not ping back after a pong (QCX does).

### Messages out of turn
- Every game message before the game closes the connection: SetTarget, Hit, the item and quickslot messages, DropGold, Talk, the shop messages, UseSkill, RaiseSkill, RaiseStatus, Restart ([`world.md`](world.md)), SendWhisper, AskFriend, AnswerFriend and RemoveFriend (above), RideMount and TrainMount ([`world.md`](world.md), "Mounts"), GiveItem, UseItemOnItem and Upgrade ([`world.md`](world.md), "Upgrades").
- Any message out of turn closes the connection: a sign-in on the game server (StartSignIn, ProviderLogin, KeyLogin), a second sign-in while one is checked or after the ticket, a ticket on the auth server, a selection while a character is being created, a move before entering (openm2: QCX checks no phases).

## Leaving
- When a connection closes (the client left, the network failed, a rule above, or the server stopping), a character in game is saved (where it is, its play time and points, [`storage.md`](storage.md)), every player that saw it gets EntityRemove (QCX), and the monsters fighting it give it up.
- Characters in game are also saved every 5 minutes (openm2).
- **LeaveGame** (openm2's: the game menu's Change character, [`protocol/openm2/session.md`](../protocol/openm2/session.md), "Changing character") leaves the world the same way without closing: the character is saved, the players that saw it get EntityRemove and the monsters fighting it give it up. The connection is back at the character list: the account's characters are loaded again once that save has landed (a login's rule, so the list shows the character as just saved), then the Empire, Phase 1 and the CharacterList come as after the ticket. The account stays in the game meanwhile, so no other sign-in takes it. A LeaveGame anywhere but in the game closes the connection.
- Stopping the server (Ctrl+C) saves every character and closes every connection first.

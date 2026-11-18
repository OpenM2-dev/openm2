# Session

How the client reaches the game on openm2's server: the two connections, versions, signing in with a provider or a device's key, the ticket, the heartbeat, choosing or creating a character, entering the game and leaving. What the server checks on top is in [`server/session.md`](../../server/session.md).

## Connections
- The client opens two connections, one after the other, both to the host the user configured (`NetConfig::host`):
  1. **auth** (openm2's server: port 11002): signing in with one of the server's OpenID Connect providers, the only way in;
  2. **game** (port 13001): the ticket login, the character list and the game.
- It closes the auth connection as soon as it has its ticket, then connects to the game server.
- No message carries a server's address, and the client never connects anywhere but the configured host.
- Every connection starts with its TLS handshake ([`README.md`](README.md), "TLS"), then Hello ("Versions"). The game connection resumes the auth connection's TLS session.

```
client                          auth server
  ── Hello ─────────────────────▶
  ◀── Welcome ───────────────────
  ── StartSignIn ───────────────▶
  ◀── SignInOptions (providers, nonce)     (a Ping every 5 s from here on)
     … the player signs in at the provider, in the browser …
  ── ProviderLogin (code) ──────▶
  ◀── SignedIn (account, ticket)
  ── close

client                          game server
  ── Hello ─────────────────────▶
  ◀── Welcome ───────────────────          (a Ping every 5 s from here on)
  ── TicketLogin (ticket) ──────▶
  ◀── Empire, Phase 1, CharacterList
 [── CreateCharacter ───────────▶          (only to create a character)
  ◀── CharacterCreated ──────────]
  ── SelectCharacter ───────────▶
  ◀── Phase 2, MainCharacter, Points, QuickSlotSet …
  ── EnterGame ─────────────────▶
  ◀── Phase 3, GameTime, Channel, EntitySpawn, EntityInfo, ItemSet …, Skills
  ◀── the world around us
  ── close
```

`crates/m2-cli/tests/bot_transcript.rs` plays such a session against the `openm2 bot` binary and checks every byte it sends; `crates/m2-cli/tests/own_server.rs` runs the bot on openm2's server.

## Versions
- **Hello** is the client's first message on every connection: `protocol`, the minor version it speaks (the server speaks only 4, so a client of another version is refused), and `build`, its name and version (`openm2 0.1.0`) for the server's logs.
- The server answers:
  - **Welcome** when it speaks that version: its own minor version and its name. The session goes on.
  - **Refused** otherwise: `reason`, the `oldest` and `newest` minor versions it speaks, and `text`, a line for the player. Then it closes the connection.
- Refused's reasons:
  - 1: another version of the protocol;
  - 2: the server is full (1,000 connections by default);
  - 3: too many connections from the client's address (16 by default).
- Reasons 2 and 3 go out as soon as the connection's TLS handshake is done, before the server reads anything; then it closes the connection.
- The server closes a connection that sends anything before Hello, a second Hello, or no Hello within 10 s of its handshake.
- **The client:**
  - on Welcome, goes on: StartSignIn on the auth server, TicketLogin on the game server. A second Welcome is ignored. Welcome's `protocol` is reported, not checked.
  - on Refused, stops with the reason and the server's line: `the game server refused the connection: it is full (The server is full. Try again later.)`.
- Hello, Welcome and Refused never change, and the major version is the TLS ALPN name ([`README.md`](README.md), "Versions").

## Signing in with a provider
A player signs in with one of the server's OpenID Connect providers, or with their device's own key when the server takes keys ("Signing in with a device key"): there are no passwords. The flow is the authorization code flow with PKCE for a native app (RFC 8252), the code relayed to the server, which exchanges it and checks the ID token.
1. On the auth server, after its Welcome, the client sends **StartSignIn**.
2. The server answers **SignInOptions**: `providers` (each one's `id`, `name`, `authorize` endpoint and `client_id`; at most 8, in the operator's order), `nonce`, single use and good on this connection only, and `key_sign_in`, whether it takes devices' keys. A server with no way in answers an empty list, an empty nonce and `false`. Asking again gives a new nonce, which replaces the old one.
   - From the first StartSignIn of a server with a way in, the auth server pings the connection as the game server does ("Heartbeat"), and closes it if it has not signed in within 10 minutes.
3. The client signs in at the provider in the system's browser:
   - it opens the authorization endpoint only if it is `https`, or plain `http` on the loopback address when the game server is on the same machine;
   - it listens on `127.0.0.1` at a port the system picks;
   - the request carries `response_type=code`, the `client_id`, `redirect_uri=http://127.0.0.1:<port>/callback`, `scope=openid`, a random `state` (16 bytes), the server's `nonce`, and `code_challenge` (the SHA-256 of a random verifier of 32 bytes, both base64url) with `code_challenge_method=S256`, and `prompt` when the client's settings name one (`sign_in_prompt`, [`../../client.md`](../../client.md)): without it, a provider that remembers the player signs them in without showing anything;
   - it answers every browser request with a page of its own that repeats nothing of the request, and waits (5 minutes at most) for `/callback` with its `state`: with `code` it goes on, with `error` it stops (`the sign-in failed: the provider refused: access_denied`). Requests with another path or state are answered and ignored.
4. The client sends **ProviderLogin**: `provider`, `code`, `verifier` and `redirect`. The code and the verifier are `secret` fields.
5. The server checks what it can (the nonce given out, a provider of its own, the redirect exactly `http://127.0.0.1:<1–65535>/callback`, a verifier of 43 to 128 unreserved characters, a code of 1 to 4,096 printable ASCII characters), uses the nonce up, exchanges the code and checks the ID token ([`../../server/session.md`](../../server/session.md), "Signing in with a provider").
6. It answers **SignedIn**: `account`, the account's name, and `ticket` ("Logins"). A provider's player gets an account of their own at the first sign-in, found again by the provider's issuer and subject, never by e-mail.
7. The client closes the auth connection and logs in on the game server with that account name and the ticket ("Logins").

- **Refusals:** LoginRefused ("Logins"), and the connection is closed (its nonce is used up).
- **The client holds no token:** the ID token and any access token stay on the server.
- **Only asking:** a client may ask for the providers and leave, as `play`'s login screen does to show a button each.

## Signing in with a device key
A server may also take a device's own key (`openm2-server serve --key-sign-in`, [`../../server.md`](../../server.md)): an Ed25519 key the client made once and keeps in a file ([`../../client.md`](../../client.md), "The device's key"). The account is the key's: the message names no account, so a key reaches its own account and nobody else's. No browser opens.
1. On the auth server, after its Welcome, the client sends **StartSignIn**, and the server answers **SignInOptions** with `key_sign_in` true and a nonce, as above.
2. The client takes the connection's **channel binding**: 32 bytes of the TLS exporter (RFC 5705) with the label `EXPORTER-Channel-Binding` and no context, RFC 9266's `tls-exporter`. Both sides of one TLS connection get the same bytes, and no other connection gets them.
3. It signs, with Ed25519 (RFC 8032), the bytes `openm2/1 key sign-in` and a zero byte, the nonce's length as one byte, the nonce, then the 32 bytes of the binding (`m2_protocol::key_sign_in::signed_message`).
4. It sends **KeyLogin**: `public_key`, its 32 bytes, and `signature`, 64 bytes. Neither is secret: the signature is worth nothing on another connection or with another nonce.
5. The server uses the nonce up, checks the signature against the nonce and its own side's binding ([`../../server/session.md`](../../server/session.md), "Signing in with a device key"), and answers **SignedIn** as for a provider: the key's account, made at its first sign-in, and a ticket. The account is named `key:` and 13 characters.
6. The client goes on to the game server with the ticket ("Logins").

- **Refusals:** LoginRefused 8 for a server that does not take keys, a KeyLogin without a nonce, or a signature that does not verify; 4 after too many failures from the address ("Logins").
- **The client** asks for SignInOptions, and stops (`the server does not take this device's key`) when `key_sign_in` is false; it stops too when it has no key, or cannot read or sign with it.
- **A signature taken from the wire** (a capture, a server in the middle) does not sign in elsewhere: another connection has another binding, and the nonce is used up.

## Logins
1. **SignedIn**'s `ticket` is 32 random bytes:
   - single use;
   - valid for 30 s;
   - one per account: a newer sign-in replaces the account's older ticket.
2. The client closes the auth connection, connects to the game server and, after its Welcome, sends **TicketLogin**: SignedIn's `account` and the ticket.
3. The server answers with the character list ("Characters").

**LoginRefused** says why a sign-in or a ticket failed:

| `reason` | Meaning | When |
|---|---|---|
| 1 | (reserved) | never sent |
| 2 | the account is in the game | auth server, for the account of a sign-in; game server, for a valid ticket of an account that joined meanwhile |
| 3 | the server is full | not sent yet |
| 4 | too many failed tries | auth server: 5 refused sign-ins in 60 s from the address; game server: 5 wrong tickets in 60 s from the address |
| 5 | the ticket is not valid | game server: a ticket not given out, already used, expired, or given for another account name |
| 6 | the provider did not confirm the sign-in | auth server: a ProviderLogin without a nonce, for a provider the server does not have, with a redirect, verifier or code that cannot be right, or whose code or ID token the provider's checks refused ("Signing in with a provider") |
| 7 | the server could not reach the provider | auth server: the provider did not answer, or the server checks too many sign-ins at once |
| 8 | the server did not accept this device's key | auth server: a KeyLogin on a server that does not take keys, without a nonce, or whose signature is not the key's over the nonce and the connection's binding ("Signing in with a device key") |

- **Every refusal closes the connection,** on both servers.
- **Failures** are counted over the last 60 s per address: refused sign-ins on the auth server, devices' keys among them (not 7: the provider's trouble is not the player's), wrong tickets on the game server. A refusal for too many tries does not count again. Players behind one address share its count.
- **The client** stops at any LoginRefused (`the auth server refused the login: the provider did not confirm the sign-in`) and does not try again on its own.
- **Secrets:** the ticket, the code and the verifier are `secret` fields ([`README.md`](README.md), "Logins"). The client keeps them in `secrecy` types and never takes them from a command-line argument.

## Heartbeat
- The game server sends **Ping** every 5 s from its Welcome on, so also during the login, the character select and loading. The auth server pings a connection from its first StartSignIn ("Signing in with a provider").
- The client answers every Ping with **Pong**, at once.
- The server takes a Pong only as a sign of life and sends nothing back.
- **Silence:**
  - the server closes a connection that has sent nothing for 60 s (any message counts);
  - the client gives up on a connection that has sent nothing for 30 s (`the game server sent nothing for 30 s while in game`). With a Ping every 5 s, that only happens when the server is gone.

## Characters
After a valid ticket the server loads the account's characters and sends, in this order:
1. **Empire:** the account's empire (1 Shinsoo, 2 Chunjo, 3 Jinno), else its first character's; 0 for an account without one, and for a game master's account, whose characters each pick their own.
2. **Phase** 1: the character list.
3. **CharacterList:** only the slots (0 to 3) that hold a character. Each has `slot`, `id`, `name`, `class`, `level`, `playtime` (in minutes), the four stats, `body_part` (the worn armour's item number, 0 for none; the select screen draws it), `hair_part` and `guild` (openm2's server sends 0 and an empty name).

Game numbers here and in the other files of this folder (empires, classes, kinds, points, windows, …) keep the values the client's game files use.

### Class values
`class` holds class and gender together:

| Value | Class | Value | Class |
|---|---|---|---|
| 0 | warrior, male | 4 | warrior, female |
| 1 | ninja, female | 5 | ninja, male |
| 2 | sura, male | 6 | sura, female |
| 3 | shaman, female | 7 | shaman, male |

The job is the value modulo 4: 0 warrior, 1 ninja, 2 sura, 3 shaman.

### Creating
- **CreateCharacter** asks for a character: `slot`, `name`, `class` and `empire`.
  - `slot`: the client sends the first free slot. The server takes it when it is free, else the first free one.
  - `name`: 1 to 24 ASCII letters and digits, the client's rule and the server's alike; it keeps names safe in chat lines and commands. No two characters share a name, compared without case.
  - `empire` (1 to 3) is set only by an account's first character, which chooses the account's empire. The client sends it only while the server says no empire (Empire 0); the server reads it only when the account has neither characters nor an empire, and always for a game master's account. Later characters join the account's empire, a game master's those they pick (it falls back to the account's for an empire out of 1 to 3), and the server says Empire 0 again after each of a game master's new characters.
- The server answers:
  - **CharacterCreated:** the new character as the character list shows it, with its slot;
  - **CharacterCreateFailed** with `reason` 0 when the name is taken, 1 for any other refusal: no empire for a new account, a name the rule above refuses, a class past 7, four characters already. The connection stays open.
- A CreateCharacter outside the character list, or while another is under way, closes the connection.
- What a new character starts with is the server's ([`server/session.md`](../../server/session.md), "Characters").
- **The client:**
  - `openm2 bot --create NAME` (with `--class` and `--empire`) plays NAME if the account has it, else creates it in the first free slot and plays it. With four other characters it stops (`the account's four slots are full and none holds NAME`); a refusal stops it too.
  - `openm2 play`'s create screen (`Command::Create`) creates the character in the first free slot, and does nothing with four characters. The new character joins the list and is shown, not played. A refusal goes back to choosing, and the screen says why (`That name is taken.`).

### Selecting
- **SelectCharacter** `slot` plays the character in that slot.
- The client picks the slot asked for, the character with the name asked for, or the first in the list.
- With `CharacterChoice::Ask` (`openm2 play`'s select screen) the session waits at `Stage::Selecting` for `Command::Select` or `Command::Create`, for as long as the player looks; the heartbeat goes on. A pick before the list, of an empty slot, or after one was made is ignored.
- The client never selects an empty slot: with no character to play it stops (`the account has no characters; create one first`).
- The server closes the connection for an empty slot or a selection out of turn.

## Entering the game
1. After SelectCharacter the server sends:
   - **Phase** 2: loading;
   - **MainCharacter:** our `vid`, class, name, position, empire and skill group, and the run speed our walks are timed with ([`movement.md`](movement.md), "Speed");
   - **Points:** every point of our character that is not 0 ([`entities.md`](entities.md), "Points");
   - a **QuickSlotSet** for each quickslot that holds something ([`inventory.md`](inventory.md), "Quickslots").
2. When MainCharacter has arrived in phase 2, the client sends **EnterGame**, once. A client with a map to load would load it first.
3. The server sends:
   - **Phase** 3: in the game;
   - **GameTime:** the server's clock, in milliseconds since it started; the client's estimate of it goes into CharacterMove ([`movement.md`](movement.md));
   - **Channel:** 1;
   - our own **EntitySpawn** and **EntityInfo** ([`entities.md`](entities.md));
   - an **ItemSet** for each item, the bag's then the worn ones ([`inventory.md`](inventory.md));
   - **Skills:** the skills learned, an empty list for none ([`skills.md`](skills.md));
   - then the world around: the players near us, the monsters and NPCs, the items on the ground ([`entities.md`](entities.md), [`ground-items.md`](ground-items.md)).
4. An EnterGame anywhere but after a selection closes the connection.

- **Phases** are 1 the character list, 2 loading, 3 in the game. Phase 1 or 2 clears what the client knows of the world (entities, ground items, the shop, items, quickslots, skills, the target, a walk); the server sends it again.
- The client is in the game from Phase 3 (`Event::EnteredGame`). It keeps what it learns in `m2_core::World`.
- Until our own EntitySpawn arrives, our character stands where MainCharacter put it. Walks should wait for the spawn: it places the character with its movement speed and ends any walk started before it. The bot and `openm2 play` wait for it.
- A character that left the game dead enters dead, and the client shows the restart window ([`combat.md`](combat.md), "Dying").

## Changing character
The client goes back to the character list without signing in again (`Command::ChangeCharacter`, the game menu's Change character):

```
client                          game server
  ── LeaveGame ─────────────────▶          (in the game only)
  ◀── EntityRemove … for the others who saw us
  ◀── Empire, Phase 1, CharacterList       (once our save has landed)
  ── SelectCharacter ───────────▶          … entering as at the login
```

- **LeaveGame** has no fields. The server saves our character and takes it out of the world, as when a connection closes ("Leaving"), and sends the list again on the same connection ([`server/session.md`](../../server/session.md), "Leaving"). Anywhere but in the game it closes the connection.
- **The client** sends it only in the game, and is at `Stage::Leaving` until the list comes; the game's commands are ignored meanwhile. Phase 1 forgets the world as always, and our character, its points and its selection too (`Event::LeftGame`); the CharacterList is then taken as after the ticket: with `CharacterChoice::Ask` the session waits at `Stage::Selecting`, else it picks as at the login. The next character enters as the first did: EnterGame once its MainCharacter has come.
- `openm2 bot --change-character` does it once after its stay, enters again and logs out after another stay.

## Leaving
- The client leaves by closing the game connection (`Command::Logout`), from the select screen as from the game.
- The chat commands `/logout` and `/quit` make the server say `Logging out.` (an info line) and close the connection itself. The client then sees the connection closed (`the game connection closed while in game: …`).
- When a game connection closes for any reason, the server saves the character, the players that saw it get EntityRemove, and the monsters fighting it give it up ([`server/session.md`](../../server/session.md), "Leaving").

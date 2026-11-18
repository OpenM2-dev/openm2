# m2-core — rules for agents

The client state machine. Messages and user commands come in, effects go out (send a message, open or close a connection, report an event). **No I/O, no clock, no threads.**

## Layout
- `src/session.rs`: `Session::step(now, input) -> Vec<Effect>`. Inputs: `Start`, `Connected` (we say `Hello`), `Closed`, `Message`, `Command` (chat, `MoveTo` with a path, `Stop`, `Select`, `Create`, the items and quickslots, fighting, the ground, shops, skills, `Restart`, logout), `SignInCode`/`SignInFailed` (what the browser brought back after `Effect::SignIn`), `KeySigned` (the device's signature after `Effect::SignWithKey`, for one connection), `Tick` (timeouts, a path's next legs, and the wait move when our walk arrives). Also the session stages, the game connection's phases, `SessionConfig` (a provider's id, the device's key (`device_key`, or the provider id `key`), or only asking for the providers; the build sent in `Hello`; `CharacterChoice`: first, by slot, by name with `NewCharacter` to create it, or `Ask`: wait at `Stage::Selecting` for `Select`/`Create`), `SessionError` with `Refusal` and `LoginRefusal`. The ticket is kept in a `SecretSlice`.
- `src/world.rs`: `World`, what the session has learned (empire, characters, our character, channel, the server clock, the entities around us with their `Track`s, points, items, quickslots, the ground, the open shop, skills, chat log, whisper conversations, friends and friend requests), and the wire enums (`Empire`, `CharacterClass`, `ChatKind`, `EntityKind`, `MoveKind`, `Window`, `QuickSlotKind`); `BAG_SLOTS`, `WORN_FROM`, `QUICK_SLOTS`. `Point` and `Track` hold positions over time; `Point::angle_to` is the server's direction rule. `World::path` is the rest of our walk along a path, and `World::follow_path(now)` starts the legs begun by `now` (the session on ticks and commands, `play` on its copy before drawing). Text from the server is `String`.
- `src/time.rs`: `Millis`, `ConnId`.
- `tests/scenarios.rs`: scenario tests; snapshots in `tests/snapshots/`.
- The flow it implements is specified in `docs/specs/protocol/openm2/`.

## Rules
- **Deterministic:** the same inputs give the same outputs, every run.
  - Time comes in as `Millis` (`now`); never read a clock.
  - `crates/m2-core/clippy.toml` forbids `Instant::now`, `SystemTime::now`, `thread::sleep`, `HashMap` and `HashSet`. Use `BTreeMap`/`BTreeSet` for anything that ends up in state dumps or snapshots.
- **Every effect and incoming message carries a `ConnId`.** Ignore messages from a connection that is no longer current; never mix the auth and game connections.
- **Core never handles addresses.** It asks for "the auth server" or "the game server" (`ServerKind`); the network layer decides where that is.
- **Exhaustive matches:** match on `ServerMessage` without a `_ =>` arm. When a message is added, the compiler shows where it needs handling. Messages of ids this build does not know never reach the core: `m2-net` skips them.
- **Replies** (Pong, the login after Welcome) are produced inside the message handler, never from a later `tick`.
- **Check what the protocol cannot carry before sending:** a value the encoder would refuse (a chat line over 512 bytes) is ignored with an event, so the network layer never drops the connection over it.
- **No panics:** `unwrap`, `expect`, indexing and `panic!` are denied in `src/`. Bad server input becomes an error event and, if it can't be recovered, a disconnect effect.
- **Secrets:** the provider's code, the PKCE verifier and the ticket stay in `secrecy` types and never appear in `Debug` output, events or the world. `secrets_never_show_in_debug_output` checks this.
- **Tests:**
  - Scenario tests feed messages and times in and snapshot the log of inputs and effects, plus the world (insta).
  - New or changed snapshot: run `INSTA_UPDATE=always cargo test -p m2-core`, then read the `.snap` diff with `git diff` line by line. Say in the handoff why it changed. (`cargo-insta` is not required.)

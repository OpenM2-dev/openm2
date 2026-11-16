---
name: add-ui-window
description: Add or change a window of openm2's interface (a RON layout in crates/m2-cli/ui/, a skin, the code that fills it in and acts on its events), then check it in a screenshot. Use when asked to add, change, move or restyle a window, panel, button, field, gauge, chat or HUD element, or anything drawn by m2-ui.
---

# Add an interface window

openm2's windows are data: a layout file (RON) says what is where, the skin file says how it looks with the user's own interface art, and a little Rust fills in values by node id and acts on events. The rules are in `docs/specs/ui.md`; read its "Layout files" and "The skin file" sections first.

## 1. Where things are
- Layouts and the skin: `crates/m2-cli/ui/` (`skin.ron` and one layout per screen or window: `intro`, `login`, `servers`, `select`, `game`, `inventory`, `shop`, `teacher`, `choose_skill`, `stable`, `upgrade`, `character`, `menu`, `whisper`, `messenger`), embedded with `include_str!` in `crates/m2-cli/src/interface.rs`.
- The code that fills windows in: `interface.rs` (`ids`, `Hud`), `inventory.rs` (the inventory and quickslot bar), `shop.rs`, `teacher.rs` (skill teachers), `choose_skill.rs` (items that train a chosen skill), `stable.rs`, `upgrade.rs`, `character.rs` (the character window), `skills.rs` (its Skills tab), `affects.rs` (the affects bar), `whisper.rs`, `messenger.rs` (the friends list), `server_list.rs` (the login screen's Servers board), `minimap.rs`, `menu.rs` and `screens.rs` (`PlayInterface`: which windows each screen shows, values from the session, clicks and keys turned into `Action`s); `play.rs` passes input to it and carries the actions out.
- A window of a new screen: add the screen to `Screen` and its windows to `Screen::windows` in `screens.rs`.
- The toolkit: `crates/m2-ui` (its `AGENTS.md`). Only change it if no combination of the existing kinds does the job.

## 2. Find the art
The art is the user's: name it by game path, never copy it into the repo. To find pieces and their sizes (needs the human's client folder and `local/keys.toml`; ask for the folder if you don't know it):
```sh
cargo build --release -p m2-cli
C=<client folder>
target/release/openm2 vfs ls --client "$C" --keys local/keys.toml "d:/ymir work/ui/pattern/"
target/release/openm2 tga info --client "$C" --keys local/keys.toml "d:/ymir work/ui/pattern/board_base.tga"
target/release/openm2 script dump --client "$C" --keys local/keys.toml "d:/ymir work/ui/public/close_button_01.sub"
```
- `.tga` files are whole images; `.sub` files are rectangles of an atlas (`docs/specs/formats/script.md`, "Sub-images").
- Boards: `pattern/board_*` (32 × 32 pieces), `pattern/thinboard_*` (16 × 16, see-through middle); bars: `pattern/titlebar_*`, `pattern/gauge_slot_*`, `pattern/chat_bar_*`; buttons: `public/*_button_0{1,2,3}.sub` (normal, hover, pressed).
- Every skin needs a fallback colour: clients without that art still get a usable window.

## 3. Write the layout
- One window per top-level node; give every node the code touches an `id` (`inventory.gold`), unique across layouts.
- Place by `anchor` and `pos` (inwards from the anchored edges) and `size` (pixels or `Fill(m)`).
- Use `click_through: true` for windows over the world that should not block walking (only their buttons and fields take clicks).
- A new skin goes into `skin.ron` with a comment saying what it is.

## 4. Fill it in and act on it
- Add the ids the code uses to `interface::ids`, and extend the test that checks every id exists in the layout.
- Values: `ui.set_text`, `set_value` (gauges, 0–1), `push_line` (logs), `set_visible`, `focus`.
- Events: `ui.take_events()` gives `Clicked(id)`, `Submitted { id, text }`, `Cancelled(id)`, `SlotPressed { id, cell, button }`, `SlotReleased { id, cell }`; turn them into `Action`s (an `m2_core::Command` for the session) in `screens.rs`, never by touching the network directly.
- Items and icons: a `Slots(columns, rows)` grid; put `SlotItem`s in its cells (`set_slot`), carry one on the pointer (`set_held`), show a `set_tooltip` for `hovered_slot`. Icons named later are loaded by `GpuInterface` on the next frame (`missing_images`).
- Tabs: `set_latched` keeps the open tab's button drawn pressed.
- Secrets (text not to show, such as a PIN; players sign in with providers, so no password field): a `secret: true` field; read it with `take_text` straight into a `SecretString`, never from the event.
- A picture the game draws (a model): an `Image` node named `@something`; `play` draws it at the node's size (`Ui::rect`) and hands it in (`PlayInterface::set_picture`).
- Test the logic with `Fonts::empty()` (no fonts, no GPU), like the tests in `interface.rs` and `screens.rs`.

## 5. Look at it
```sh
# the client imported first (the verify-visually skill, 1): view ui reads asset folders
target/release/openm2 assets import --client "$C" --keys local/keys.toml --out local/assets
A=local/assets
target/release/openm2 view ui crates/m2-cli/ui/<file>.ron --assets "$A" \
  --screen game --value taskbar.health=0.7 --show chat.bar --screenshot local/ui.png
```
- Read the PNG. To see details, crop and enlarge part of it (any small script that reads PNGs; keep it out of the repo).
- The picture shows the user's art: keep it in `local/`.
- In Git Bash, arguments starting with `/` get rewritten into paths; prefix the command with `MSYS_NO_PATHCONV=1` for values like `--set "chat.input=/m 101"`.
- A screen of `play`: `view ui --screen login|select|create|loading|game` (no character picture), or `play --screenshot --screen login` (no server needed).
- The inventory with real items: `view ui --screen game --show inventory --assets "$A" --item 0=10 --item 94=10 --item 7=27001x5 --quick 0=7` (bag slots 0–89, worn from 90; item numbers from `openm2 proto dump`).
- For the window in the game: `openm2 play … --screenshot local/play.png` on openm2's server (`docs/server.md`; ask before starting a server).

## 6. Finish
- `docs/specs/ui.md`: the window under "Play's screens" or "The in-game interface", new skins or kinds in their tables.
- `just check`, then the handoff (AGENTS.md); say which screenshots you looked at.

# Interface

How openm2 describes, lays out, drives and draws its interface: windows written as RON layout files, drawn with the user's own interface art, turned into a draw list that the renderer draws over the world.

**Code:** `crates/m2-ui` (layouts, skins, state, input, text, draw lists; no GPU, no window, no clock), `m2_render::ui` (the pipeline), `crates/m2-cli/src/interface.rs` (an asset folder's art loaded for a layout, the in-game values), `crates/m2-cli/src/screens.rs` (`play`'s screens), `crates/m2-cli/src/inventory.rs` (the inventory and the quickslots), `crates/m2-cli/src/shop.rs` (talking to NPCs and their shops), `crates/m2-cli/src/character.rs` (the character window), `crates/m2-cli/src/skills.rs` (its Skills tab), `crates/m2-cli/src/affects.rs` (the affects bar), `crates/m2-cli/src/menu.rs` (the game menu and the Settings window; `settings.rs`, the player's settings file), `crates/m2-cli/src/minimap.rs` (the minimap's and the atlas's pictures), `crates/m2-cli/ui/` (the skin file and layouts). Decision 009 explains the choices.

**Sources:** openm2's own design. The art is the user's client's: TGA files ([`formats/tga.md`](formats/tga.md)), DDS files ([`formats/dds.md`](formats/dds.md)) and sub-images ([`formats/script.md`](formats/script.md), "Sub-images"), named by game path; or an asset folder's PNG files and regions of them, named by its own skin ([`assets.md`](assets.md), "The interface's skin"). The original client's own window scripts are not read or copied; openm2's layouts are its own.

**Checked:** with `openm2 view ui --screenshot` on a TMP4 client's art: the login, select, create, loading and game screens (`--screen`), the inventory and quickslots with the client's items, the shop, the skills window, the taskbar, the minimap and the atlas, at scale 1 and scaled 2.5 times (`--ui-scale 2.5 --size 3200x2000`, sharp text with either filter); `play --screenshot` against scripted servers and openm2's own; and played in `play`'s window, from logging in to logging out. The tooltips and the frames going round are checked by tests.

## Layout files
A layout file holds windows: top-level nodes, the first one at the bottom. Each node is placed in its parent (a window in the screen) and may hold children.

```ron
(
    windows: [
        (
            id: "status",
            pos: (8, 8),
            size: (250, 64),
            kind: Panel(skin: "thin_board"),
            children: [
                (id: "status.name", pos: (12, 7), size: (Fill(12), 16), kind: Text(text: "")),
                (id: "status.health", pos: (36, 28), size: (120, 8), kind: Gauge(skin: "health", value: 1.0)),
            ],
        ),
    ],
)
```

| Field | Meaning | Default |
|---|---|---|
| `id` | the name the application uses (`chat.input`); unique across all layouts of an interface | none |
| `anchor` | which point of the parent `pos` counts from: `TopLeft`, `Top`, `TopRight`, `Left`, `Center`, `Right`, `BottomLeft`, `Bottom`, `BottomRight` | `TopLeft` |
| `pos` | offset in pixels, **inwards** from the anchored edges: `BottomRight` with `(8, 4)` puts the node's bottom right corner 8 px left of and 4 px above the parent's. Along a centred axis it moves right or down from the centre | `(0, 0)` |
| `size` | width and height: a number of pixels, or `Fill(m)`: what is left of the parent after the offset and `m` pixels (centred: `m` short of both edges) | required |
| `hidden` | starts hidden, with its children | `false` |
| `click_through` | for windows: clicks pass to the game except on buttons and fields | `false` |
| `kind` | what it is (below) | required |
| `children` | nodes inside it, drawn after it in order | none |

Positions and sizes are rounded to whole pixels, so art and text stay sharp. Optional values are written plainly (`id: "chat"`, not `Some("chat")`). Unknown fields are errors that name the line.

### Kinds
| Kind | Draws | State the application sets |
|---|---|---|
| `Group` | nothing; holds children | |
| `Panel(skin)` | a panel skin over the node | |
| `Image(image)` | an image (game path of a `.tga`, `.dds` or `.sub`) stretched over the node; a name starting with `@` is a picture the application draws (`@character`, `@minimap`, `@atlas`) | the application hands in the picture |
| `Text(text, align, colour, size)` | one line, centred vertically; `align` `Left`, `Center`, `Right`; colour and size default to the skin file's | `set_text` |
| `Button(skin, text)` | a button skin's look (normal, under the pointer, pressed) and a centred label | `set_text`; `set_latched` keeps it drawn pressed (the open tab); clicks come back as events |
| `Edit(skin, max_chars, secret)` | a one-line field over an optional panel skin; `secret` shows `*` per character | `set_text`, `focus`, `take_text`; Enter and Escape come back as events |
| `Gauge(skin, value, from)` | a gauge skin filled by a value from 0 to 1, from the left (`from: Left`, the default) or from the bottom up (`from: Bottom`) | `set_value` |
| `Slider(bar, knob, value)` | a value from 0 to 1 the pointer sets: the `bar` panel skin over the node, the `knob` panel skin at its image's size (times the scale; a square as tall as the node when it is no image or has no art) with its middle travelling between the bar's ends, less half the knob on each side | `set_value`, `value`; the pointer's changes come back as events |
| `Log(max_lines)` | lines of text, newest at the bottom, wrapped to the node's width, oldest dropped past `max_lines`; the wheel scrolls back | `push_line` (text and colour) |
| `Slots(skin, columns, rows)` | a grid of item slots: `columns` × `rows` cells of equal size (default 1 × 1), the panel `skin` under each if given, and the items in them ([below](#slots)) | `set_slot`, `clear_slots`; presses and releases on cells come back as events |

`Ui::add` refuses a layout whose nodes name a missing skin, a skin of the wrong kind (a button needs a `Button` skin, a gauge a `Gauge` skin, panels, fields, slots and a slider's bar and knob a panel skin), an id already used, or a slot grid of no cells or more than 1,024.

- **Secret fields** (`secret: true`, for text not to show, such as a PIN; openm2's own screens have none since players sign in with providers): their text never leaves in an event (`Submitted` carries an empty text) or in debug output (`<redacted>`); the application moves it out with `take_text`, which empties the field, straight into a type that wipes it.
- **Looks that change:** `Ui::set_look(id, look)` draws a panel or a gauge with another look of the skin file, of its kind, and `Ui::look(id)` says which: the frames of an animation the application plays (the taskbar's globe).
- **Where nodes are:** `Ui::rect(id)` gives a node's place from the last layout, for pictures the application draws at that size. `Ui::is_hovered(id)` says whether the pointer is over a node, shown, in the topmost window there (a click-through window counts only where it takes the pointer): for a tooltip over a node that takes no input, such as a gauge.
- **Sizes that change:** `Ui::set_size(id, (width, height))` gives a node another size from the next layout on, its children placed by their anchors as always: a window that fits what it shows (`play`'s login board).
- **Pictures:** `image_paths` leaves out `@` names; the application draws those itself and hands them in with `set_sprite` like any image.
- **Images that come later:** `missing_images` names the images without a sprite yet (new item icons); the application loads each once and hands it in.
- **Labels:** `set_labels` puts texts at points of the screen, under every window (they belong to the game's picture: damage over heads, names over characters and over items on the ground). Each `Label` is centred on its point in its colour and size (0: the skin file's), over a shadow of the same alpha 1 px below right; its `lead`, if any, is text in a colour of its own just before it (a level before a name), the two centred together. Its `image`, if any, is a picture in place of both (a game master's mark): an image look of the skin at a size in layout pixels and an opacity, centred across on the point and standing where the text's line would, its bottom on the line's; while the look has no image, the text shows. Labels take no input; `label_at` says which is drawn over a point of the screen (the last there: over its picture, or over its line of text, lead and text together), so that `play` picks an item up by its name (`render.md`, "Items on the ground").

### Slots
A slot grid shows items the application puts in its cells (`set_slot(id, cell, item)`, cells counted row by row from 0). An item (`SlotItem`) has:

| Field | Meaning |
|---|---|
| `icon` | the icon's image, a game path like the layouts' images |
| `count` | drawn in the bottom right corner when more than 1 |
| `height` | cells it covers downwards in its column, at least 1 (a sword two cells tall); it sits in its top cell |
| `dimmed` | drawn darker (the item the pointer carries, left behind in its slot) |
| `cooldown` | cooling down (a skill used): what is left in thousandths and the whole seconds left; that part of the icon, from the top, is darkened and the seconds are drawn in its middle |

- **Drawing:** the skin under each cell, then each item's icon at its own size, centred in the cells it covers (smaller if it does not fit), cut at the grid's edge; without the icon's art, a brown square instead. The cell or item under the pointer is lit.
- **Cells and items:** a press, release or hover on a cell an item covers counts as on the item's top cell.
- **Events:** `SlotPressed { id, cell, button }` when the main or secondary button goes down on a cell; `SlotReleased { id, cell }` when the main button comes up over one, wherever it went down (dragging). The application decides what they mean.
- **The carried item:** `set_held(item)` draws an item centred on the pointer over every window.
- **Tooltips:** `set_tooltip(lines)` draws lines of coloured text in a box below right of the pointer, over every window, moved left or up to stay on the screen; not while an item is carried. `hovered_slot` tells the application which cell the pointer is over.

## The skin file
One file names the looks and the font (`crates/m2-cli/ui/skin.ron`, the built-in skin, naming the client's art, which `assets import` converts into an asset folder's skin in TOML, [`assets.md`](assets.md), "The interface's skin"; drawn without an asset folder, it shows its fallback colours):

| Field | Meaning |
|---|---|
| `font` | `families`: font family names, best first; the first one installed is used. `size`: pixels |
| `text_colour` | colour of text that names none |
| `tooltip` | the panel skin behind tooltips; a see-through black if not given |
| `skins` | looks by name |

| Skin | Draws | Fallback |
|---|---|---|
| `Colour(rgba)` | a flat colour | |
| `Frame(corners, edges, fill, fallback)` | a board: four corners (top left, top right, bottom left, bottom right) at their own size, four edges (top, bottom, left, right) repeated between them, the middle (`fill`) repeated inside the edges | the fallback colour over the whole node if any corner or edge is missing; over the middle if `fill` is missing or not given |
| `Bar(left, middle, right, fallback)` | a horizontal bar: the ends at their own width, the middle repeated, all at the node's height | the colour over the node |
| `Image(image, fallback)` | one image stretched over the node | the colour |
| `Tile(image, fallback)` | one image repeated over the node from its top left corner at its own size, the last ones cut (screen backgrounds) | the colour |
| `Button(normal, hover, down, text_colour, fallback)` | one of three images by the pointer: over it, pressed on it, or neither | the colour, ×1.25 under the pointer, ×0.7 pressed |
| `Gauge(slot, fill, inset, fallback)` | the `slot` skin over the node, then `fill` repeated from the left over the inset rectangle (left, top, right, bottom) as far as the value goes, keeping its width and taking the rectangle's height; for a gauge `from: Bottom`, repeated from the bottom up, keeping its height and taking the rectangle's width, the top copy showing its lower part | the colour instead of `fill` |

Colours are RGBA, 0–255, in gamma space like the art. Every image path is a game path in the user's client (an asset folder's skin: a PNG or a region of one); the application loads each and hands it to the interface as a sprite (an image and the part of it to draw). Paths it could not load, and empty ones, draw their fallback.

**The layouts name art only through looks,** so an asset folder's skin replaces all of it. Besides the looks of the layouts' boards, bars, buttons and gauges, the built-in skin has `Image` looks, see-through without the art, for the pictures the layouts show and the minimap draws with:

| Look | What |
|---|---|
| `minimap_frame` | the minimap's round frame (`minimap.sub`) |
| `key_1` to `key_4`, `key_f1` to `key_f4` | the quickslots' keys; `key_1` to `key_4` also number the bar's pages |
| `quick_page_up`, `quick_page_down`, `quick_page_board` | the quickslot bar's page buttons (`game/taskbar/quickslot_upbutton_01.sub` to `03`, `quickslot_downbutton_01.sub` to `03`, 9 × 5) and the board behind the page's number (`quickslot_button_board.sub`, 9 × 8) |
| `experience_frame` | the taskbar's experience frame with its four empty orbs (`exp_gauge.sub`); a see-through dark box without the art |
| `taskbar_base` | the taskbar's base: a glass globe and the troughs of health, mana and stamina (`gauge.sub`); a see-through dark box without the art |
| `globe_1` to `globe_17` | the globe glowing red, frame by frame (`rampage_01/00.sub` to `16.sub`) |
| `equipment`, `money` | the inventory's equipment backdrop and gold coin |
| `slider_bar`, `slider_knob` | a slider's bar (`game/windows/sliderbar.sub`, 175 × 12, spear tips at its ends) and its knob (`sliderbar_cursor.sub`, 53 × 12): the Settings window's volumes |
| `logo_left`, `logo_right` | the login screen's logo, unless the settings file names the game's own ([`client.md`](client.md)) |
| `flag_shinsoo`, `flag_chunjo`, `flag_jinno` | the empires' flags |
| `minimap_camera`, `minimap_player`, `minimap_dot`, `minimap_target_1`, `minimap_target_2` | what the minimap draws its picture with ("The minimap" below) |

**Loading art** (`interface.rs`): `.tga` and `.dds` files are drawn whole (DDS: the first level); a `.sub` file draws its rectangle of the image it names (version 1.0: in `d:/ymir work/ui/`, 2.0: beside it), cut to the image if it goes past its edge. An asset folder's PNG files are drawn whole, or their region, cut the same way. Each image file is uploaded once. What is missing is printed as a note.

## Input
The application passes pointer and keyboard input in; the interface answers whether it took it, so the game does not act on it too.

- **Pointer:** `pointer_moved` sets what is under the pointer: the topmost shown window there, and the deepest button, field, log or slot grid in it. A hidden node and its children take nothing. A click-through window counts only where it has one of those, so it never comes to the top over others (screen backgrounds, pictures).
- **Press:** a button under the pointer is pressed; a field takes the keyboard (caret at the end); anything else takes the keyboard away from any field; a slot cell reports `SlotPressed`; a slider takes the pointer's place along it (below). The window under the pointer comes to the top. The press is the interface's if it is over a window, except a click-through window, where only its buttons, fields and slots count.
- **Secondary press** (`press_secondary`): a slot cell reports `SlotPressed` with the secondary button and the window comes to the top; fields keep the keyboard. Taken as for the main button.
- **Release:** a button pressed and released under the pointer is clicked (`UiEvent::Clicked(id)`); over a slot cell, `SlotReleased`.
- **Sliders:** pressed, a slider takes the value where the pointer is, and follows it as it moves until the button comes up, off the slider too, held at its ends; each new value is `UiEvent::Changed(id)` (read it with `value`). Its window comes to the top as for any press. `set_value` sends no event.
- **Raising:** `raise(id)` brings a window to the top (a window just opened).
- **Wheel:** scrolls the log under the pointer, a line per notch.
- **Keys** (only while a field has the keyboard): typed text is inserted at the caret, control characters dropped, at most `max_chars` characters. Backspace and Delete remove a character, Left, Right, Home and End move the caret, Tab moves to the next shown field of the same window (round), Enter sends `UiEvent::Submitted { id, text }`, Escape takes the keyboard away and sends `UiEvent::Cancelled(id)`.
- **The clipboard**: `paste(text)` inserts text at the caret of the field with the keyboard on one line (line breaks and tabs as spaces, those at its ends left out), then as typed text; `copy()` gives that field's whole text, none from a secret field. `play`'s window and the launcher's call them for Ctrl+V or Shift+Insert and Ctrl+C or Ctrl+Insert while a field has the keyboard (⌘ for Ctrl on macOS; not with Alt, which AltGr is on Windows; the letter typed on a Latin layout, else the one at the key's place on a US keyboard), through the system's clipboard (arboard). A failed clipboard is a note; one with no text pastes nothing.
- Long text in a field scrolls left so the caret stays inside.

## Text
- **Fonts** come from the system; openm2 ships none. The skin file's first installed family is used (Tahoma first: the closest to the original client's look); without any of them, the system's sans-serif. Without any font at all, text takes no space and draws nothing.
- **Shaping and line breaking** are cosmic-text's (`Shaping::Advanced`); a log wraps at word boundaries, or inside a word that does not fit.
- **Lines** are 1.25 × the font size apart, rounded.
- **Glyphs** are rasterized (hinted) once into atlas pages of 1024 × 1024 RGBA: white with the glyph's coverage in alpha, so the vertex colour tints them; colour glyphs keep their colours. Up to 4 pages; when the last one is three-quarters full the atlas starts over at the next frame. A page's version goes up when it changes, and the renderer uploads it again.
- **Shaped texts** are kept for reuse; past 2,048 of them, those not drawn for 60 frames are dropped.

## Drawing
`Ui::draw(width, height)` lays every node out for the screen and returns the draw list and the glyph pages:

- **Order:** the labels, then windows from the bottom up; in each, a node, then its children in order; then the carried item or the tooltip.
- **Quads:** each is two triangles of vertices with a position in pixels, texture coordinates and a colour; quads that share a texture (the white texel for flat colour, an image, or a glyph page) and a clip rectangle form one batch.
- **Clipping:** logs and fields clip their text to their rectangle; a frame's middle and edges are clipped to the frame. Quads wholly outside their clip are left out.
- **Repeating:** repeated art (board middles and edges, bar middles, gauge fills) is drawn as one quad per copy, the last ones cut, so it works for sub-images packed in an atlas.

How the renderer draws the list is in [`render.md`](render.md), "Interface".

## Scale
Layouts are written in layout pixels. `Ui::set_scale` draws them that many times larger (0.5 to 4; 1 by default), for displays with more pixels to the inch:

- **What grows:** offsets, sizes and `Fill` margins (then rounded to whole pixels of the screen); the pieces of boards and bars, tiles and gauge patterns, gauge insets, item icons; font sizes; paddings, the caret, a label's shadow offset and the tooltip's place from the pointer.
- **Text** is shaped and rasterized at the scaled size, so it stays sharp. **Art** is stretched; the renderer smooths it or keeps its pixels (`render.md`, "Interface").
- **What does not:** what the application hands in or reads back is in the screen's pixels: the size to draw, the pointer, node rectangles, label points (a label's font size is in layout pixels).
- **In `play` and `view ui`:** a window uses the display's scale factor (Windows' display scaling: 1.25 at 125%, 2.5 at 250%), times the player's interface size in `play` (the Settings window, "The game menu"), and follows it to another display; the window's size is in the same units (1280 × 800 by default, so 3200 × 2000 pixels at 250%), so the windows take the same share of it on any display. Pictures (`--screenshot`) are drawn at 1. `--ui-scale` sets the scale for both, `--ui-filter smooth|sharp` how the art looks (smooth by default). a touchpad's scrolling counts 40 layout pixels a line.

## Play's screens
`openm2 play` holds every screen's layouts in one interface (`intro.ron`, `login.ron`, `servers.ron`, `select.ron`, `game.ron`, `inventory.ron`, `shop.ron`, `character.ron`, `menu.ron`) and shows one screen's windows at a time (`screens.rs`). The session's stage picks the screen; the player's clicks and keys become actions that `play` carries out (sign in, a `Command` for the session, back, quit).

| Screen | Windows | When |
|---|---|---|
| Login | `intro` (the red pattern, repeated), `login.logo` (the client's logo, or in its place the settings file's `logo`, as large as fits the client's 560 × 170, its sides kept, centred there), `servers` (the Servers board, below) until a server is joined, then `login` (`Sign in`: a status line, a wide button per provider of the server's, up to four, hidden until the server says which, Back and Quit; `play` fits the board's height to the buttons shown, Back and Quit 12 px under the last), `login.server` (the server joined: `Server NAME · ADDRESS`) | no session, or signing in (its buttons wait) |
| Select | `intro`, `select.model` (the character's picture, `@character`), `select` (dragon arrows, status line, Create, Play, Back), `select.info` (name, class, level, empire, guild, vitality, intelligence, strength, dexterity) | the character list is here ([`Stage::Selecting`](protocol/openm2/session.md)), or the game menu's Change character is under way (`Stage::Leaving`) |
| Create | `intro`, `select.model` (the chosen class's picture), `create` (name, class, gender, and on an account without an empire (no characters yet, or a game master's) the empire with its flag; status line, Create, Cancel) | the player chose Create, or the account has no characters yet |
| Loading | `intro`, `loading` (`Loading <name>` and the client's loading gauge) | a character is picked, until the map is drawn |
| Game | `chat`, `taskbar`, `minimap`; `inventory`, `character` and `atlas` when opened, `minimap.closed` once the minimap is closed, `target` while there is a target, `restart` while our character is dead, `shop` while an NPC's shop is open, `drop` and `trade` while they ask (below) | in the game |

- **Login:** the Servers board comes first (below). Players sign in only with the server's providers, or with the device's own key where the server takes keys. When a server is joined, `play` asks its auth server for its providers on a connection of its own (`SessionConfig::sign_in_options`; the status line says `Asking the server how to sign in...`), and shows a button for each with the provider's name from the server's providers file (the operator's words for players, such as `Sign in` or `Sign in with Google`). A server that takes devices' keys gets a **This device** button too, after its providers (on the last button when it has four): it signs in with the device's key ([`client.md`](client.md), "The device's key"), with no browser, the status line saying `Signing in...`. A server that offers none says `The server offers no way to sign in.`; one that cannot be reached says why. A button, or Enter for the first, signs in with that provider: the status line says `Opening your browser...`, then `Continue in your browser...` while the system's browser is open at the provider's page; the address is printed too, for when no browser opens (`--no-browser` only prints it). While it waits, Quit says Cancel: Cancel or Escape ends the sign-in and the buttons are back; a button starts it over, for a browser closed by mistake. When the browser comes back (or the sign-in ends), the window comes to the front, or flashes in the taskbar when the system keeps the browser in front: the provider's page cannot send the player back or close its tab, so it says the sign-in is done and the tab may be closed. What went wrong shows in the status line: `The provider did not sign you in.`, `The server could not reach the provider. Try again later.`, `The account is in the game already.`, any other refusal or error as a sentence. `--provider ID` (or `provider` in `openm2.toml`, [`client.md`](client.md)) signs in with one at once; the providers are asked for all the same, and their buttons come while it runs, so a sign-in whose browser was closed starts over, and one cancelled or failed starts again, from a button. Their coming doesn't change the status line then: it keeps `Continue in your browser...`, or why the sign-in failed. Quit (when no sign-in waits) or the title bar's X closes the window; Back, or Escape when no sign-in waits, goes back to the Servers board. Checked with `view ui --screen login --show login.provider.0 …` on a client's art: the client's wide buttons centred on the board under the status line, two and four of them, the waiting line alone, at scale 1 and 2.5.
- **The Servers board** (`servers.ron`, `server_list.rs`): what the login screen shows first. Its rows: the server `openm2.toml` names first, its name (`server_name`, else its host) after a ★, then the servers the player added ([`client.md`](client.md), "The player's servers"), eight a page with arrows and `1 / 2` under them. Each row is a flat button with three texts on it: the name, the address without the key, and how the server answered, `…` while it is asked: `42 ms · openm2-server 0.17.0` (the TCP connection and TLS handshake, then its `Welcome`'s name), `new key: Join to check it` (a self-signed certificate and no key held), `its key changed`, `another version` or `no answer`. Every server is asked when the board comes up, on Refresh and after a change, eight at once, each with a 5 s limit; answers of an earlier round are dropped. The row picked stays pressed: the one joined last when the board comes up, else the first. Join, Enter, or a click on the row picked joins it: Join says `...`, the status line `Reaching NAME...`, and the board waits while the server is checked (`servers::check`); then its Sign in board comes, or the status line says why not (the server's words as a sentence; `The key of NAME changed. If its owner says so, remove it and add it again.`). A self-signed server with no key held gets the question: `First time on NAME. Its key is`, the key in gold, `Compare it with the key its owner publishes. Trust it?`, with Trust (or Enter) and Cancel (or Escape: `Not joined: its key was not trusted.`). Add opens the dialog (`Add a server`: Name, Address with the line `host, host:AUTH:GAME, or a link with #sha256:…` under it, a status line, Save and Cancel, the title bar's X): Enter in Name goes to Address, Enter in Address or Save saves, Escape, Cancel or the X closes it. What is wrong is said in its status line (`Give both ports, host:AUTH:GAME, or the host alone for 11002 and 13001.`, `That server is listed already, as NAME.`, …). Edit opens it on the server picked, its address as its link; a new address without a key keeps the key when it is the same place. Remove asks (`Remove NAME from your servers?`, Remove or Keep). The first server can be neither edited nor removed (the status line says so); at 64 servers Add says the list is full. Escape closes the question or the dialog, else the window, as do Quit and the title bar's X. While the dialog or the question is open, the board's buttons wait. Checked with `view ui --screen login` and `--set`/`--show` of its ids on an import: four rows, the trust question and the dialog with a mistake, at scale 1.
- **Select:** the left and right arrow keys or the dragon swords go through the characters (round); Enter or Play picks the one shown (`Command::Select`); Create opens the create screen (not with four characters); Escape or Back logs out to the login screen. The arrows hide with one character, Play with none. The picture shows the character's class and gender in its base outfit with its hair, playing its wait motion (`render.md`, "Playing"). Stats and guild are the character list's.
- **Create:** the red arrows change class (warrior, ninja, sura, shaman), gender and, on an account without characters, the empire (Shinsoo, Chunjo, Jinno, with their flags), while the server says the account has none: before its first character, and always on a game master's account. Other accounts with characters keep their own ([`session.md`](protocol/openm2/session.md), "Characters"). Enter in the name field or Create makes it (`Command::Create`; the name is checked like the bot's: 1 to 24 ASCII letters and digits); the status line says `That name is taken.` when the server refuses it. Made, it shows on the select screen. Escape, Cancel or the X goes back to the select screen.
- **Changing character** (the game menu): the select screen comes at once, showing the character just left with the list as it was, the status line saying `Leaving the game...` and its buttons waiting; when the server's list comes (the character as just saved), the line clears and the screen is the player's. Escape or Back logs out from there, as always.
- **Loading:** the gauge stands at a quarter once a character is picked, half while the server loads it, and four fifths in the game while the map is read (the screen is drawn once before). Escape logs out. A warp that takes our character to another map brings the loading screen back while that map is read, as the original does; the game's windows close as when leaving the game.
- A session that ends (an error, or the server let us go) returns to the login screen, on the Sign in board of the server joined, with why. `--slot N` skips the select screen; `--provider ID` signs in at once to the settings' server, without the Servers board.

## The in-game interface
`openm2 play` draws `crates/m2-cli/ui/game.ron` over the world:

- **Taskbar** (`taskbar`, bottom centre, 699 × 48): its base (the look `taskbar_base`, `gauge.sub`: a glass globe and three troughs) standing above a thin board, then the experience, the quickslots with their page buttons, the messenger button ([below](#friends)), the character button ([below](#the-character-window)), the inventory button ([below](#the-inventory)) and the client's gears (`game/taskbar/system_button_01.sub` to `03`, the look `system_button`, 32 × 32), which open and close the game menu ([below](#the-game-menu)).
  - **The globe** (`taskbar.globe`): always glowing, the client's 17 red frames (`rampage_01/00.sub` to `16.sub`, the looks `globe_1` to `globe_17`) in turn, 100 ms each, by `play`'s clock (`Ui::set_look`). Clicking it does nothing.
  - **Health and mana** (`taskbar.health` and `taskbar.mana`: points 5 and 6, 7 and 8, [`protocol/openm2/entities.md`](protocol/openm2/entities.md), "Points"): gauges over the base's first two troughs, filled with the client's flowing fills (`pattern/hpgauge/01.tga` to `07.tga` and `pattern/spgauge/`, the looks `health_flow_1` to `health_flow_7` and `mana_flow_1` to `mana_flow_7`), 100 ms a frame. Over each, a tooltip: `Health: 4,062 / 8,124`, `Mana: 120 / 300`. The third, thin trough is the original's stamina, which openm2's server doesn't have: it stays empty.
  - Our character's name and level are not on the taskbar, as in the original: they show over its head, as every character's (`render.md`, "Names over characters"); the level is in the experience's tooltip too.
- **Experience** (`taskbar.experience`, on the taskbar after the base): our experience in the level (point 3) of what the next level needs (point 4) as four orbs over the client's frame (the look `experience_frame`), a quarter of the level each, each filled from the bottom up with the client's golden orb (`exp_gauge_point.sub`, the gauge look `experience`) as its quarter goes. Empty until the server sends the points; full at the highest level, where the server sends 0 and 0. Over the frame, a tooltip: `Level 12`, `Experience: 1,875 / 3,000 (62%)` (the percentage rounded down) and `1,125 more for level 13`; or `Level 99` and `Highest level`. The orbs, their quarters and the words are openm2's, read from the art.
- **Chat** (bottom left, 56 px up so the taskbar fits under it on narrow windows; click-through): the chat lines as they arrive, coloured by kind (talk light, information yellow, notices orange, party blue, guild green, command grey, shout salmon). Text from the server is shown as printable ASCII, anything else as `?`.
- **Chat line:** Enter opens it and gives it the keyboard; Enter sends what was typed as a talk line (`Command::Chat`) and closes it; Escape closes it. `/w NAME TEXT` and `/whisper NAME TEXT` (the command in any case) are whispers instead ([below](#whispers)): `Command::Whisper`; `/w NAME` alone opens that conversation; `/w` without a name, or with one no character could have, gets the line `Whisper to whom? /w NAME TEXT`. Typed text goes out as it is, in UTF-8; the server drops lines with control characters. An empty line sends nothing.
- **Affects** (top left, 384 × 32; click-through but for its icons' tooltips): [below](#the-affects-bar).
- **Target** (top centre; click-through): the entity we chose (`World::target`), until it leaves: its name (the server's for players and NPCs; for monsters, which the server sends no name for, the locale's monster table's, `m2_assets::items::read_monsters` or an asset folder's `[[monster]]` names, decoded like item names; else its kind and number) and its health from the server's TargetHealth as a red gauge (full until the server says, empty once it died).
- **Restart** (centre, while our character is dead): `You died.` and two buttons, Restart here and Restart in town (`Command::Restart`, [`protocol/openm2/combat.md`](protocol/openm2/combat.md), "Dying"). It shows at once; it goes when the server brings us back.
- **Drop question** (centre, when an item of the bag is let go of over the world): `Drop Sword+0?`, with the count for a stack (`Drop 5 Red Potion (S)?`), and two buttons: Drop (`Command::Drop`, the whole stack) and Cancel. Escape cancels too. It goes when the item leaves its slot, and Drop sends nothing then.
- **Minimap** (top right) and **atlas** (centre, when opened): [below](#the-minimap).
- **Shop** (left of the inventory) and its **question** (centre): [below](#the-shop).
- **The character window** (left; C, K or the taskbar's character button) with our stats and our **skills**: [below](#the-character-window) and [below](#skills).
- **Game menu** and **Settings** (centre, Escape or the taskbar's gears): [below](#the-game-menu).
- **Names** over the characters near ours, players and monsters with their level (`render.md`, "Names over characters"), and **names** over the items on the ground, as labels (`render.md`, "Items on the ground"). Damage numbers are effects in the world (`render.md`, "Our fighting").
- **Walking:** a left click on the ground walks there; W, A, S and D or the arrow keys run away from the camera, towards it and to its sides (`render.md`, "Walking with the keys").
- **Fighting:** a left press on a monster goes for it, Space attacks while held (`render.md`, "Our fighting"). A left press on an NPC walks up to it and talks to it ([below](#the-shop)); on another player, chooses it (the target window), for the skills used on others ([below](#skills)). A warp or a goto is drawn as nothing and is never picked: a press on its point is a press on the ground there, and our character walks to it.
- While a field has the keyboard, keys are the interface's (F types an `f`, it does not switch the fog; W, A, S and D type letters, they do not walk; Space types a space; Escape closes the chat line, it does not open the game menu). Clicks on the taskbar or the chat line do not walk.
- `--no-interface` draws the world alone in the game (the screens before it stay).

## The minimap
`minimap` in `game.ron`: the map around our character, north up, in the client's round frame (the look `minimap_frame`: `d:/ymir work/ui/minimap/minimap.sub`, 136 × 137), with its buttons over the ones drawn in the frame (atlas, close, zoom out, zoom in; their places measured on the art). `minimap.rs` draws the picture on the CPU, 128 layout pixels across (at the interface's scale in screen pixels), and hands it in as `@minimap`; the frame's ring covers its edge. The art it draws with is the images of the skin's looks below (`minimap_camera`, `minimap_dot`, `minimap_target_1` and `_2`, `minimap_player`).

- **The map:** the areas' `minimap.dds` pictures ([`formats/map.md`](formats/map.md); an asset folder's `minimap.png`), read the first time they show, sampled between texels inside each area; black off the map and where an area has none. Our character's map position is the middle.
- **Zoom:** 0.5, 1, 2 or 4 layout pixels a metre (128, 64, 32 or 16 m from edge to edge), 1 to start (a texel a pixel); the + and − buttons step through them. openm2's steps.
- **The camera's view:** the client's `minimap_camera.dds`, a see-through dark disc with the view cut out of it, over the whole picture, turned so the view points where the camera looks: what the camera sees is lighter.
- **Marks:** each entity near us that is alive, the client's `whitemark.sub` (4 × 4) tinted by kind: monsters red, NPCs yellow, metin stones violet, other players blue, warps (portals) green; doors, buildings and horses get none (openm2's colours). Our target blinks between `targetmark01.sub` (white) and `targetmark02.sub` (red), half a second each. Marks are drawn where the entities are drawn, and only inside the circle.
- **Our arrow:** `playermark.sub` (pointing up) in the middle, turned to where our character faces.
- **Cut** to the circle that fills the picture, with an edge a pixel soft (openm2's own; the client's `minimap_image_filter.dds` is the same circle, hard edged).
- **Close** (X): the minimap gives way to the client's small map button (`minimap_open_default.sub`, 32 × 32) in its corner, which opens it again. Entering the game opens it.
- The picture is drawn again only when what it shows moved by a pixel (our place, the marks, the camera by a degree, the zoom, the size), so a still screen costs nothing.
- Without the client's art: the map alone, plain squares for marks, a white one for us.

## The atlas
`atlas` in `game.ron`, opened and closed by the minimap's M button or the M key, closed by its X or Escape: a board with the map's folder name and the map's atlas picture ([`formats/map.md`](formats/map.md), "Atlas pictures") at its own size (times the interface's scale), centred in a 256 × 256 box (smaller if it does not fit), drawn on the CPU as `@atlas`.

- **Marks:** the points of the locale's point file ("Atlas points"; an asset folder's `map.toml` marks): NPCs yellow, warps green, in the minimap's dots; our arrow where we are, turned to where we face. The points' names are Korean in every locale of the client, so openm2 shows none. Monsters and other players are not marked.
- A map without an atlas picture (most dungeons) shows an empty box.

## The inventory
`crates/m2-cli/ui/inventory.ron`, filled in by `inventory.rs` from the session's items and quickslots ([`protocol/openm2/inventory.md`](protocol/openm2/inventory.md)). The server has the last word: a click sends a request, and the windows change when the server's answer changes the world.

- **Quickslot bar** (on the `taskbar`, "The in-game interface"): 8 slots for keys 1 to 4 and F1 to F4 (their labels from the client's taskbar art), and a button that opens the inventory. **Four pages** of them (quickslots 0 to 7, 8 to 15, 16 to 23, 24 to 31), as the original's taskbar: right of the slots a button for the page before (`taskbar.page_up`, the up arrow) over the page's number (`taskbar.page`, the client's digits 1 to 4 on their board) over one for the page after (`taskbar.page_down`), going round from the last page to the first and back; Shift and a digit key 1 to 4 (by its place on the keyboard) choose a page; while Alt is held the bar shows the page after the chosen one (the original's, which shows names with it). The keys, the slots, what is put on them and their tooltips are the page's shown. Every game starts on the first page, after a warp to another map too (the original's game window starts again). Not as the original: no tooltips on the page buttons (no button of `play` has one). A quickslot holding an item shows the item in its bag slot, with its count; one whose item moved away (the server does not follow it, [`protocol/openm2/inventory.md`](protocol/openm2/inventory.md), "Quickslots") shows nothing. One holding a skill shows the skill's icon at our grade, and its name over it ([below](#skills)).
- **Inventory** (`inventory`, right, opened with I or the taskbar's button, closed with I, its X or Escape): the client's equipment backdrop (`equipment_bg_without_ring.tga`) with a slot over each place drawn for an item, the bag's two pages of 5 × 9 behind tabs I and II, and our gold (point 11) with commas.
- **Worn slots** over the backdrop, as openm2 reads the art (the silhouettes drawn in it; not confirmed): weapon (the left column, three cells tall), helmet (the head), armour (the body, two cells), shoes (the feet), the two unique items (the dragon medallions, top and bottom right), shield, bracelet, arrows, earrings, necklace. Three places of the backdrop (the knees and the two orange-framed cells at the bottom) hold nothing yet.
- **Icons** come from the item list of the locale `locale.cfg` names, sizes (cells tall) and names from its item table ([`formats/script.md`](formats/script.md), "Item list" and "Locale setting"), or from an asset folder's item table ([`assets.md`](assets.md), "Items"). New icons are loaded when they first show.
- **Tooltips:** over an item, its name (the item table's, UTF-8), how many when more than one, and its level limit; over an empty worn slot, what it is for.

| Doing | What happens |
|---|---|
| left click on an item (bag, worn, quickslot) | it is picked up: drawn at the pointer, dimmed in its slot |
| left click on a slot while carrying, or letting go there after dragging | a bag or worn item: a move (`Command::MoveItem`, the whole stack), which wears it or takes it off when one end is a worn slot; onto a quickslot: the quickslot holds that bag slot (`SetQuickSlot`); a quickslot onto another: they swap (`SwapQuickSlots`); put back where it came from, or anything else: nothing |
| left click on the world while carrying | a quickslot is emptied (`ClearQuickSlot`); an item of the bag is put back and the drop question asks whether to drop it ("The in-game interface"); a worn item is put back; no walk |
| right click on an item | it is used (`UseItem`): worn, taken off, or used up as the server decides; a quickslot uses its item |
| 1 to 4, F1 to F4 | use the quickslots of the page shown: an item, or a skill |
| Shift and 1 to 4 | show quickslot page 1 to 4 |
| Alt (held) | the quickslot bar shows the next page |
| Escape | closes the game menu or the Settings window, else the shop's question, else answers No to the friend question, else the drop question or the one before forgetting, or puts back what the pointer carries, else closes the shop, else the teacher, else the stable, else a soul stone's window, else the whisper window, else the friends list (its name field first), else the inventory, else the character window, else the atlas, else opens the game menu |
| C, K | open the character window on its Character or Skills tab, or close it ([The character window](#the-character-window)) |

Letting go on the cell it was picked up from keeps it on the pointer (click, then click where it goes). Right click while carrying puts it back. Leaving the game closes the inventory and forgets what was carried.

## The shop
`crates/m2-cli/ui/shop.ron`, filled in by `shop.rs` from `World::shop` ([`protocol/openm2/shops.md`](protocol/openm2/shops.md)). As with the inventory, the server has the last word: the bag and our gold change when its answer comes.

- **Talking:** a left press on an NPC walks up to 2.5 m of it (a little short of where it stands, around walls, as for items on the ground, `render.md`) and talks to it (`Command::Talk`); a click elsewhere, a monster or an item gives up. An NPC with a shop opens it, a teacher its window ([below](#teachers)); others say nothing yet (quests are not handled).
- **The shop** (`shop`, left of the inventory, their tops level; the inventory opens with it): the NPC's name in the title bar (as the target window names it), the shop's 40 places as a grid 5 wide and 8 high, each item at its place, tall ones covering the places under them, with their counts. Over an item: its name, how many when more than one, and `Price: 1,000 gold` (the server's price, for the whole stack).
- **Buying:** a left or right click on an item asks `Buy 5 Red Potion (S) for 50 gold?` (`trade`, centre: Buy and Cancel). Buy sends `Command::Buy` if the item is still in its place. When it costs more than our gold, or no slot of the bag has room for its height (on one page, as the server places it), the chat says `You do not have enough gold.` or `There is no room in your bag.` instead, in the colour of the server's information lines; the server's own refusals say the same.
- **Selling:** an item of the bag carried onto the shop (put down on it, or dragged and let go over it) is put back and the question asks `Sell Sword+0 for 100 gold?`, or for a stack `Sell 5 Red Potion (S) at 30 gold each?` (the item table's sell price; what the shop pays is the server's: openm2's pays it for each, [`protocol/openm2/shops.md`](protocol/openm2/shops.md), "Selling"). Sell sends `Command::Sell`, the whole stack. A worn item: `Take it off before selling it.`
- **Closing:** the shop's X or Escape (after the question and what the pointer carries) closes it (`Command::CloseShop`); so does walking farther than 10 m from the NPC, or the NPC leaving. The question goes with the shop, or when what it asks about leaves its place.
- The distances are openm2's choices, inside the server's: it talks within 5 m and trades within 12 m ([`protocol/openm2/shops.md`](protocol/openm2/shops.md)).

## Teachers
`crates/m2-cli/ui/teacher.ron`, filled in by `teacher.rs` from `World::teacher` ([`protocol/openm2/skills.md`](protocol/openm2/skills.md), "Teachers"). The server has the last word: the group comes with its answer.

- **The window** (`teacher`, centre, clear of the character window and the inventory): the NPC's name in the title bar, a line of what it says, and the six skills of the group it teaches, a row each with the icon and the name (a skill the client's descriptions leave out is an empty row). Over an icon: the name, the description and its lines. It uses looks every import has (the board, the title bar, slots, the large button).
- **What it says:** `I teach only shamans.` for another class, `You have learned these skills.` for our group, `You have chosen other skills already.` for the other, `Come back at level 5.` below the server's level, else `I can teach you these skills.` and **Learn** (`Command::LearnSkillGroup`), which shows only then. The group learned opens the character window on its Skills tab, and the line says so.
- **Closing:** its X or Escape (after the shop) closes it (`Command::CloseTeacher`); so do walking farther than 10 m from the NPC, the NPC leaving, a shop opening, and leaving the game. A teacher opening closes the shop.
- Checked by `teacher.rs`'s tests (every line, Learn, the rows by number without descriptions) and `screens.rs`'s (Learn, the Skills tab, the X, Escape); the looks with a client's art with `view ui --screen game --teacher 0:1 --point 1=5 --skill-group 0`.

## Training
Decision 039 ([`protocol/openm2/skills.md`](protocol/openm2/skills.md), "Training"). Books and the items that help need nothing of their own: a right click on them (or their quickslot) sends UseItem, and the server answers with a line.
- **A soul stone's window** (`crates/m2-cli/ui/choose_skill.ron`, filled in by `choose_skill.rs` from `World::skill_choice`; centre, where the teacher's stands): the item's name in the title bar, `Which skill do you train?`, and a row for each skill it offers (at most six): the icon at our grade, the name, the level, and Train (`Command::TrainSkill`). Over an icon: the name, the level, the description and its lines. Its X or Escape (after the teacher) closes it (`Command::CloseSkillChoice`); so do training, the item's place changing and leaving the game. It uses looks every import has (the board, the title bar, slots, the large button), so the import's number stays 5.
- **Before forgetting:** a book that forgets (the item table's kind 22) used from the bag or a quickslot asks `Forget every skill and choose again?` (`forget`, where the drop question stands): Forget sends UseItem, Cancel or Escape nothing; the question goes with its book.
- **The wait:** the Skills tab's tooltip over a master's skill says `Ready for a book` or `Next book in 1 h 5 min`, a grand master's `Ready for a soul stone` or `Next soul stone in 42 min`: `next_read` against the computer's clock (a clock that is off shows another wait; the server's answer holds).
- Checked by `choose_skill.rs`'s tests (the rows, Train, closing), `inventory.rs`'s (the question, from the bag and a quickslot, Cancel, Escape, the book gone), `skills.rs`'s (the waits) and `screens.rs`'s (Train, the X, Escape); the looks with `view ui --screen game --choose-skill 1,2 --skill 1=30 --skill 2=39` and `--show forget`.

## Stables
`crates/m2-cli/ui/stable.ron`, filled in by `stable.rs` from `World::stable` and `World::mount` ([`protocol/openm2/mounts.md`](protocol/openm2/mounts.md)).

- **The window** (`stable`, where a teacher's goes): the NPC's name in the title bar; our mount (`Your mount: level N, stamina A/B` or `You have no mount yet.`); the next level (`Level N: speed S, stamina T`), what it needs (`Needs level R, G gold.`, or `Free of charge.`) and up to four lines of the items it costs (`2 × NAME`); **Get a mount** (the first) or **Train** (`Command::TrainMount`), not past the last level (`It cannot be trained any further.`).
- **Closing:** its X or Escape (after the teacher) closes it (`Command::CloseStable`); so do the NPC leaving, a shop or teacher opening, and leaving the game.
- **The stamina gauge** (`taskbar.stamina`): our mount's stamina in the taskbar base's thin trough, shown while we have a mount; over it, `Mount's stamina: A / B`.
- **R** gets on our mount, or off it (`Command::Ride`).
- Checked by `stable.rs`'s tests (every line, Train's text) and with `view ui --screen game --stable 2 --mount 18/30 --item 0=VNUM` on a client's art.

## Upgrades
`crates/m2-cli/ui/upgrade.ron`, filled in by `upgrade.rs` from `World::upgrade` ([`protocol/openm2/upgrades.md`](protocol/openm2/upgrades.md)). The server has the last word: the item changes with its answer.

- **At a smith:** an item of the bag let go over a living NPC (put down on it, or dragged and let go over it) goes back to its place, and our character walks up to 2.5 m of the NPC as for talking and asks it (`Command::GiveItem`); a smith offers, another NPC says so in a line.
- **With a scroll:** an item of the bag put onto a different item of the bag (on any of the places a tall item covers) asks the server (`Command::UseItemOnItem`): a scroll offers. Onto the same item it still stacks, onto an empty place it moves.
- **The window** (`upgrade`, where a stable's goes): the smith's name, or the scroll's, in the title bar; what the item becomes, its icon as tall as it is; the item now, `becomes NAME`, `Chance: 50%`; `1200 gold (you have 5000)` or `No gold needed.`; up to four lines of the items it costs, `2 × NAME (you have 3)`; what a failure does (`If it fails, the item is destroyed.`, `… goes down a level.`, `… stays as it is.`); **Upgrade** (`Command::Upgrade`, which closes it) and **Cancel**. It uses looks every import has (the board, the title bar, slots, the large button); the import's number went to 8 with the upgraded items' shine and glows (`render.md`).
- **Closing:** its X, Cancel or Escape (after the stable) closes it (`Command::CloseUpgrade`); so do walking farther than 10 m from the smith, the smith leaving, the item's or the scroll's place changing, a shop, teacher or stable opening, and leaving the game.
- Checked by `upgrade.rs`'s tests (every line), `inventory.rs`'s (onto another item, onto a tall item's lower place, onto itself, what the pointer carries for an NPC) and `shop.rs`'s (the walk up, then GiveItem); the looks with `view ui --screen game --item 0=VNUM --item 7=VNUMx5 --upgrade NEXT --show inventory` on the test world's art.

## Fishing
Decision 048 ([`protocol/openm2/fishing.md`](protocol/openm2/fishing.md); the server's rules in [`server/world.md`](server/world.md), "Fishing").
- **Holding a pole:** a worn fishing pole (item type 13) is drawn in the right hand (the warrior's `equip_right_hand`, the others' `equip_right`), and its holder stands, walks and runs with its class's `fishing/` motions (`wait`, `walk`, `run`); on a mount, the plain riding ones. Others are drawn the same from what they wear.
- **Casting:** Space, with a pole worn and on foot (on a mount Space attacks as without one), looks for water **6 m** out on the map's attributes (the water flag, a mask a metre a square, `walls.rs`): straight ahead, then 10° to either side in turn up to 180°, and asks the server to cast that way (`Command::Cast`); with none, the notice `There is no water close enough to fish.`. The fighter, walker and talker stop.
- **Reeling in:** with our line out, Space, a move key or a click on the ground sends `Command::ReelIn` instead (a move key or a click at most every 0.5 s), and our character doesn't move.
- **Each fisher's steps** (anyone's, from Fishing): the cast plays `throw` once, then `fishing_wait` round and round while the line is out (moving drops back to the pole's own); the bite plays `fishing_react` once, after `throw` if it comes during it, and shows the fish over the fisher's head (`[effects] fish_bite`, at the top of its model, following it); a catch plays `fishing_catch`, a loss `fishing_fail`, a reel-in before a bite `fishing_cancel`, then the pole's standing. Hits don't interrupt them. The motions' fishing effects (the float, its ripples, the splash) play at the float ([`render.md`](render.md), "Effects").
- **Skills** are refused with a pole in hand: `Not with a fishing pole in hand.`
- **The pole's tooltip:** its name and level, `Points: N / M`, `Bait: NAME` or `No bait on the hook`, and, full, `Full: the Fisherman can better it.`
- **The Fisherman** is the upgrade window ("Upgrades"): a pole dropped onto him offers the next pole, no gold, its chance.
- **Fish, the fire, boosts:** used from the bag as any item (`UseItem`); an item dropped onto a fire NPC is GiveItem, as onto a smith. What items raise shows in the affects bar ("The affects bar").
- Checked by `play.rs`'s tests (the search for water, `ahead`), `inventory.rs`'s (the tooltip), the assets' grip test, and in the window on a local server.

## Whispers
`crates/m2-cli/ui/whisper.ron`, filled in by `whisper.rs` ([`protocol/openm2/chat.md`](protocol/openm2/chat.md), "Whispers").
- **Envelopes** (`whispers`, left edge, click-through but for them): one for each conversation the session keeps, eight at most, in the order they began; the client's `game/windows/btn_mail_up.sub` (the look `envelope`, 32 × 32; the client has one picture for it). A conversation with lines not seen yet has a red mark on its envelope (`whisper.unread0..7`, the look `unread`), shown and hidden every half second. The open conversation's envelope stays pressed.
- **The window** (`whisper`, beside the envelopes): one, showing the conversation picked: its player's name as the title, its lines (theirs in pink, ours lighter, the server's refusals in the info colour: `Bob is not in the game.`, `Too many whispers: wait a little.`), our line, End and the X. An envelope opens its conversation, or closes it when it is the one shown; `/w NAME` opens one before it has a line, and the name takes the server's spelling with the first. Enter in its line whispers (`Command::Whisper`) and keeps the keyboard; Escape in the line only gives the keyboard back; End forgets the conversation (`Command::ForgetWhispers`: its envelope goes); the X or Escape closes the window, the conversation stays.
- A refusal of a whisper whose conversation is not shown is a line in the chat too.
- Leaving the game forgets them all (they were the character's).

## Friends
`crates/m2-cli/ui/messenger.ron`, filled in by `messenger.rs` ([`protocol/openm2/friends.md`](protocol/openm2/friends.md)).
- **Opening it:** the taskbar's messenger button (`taskbar.messenger`, the client's `game/taskbar/community_button_01.sub` to `03`, the look `messenger_button`, 32 × 32, left of the character button) opens and closes it; so do its X and Escape.
- **The list** (`messenger`, left of the inventory): a friend a row, ten a page (the client's wide button, `xlarge_button`), by name; a mark for whether it is in the game (`game/windows/messenger_list_online.sub` and `messenger_list_offline.sub`, the looks `friend_online` and `friend_offline`, 15 × 16); `No friends yet.` without any; the arrows and `1 / 2` with more than a page. A click picks a row, which stays pressed.
- **Its buttons** (the client's `messenger_whisper`, `messenger_delete` and `messenger_add_friend` pictures, 21 × 18): Whisper opens the conversation with the friend picked; Remove asks `End your friendship with Bob?` first; Add shows a field for a name, and Enter asks that player (`Command::AskFriend`); Escape takes the field away.
- **The question** (`friend_question`, centre): `Bob asks to be your friend.` for the oldest request waiting, Yes and No (`Command::AnswerFriend`); Escape answers No. A request that ends unanswered takes its question away; the next comes.
- **Lines in the chat:** the server's notices (`You asked Bob to be your friend.`, `Bob is not in the game.`, `Bob is your friend already.`, `Your friends list is full.`, `The friends list of Bob is full.`, `Too many friend requests: wait a little.`, `Bob declined your friend request.`, `Bob did not answer your friend request.`), and `Bob is now your friend.` and `Bob is no longer your friend.`. Friends coming and going show only in the list.
- Checked by `whisper.rs`'s, `messenger.rs`'s and `screens.rs`'s tests (`/w`, the envelopes and their marks, the window's lines, End, the rows and pages, the buttons, the question, Escape); the looks with `view ui --screen game --whisper NAME=TEXT --whisper-to NAME=TEXT --open-whisper NAME --friend NAME[:online] --friend-request NAME --show messenger`.

## The character window
`crates/m2-cli/ui/character.ron` (`character`, left, 253 × 376, the original's width), filled in by `character.rs` (the window and its Character tab) and `skills.rs` (its Skills tab, [below](#skills)). Its words and keys are openm2's; the original has a character window with tabs, by what is known, and its client has art for it, which `play` uses.

- **Opening it:** C opens it on the Character tab and K on the Skills tab; the same key again closes it, the other switches tabs. The taskbar's character button (`taskbar.character`, the client's `game/taskbar/character_button_01.sub` to `03`, the look `character_button`, 32 × 32, left of the inventory button) opens it on the tab shown last, or closes it; so do its X and Escape (after the inventory, before the atlas). Leaving the game closes it and goes back to the Character tab.
- **The tabs** at its bottom (`character.tab_status` `Character` and `character.tab_skills` `Skills`, the client's `game/windows/tab_button_large_01.sub` to `03`, the look `tab_large`, 78 × 19): the open one's is held down (`set_latched`), and the title says which it is.
- **The Character tab** (`character.status`), from `World::me` and our points ([`protocol/openm2/entities.md`](protocol/openm2/entities.md), "Points"):
  - our job's face (the client's `game/windows/face_warrior.sub`, `face_assassin.sub`, `face_sura.sub`, `face_shaman.sub`; the looks `face_warrior`, `face_ninja`, `face_sura`, `face_shaman`, 43 × 43), our name, `Ninja, level 12`, and `Experience: 75%` of the level (rounded down; `Highest level` where the server sends 0 and 0);
  - Strength, Vitality, Dexterity and Intelligence (points 12 to 15), each with a + (the look `plus`) while there are status points;
  - `Status points: 3` (point 26);
  - what they make: maximum health and mana (6, 8), attack (the attack grade, 18), defence (16), attack and movement speed (17, 19), with commas between thousands.
- **Spending a status point:** a + sends `Command::RaiseStatus`, a RaiseStatus message ([`protocol/openm2/entities.md`](protocol/openm2/entities.md), "Spending a status point"). The server has the last word: the stat changes when its PointsChanged comes, and refusals come as chat lines.
- `play --screenshot --character` and `--skills` open it on either tab; `view ui --screen game --show character` (or `--show skills`) draws it, with `--point POINT=VALUE` for our points.

## Skills
The character window's Skills tab (`skills` in `crates/m2-cli/ui/character.ron`), filled in by `skills.rs` from `World::skill_group`, `World::skills` and our skill points (point 28) ([`protocol/openm2/skills.md`](protocol/openm2/skills.md)), with the names and icons of the locale's skill descriptions ([`formats/script.md`](formats/script.md), "Skill descriptions") or an asset folder's skills file ([`assets.md`](assets.md), "Skills"). The server has the last word: a point spent changes the level when its answer comes.

- **The tab** (`skills`; K opens the character window on it), laid out as the original client's skill page (its scripts), with its art (looks `horizontal_bar`, `skill_board`, `skill_points`, `skill_cover`, `skill_group`):
  - **The bar** (`horizontal_bar`, 223 wide, its node 32 tall: the client's pictures are 32 × 32 with the bar in their top 17 rows, drawn at their own height as the original draws them): our group's name in the original's gold (`skills.group`, the skills file's `[[group]]` name, else `Group 1`), or, before a group is chosen, a button for each group (`skills.group1`, `skills.group2`, the look `skill_group`) that shows its skills, the one shown held down; and the skill points left in the client's label (`skills.points`, `available` in the English client's art).
  - **The board** (`skill_board`, 227 × 152): the six skills of the group, in two columns of three rows (skill 1 top left, 2 top right, …), each a row of three icons 37 px apart, one a grade: normal, master, grand master (`skills.icon{row}_{grade}`, the grade's icon). The grade reached is uncovered, with the level in its bottom right corner (`skills.mark{row}_{grade}`: `5`, a master's `m3`, a grand master's `g1`, a perfect master's `p` on the last; none before it is learned); the other grades are covered by the client's dark slot cover (`skills.cover{row}_{grade}`), and every grade before a group is chosen. A + (`skills.up{row}`) sits on the first icon's bottom left while there are points and the skill is not a master yet (masters take books, not points). A skill the client's descriptions leave out (the warrior's and ninja's sixth) leaves its row empty.
  - **Support skills:** the bar with its title and two rows of six empty slots, as the original's (openm2 has no support skills yet; the original hides their points in the European clients).
  - **Over an icon:** at the grade reached, the name, the level (`Not learned`, `Level 5`, `Master M3`, `Grand master G1`, `Perfect master`), its mana (`Mana: 64`, at our level of it, at least level 1, worked out as cooldowns are), a master's or grand master's next training ("Training"), the description and its lines; at another grade, the name only, as the original.
  - **Not as the original:** the level is written in the interface's font, not the client's digit pictures (`ui/game/taskbar/0.sub` to `9.sub`, `m`, `g`, `p`); the window keeps openm2's two tabs and its title (`Skills`), not the original's four tab pictures; the horse's skills page is not there.
- **Spending a point:** the + sends `Command::SkillUp`, a RaiseSkill message ([`protocol/openm2/skills.md`](protocol/openm2/skills.md), "Spending a skill point").
- **Onto a quickslot:** a learned skill's icon at its grade picked up (a left click, or a drag) and put on a quickslot (a click on it, or letting go over it) is set there (`SetQuickSlot`, kind 2, the skill's number). Elsewhere it is put back.
- **Using a skill:** a right click on its icon at its grade in the window, or its quickslot's key or a right click on the quickslot. `play` checks first, and the chat says why not, in the colour of the server's information lines:
  - not learned: `You have not learned this skill yet.`;
  - riding: `Get off your mount to use this skill.` (a job's skills are used on foot; Dash doesn't start its run);
  - a skill with `WEAPON_LIMITATION` and none of its weapons in hand (the weapon's subtype against the description's weapon words; `DOUBLE_SWORD` taken as daggers, openm2's reading): `This skill needs a weapon.` or `This skill cannot be used with this weapon.`;
  - a skill with `NEED_TARGET` (and not `CAN_USE_FOR_ME`) without a target we can fight: `This skill needs a target.`;
  - a skill cooling down: `The skill is ready again in 7 s.`;
  - its target farther than the skill's range (the skill table's, in an asset folder's skills file `target_range`; else 5 m, a hit's reach): `Too far away.`;
  - its mana (the skill table's formula, worked out as cooldowns are) above ours (point 7): `You do not have enough mana.`
  Then the character faces the target, if there is one, and plays the skill's motion where it stands (`Command::UseSkill`; `render.md`, "Skills"). A skill used on others (`ONLY_FOR_ALLIANCE`, the shaman's) goes to the player we chose (a left press on it), else to us; another that can be used on oneself (`CAN_USE_FOR_ME`) to our character. A charging skill (`CHARGE_ATTACK`, Dash) on a foe out of its reach but within 15 m first runs up to 1.5 m short of it (`Command::MoveTo`), and is used once within 2.5 m; a click, a movement key, a press on something else or 3 s give it up (openm2's way). No other skill starts while the motion plays; a held attack goes on after it.
- **Cooldowns:** a skill used starts cooling down for its cooldown formula in the locale's skill table ([`formats/script.md`](formats/script.md), "Skill table"), worked out with openm2's skill power at our level of it (`k` = level × 0.025, `m2_content::SKILL_POWER`) and our points, the same rule openm2's server uses with the same formulas when they were imported from this client ([`server/world.md`](server/world.md), "Skills"). While it cools down, its icon in the window and on its quickslots is darkened by what is left, from the top, with the seconds left on it, and using it is refused with a line (above). No protocol packet carries a cooldown: the client works it out, as the client's own skill files (`CoolTimeFormula`) suggest the original does. The server keeps its own and refuses an early use whatever the client shows. Without a skill table, no cooldown shows.
- **Not checked:** what the client can't work out of a mana cost (`atk`, the weapon's roll: 0 here, so the server may still refuse with its line), passive skills.

## The affects bar
`affects` in `crates/m2-cli/ui/game.ron` (top left, 8 px in), filled in by `affects.rs` from `World::affects` (`Affects`, [`protocol/openm2/skills.md`](protocol/openm2/skills.md), "What skills raise"): what skills raise on our character for now, ours and others' on us.
- **Icons:** a slot grid of 12 (`affects.icons`, the look `affect_slot`: nothing under them), a skill each in the server's order: its icon at our grade of it (normal for another's), with the seconds left on it, the longest of its points', in white over a shadow (a slot's cooldown with nothing darkened, `Cooldown { left: 0, .. }`); a skill that stays on shows none.
- **Tooltip:** the skill's name, a line for each point (`Attack speed +25%`, `Movement speed -10%`, `Attack +40`, `Defence +52`, `Damage taken -20%`, `Reflects 30% of melee damage`, `Critical hits 15%`, `Dodges 30% of shots`, `10% of damage dealt back as health`, `Mana shield: 25% mana a health point`, `Cooldowns -36%`, `Hidden from monsters`, `Strength +10`, `Dexterity +10`), then `42 s left` or `On until used again`.
- **Items** (`World::item_boosts` from ItemBoosts): after the skills, each item used that raises a point for a while, its icon with its seconds left; its tooltip the item's name, what it raises and `600 s left`.
- Its place and words are openm2's; the original shows what raises a character's points at the top left, by what is known. Checked by `affects.rs`'s tests (the words, the icons, the times) and with a client's icons with `view ui --screen game --affect 3:1=25/90 --affect 63:3=40/0`.
- The tab's key and its rules are openm2's; its layout is the original's. What the original checks before using one is open (`docs/roadmap.md`, "Skills in the original").

## The game menu
`crates/m2-cli/ui/menu.ron`, filled in by `menu.rs`: openm2's own windows, with the client's art (its boards, title bars, buttons, the taskbar's gears and the slider). In the game, Escape with nothing else to close opens the menu, as do the taskbar's gears; Escape, its X, the gears or Back to the game close it. Leaving the game closes it.

- **The menu** (`menu`, centre): five wide buttons.
  - **Settings** opens the Settings window in its place.
  - **Change character** goes back to the character list without signing in again (`Command::ChangeCharacter`, [`protocol/openm2/session.md`](protocol/openm2/session.md), "Changing character"): the server saves the character and sends the list, and the select screen shows it ("Play's screens").
  - **Log out** ends the session: the login screen, as Back on the select screen.
  - **Quit game** closes the window, logging out.
  - **Back to the game** closes the menu.
- **Settings** (`settings`, centre): what it shows applies at once, and `play` keeps it in the player's settings file ([`client.md`](client.md), "The player's settings").
  - **Music volume** and **Sound volume** (every sound but the music): sliders with the client's bar and knob (the looks `slider_bar` and `slider_knob`), in whole percents, the percentage beside each;
  - **Mute all sound**: a button saying `Off` or `On`, held down while on: nothing plays, the volumes are kept;
  - **Interface size**: − and + a tenth at a time from 50% to 200%, times the display's scale (or `--ui-scale`); the windows grow or shrink at once ("Scale"). Not a slider: the windows would move under the pointer while dragging;
  - **Show names**: a button saying `On` or `Off`, held down while on (the default): the names and levels over characters' heads (`render.md`, "Names over characters"); the target window names the target either way;
  - **Back** goes back to the menu; Escape or the X close both.
- The windows, their order and their words are openm2's; the original's own system and option windows are not copied.
- **An asset folder imported before these looks** (import 3 and earlier, [`assets.md`](assets.md), "The import") lacks the gears and the slider's art: `play` draws them in their fallback colours (the knob a light square) and notes it, until the client is imported again.

Checked by the tests of `screens.rs` (Escape and the gears open it, the sliders, Mute, the size and Show names change the settings, Change character shows the select screen at once, Log out and Quit) and `m2-ui`'s (sliders pressed, dragged, drawn); the looks with a client's art by eye with `view ui --screen game --show settings`.

## Tools
| Command | What it does |
|---|---|
| `openm2 view ui [LAYOUT…] --assets <dir> --screenshot out.png` | Draws layouts (the in-game one by default) over a plain background, with an asset folder's skin, items and skills (fallback colours without `--assets`). `--screen login\|select\|create\|loading\|game` draws one of `play`'s screens instead (without the character's picture). `--item SLOT=VNUM[xCOUNT]` and `--quick INDEX=SLOT` fill the game screen's inventory and quickslots with the folder's items, as `play` would (`--show inventory` to see the bag); `--good PLACE=VNUM[xCOUNT]` opens a shop selling them at the item table's buy price; `--teacher JOB:GROUP` a teacher's window (what it says follows `--class`, `--skill-group` and `--point 1=LEVEL`); `--choose-skill SKILL,…` a soul stone's window offering those skills. `--skill SKILL=LEVEL` gives our character a skill (`--show skills` to see the window; `--skill-group`, `--class`, `--skill-points`), `--quick-skill INDEX=SKILL` puts one on a quickslot. `--set ID=TEXT`, `--value ID=0.5`, `--line ID=TEXT`, `--show ID`, `--focus ID` set state first. `--ui-scale 2.5` (with a `--size` 2.5 times as large) draws them as on a 250% display, `--ui-filter` chooses the art's look ("Scale"). Without `--screenshot`, a window that takes the mouse and keyboard and prints clicks, submitted fields and slot presses |
| `openm2 play … --screenshot out.png --screen login\|select\|game` | One of `play`'s screens as drawn in the game (`render.md`, "Playing"); `--atlas` opens the atlas first, `--inventory` the inventory, `--character` and `--skills` the character window on either tab, `--shop` talks to the nearest NPC (its shop opens), `--skill SKILL` uses a skill at the nearest monster |

## Not covered yet
- Moving windows by their title bar.
- The minimap's and the atlas's marks for party members, guild areas and quest waypoints (the client's `waypoint*.sub`, `mini_waypoint*.sub`, `guildarea01.sub`), the atlas's names and tooltips, zooming the minimap with the wheel, the original's coordinates or time under the minimap.
- Splitting stacks, dropping part of a stack or gold, the costume window, the quickslot bar's pages, an item's full tooltip (bonuses, sockets, description, what a shop pays for it).
- Passive skills; the original character window's other pages (emotions, quests); the original's teacher dialog (a quest's pages, by what is known).
- Buying several at once, players' own shops, quest dialogs with NPCs.
- Deleting characters, choosing a look ("shape") at creation, the original's empire screen (a map of the three kingdoms), choosing a server or channel, remembering the account name, the loading screen's pictures.
- Input methods (IME); choosing part of a field's text (copying takes it all), and the clipboard in `view ui`'s window.
- Smoother upscaling of the art (a pixel-art upscaler when it is loaded).
- More settings: the camera, the fog, the window's size or full screen, keys; the original's help and options windows.
- The original client's fonts and text effects (outlines, shadows); which font face and size it uses.
- Item names of clients in other codepages than 1252, which the import decodes (`docs/roadmap.md`, "Item names in other codepages").

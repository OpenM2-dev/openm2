# m2-ui — rules for agents

`m2-ui` is the interface toolkit: windows from RON layout files, a skin file naming the user's interface art, the windows' state, pointer and keyboard input in, events and a draw list out. No GPU, no window, no clock, no file I/O: the application loads art and fonts' text comes from the system. The rules for what it does are in `docs/specs/ui.md`; the choices in `docs/architecture.md` (Design choices). `m2-render` draws its lists (`m2_render::UiRenderer`).

## Layout
`src/`:
- `geometry.rs`: `Rect`, `Anchor`, `Length` (pixels or `Fill(m)`, with its own RON deserializer), `place`.
- `layout.rs`: layout files: `Layout`, `Node`, `Kind`, `Align`.
- `skin.rs`: the skin file: `Skins` (with the tooltip's skin), `Font`, `Skin` (colour, frame, bar, image, tile, button, gauge).
- `slots.rs`: item slots: `SlotItem` (icon, count, height in cells, dimmed), `PointerButton`, the grid state and its cell geometry (`Slots`: an item covers cells below its top cell).
- `ui.rs`: `Ui`: nodes in an arena, ids, sprites, hover, press, focus, events (`UiEvent`), `Key`, the carried item, the tooltip and the labels over the game (`Label`); the `Painter` that turns nodes into quads.
- `text.rs`: `Fonts` (cosmic-text's font system and swash cache), the glyph `Atlas` and its `GlyphPage`s, shaped texts kept for reuse.
- `draw.rs`: `DrawList`, `Batch`, `Vertex` (`Pod`, uploaded as it is), `TextureRef`, `ImageId`, `Sprite`, `Colour`.
- `error.rs`: `UiError`, and `from_ron` (RON with plain optional values).

`tests/ui.rs`: layouts, validation, placement, input and events, and the quads skins draw, all without fonts (`Fonts::empty()`).

## Rules
- **No panics:** `unwrap`, `expect`, indexing and `panic!` are denied in `src/`. Chat text comes from the server and layouts from files: use `get`, char boundaries, saturating arithmetic. Tests may unwrap.
- **No I/O, no GPU, no clock:** the application hands in sprites (`Ui::set_sprite`), input and the screen size. Keep it that way, so everything is testable without a window.
- **Layouts are data.** A new window is a RON file plus the code that fills in its ids (`add-ui-window` skill). Add a widget kind only when no combination of the existing ones does the job; a new kind gets a row in `docs/specs/ui.md`, a test, and drawing in `Painter`.
- **Tests never depend on fonts:** use `Fonts::empty()` (text takes no space). System fonts differ between machines; check text by eye with `openm2 view ui --screenshot` (the `verify-visually` skill).
- **Whole pixels:** placement rounds to whole pixels and glyphs are drawn at whole-pixel positions. Keep it, or art and text blur.
- **Scale:** layouts and skins are in layout pixels; `Ui::layout` and the `Painter` multiply what they draw by the scale (`Painter::px`, `font_size`, `hairline`, and `sprite`, which hands out sprites at their scaled size). A new length in the painter goes through them. What the application hands in or reads back (pointer, node rectangles, label points, the size to draw) stays in screen pixels (`docs/specs/ui.md`, "Scale").
- **cosmic-text stays in `text.rs`.** Nothing else touches it, so the text stack can be replaced in one place.
- **Secret fields stay secret:** a `secret` field's text never goes into an event or `Debug` output; the application takes it with `take_text`. Keep it so when touching `Edit` or events.

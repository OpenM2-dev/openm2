# m2-content — rules for agents

openm2's own game content format: what the server's world holds, as a folder of TOML files and the maps' walk masks. The format is described in `docs/specs/server/content.md`; how the server uses it in `docs/specs/server/world.md`. **No I/O:** the edges read files into `Sources` (`m2-server`'s `content::load_dir`) and write what `Content::to_files` gives (`openm2 content import`, in `m2-cli`).

## Layout
- `src/model.rs`: the entries as the files say them (`Map`, `Monster`, `Item`, `Race`, `Level`, `Drop`, `Start`, `StartItem`, `Good`, `Skill`, `Pack`, `PackMember`, `Group`, `Npc`), `Parts`, the checked `Content`, and the world-unit view the server uses (`GroupSpawn` with what it puts in the world (`Spawned`) and its `Area`, a circle or a rectangle; `NpcSpawn`, a shop's goods in their places (`ShopGood`), and integer helpers such as `Monster::attack_range_cm`, `damage_percent`, `regen_ms`, `Drop::chance_ppm`); what the server reads of an item in the client's numbering (`Item::wear_position`, `refuses`, `level_limit`, `stack`, `weapon_damage`, `defence`, `potion`, `applied`), each documented in `content.md`'s table.
- `src/formula.rs`: `Formula`, skills' formulas: openm2's own reader (numbers, names, `+ - * /`, brackets, four functions) and evaluation in whole thousandths.
- `src/sources.rs`: `Sources` (path → text, and the walk masks' bytes), which files a folder may hold, the size caps.
- `src/walk.rs`: `WalkMap`, a map's squares a metre wide, open or blocked: read from and written to PNG, `clear` (a straight walk crosses no blocked square, followed in whole centimetres), `find_path` (bounded A*, legs joined with a clearance). The server checks walks and moves monsters with it; `play` finds its paths with it, so both agree.
- `src/load.rs`: `parse`: each file's format line first, then the whole file, every entry with its place (`toml::Spanned`); `parse_packs`, a `packs.toml` on its own, for the import.
- `src/validate.rs`: the rules and caps; `Content::build` checks parts put together in code by the same rules.
- `src/write.rs`: `Content::to_files`, the same text for the same content; `IMPORT_MARKER`.
- `src/error.rs`: `ContentError` (`file:line:column: message`) and `Errors`.
- `tests/parse.rs` (a folder read field by field, every rule broken with its line, written files read back), `tests/random.rs` (random and damaged folders never panic; random valid content reads back the same), `tests/walk.rs` (masks, straight walks and paths on hand-drawn grids, PNG files built byte by byte, a folder's masks and their rules).

## Rules
- **No panics:** `unwrap`, `expect`, indexing and `panic!` are denied in `src/`. The files are the operator's, but a typo must give a message, not a crash.
- **Every rule names its place:** a problem with an entry carries the entry's file and line; all problems are collected (up to `MAX_ERRORS`), never just the first.
- **Strict:** unknown keys and files are refused (`deny_unknown_fields`); every reference must exist; every number has a range. A new rule goes into `docs/specs/server/content.md` and `every_broken_rule_is_named_with_its_line`.
- **Stable output:** no `HashMap` or `HashSet` (`clippy.toml`); entries are written in a fixed order, floats in their shortest form with `.0` kept, so writing twice gives the same bytes and reading them back gives the same `Content`.
- **Units:** files are in metres, seconds and degrees; conversions to world units (centimetres, milliseconds, steps of 5 degrees) happen here, once, so the server's core never sees a float from a file.
- **Formulas in thousandths:** a formula is parsed once, when the folder is read, and worked out with `i64` thousandths, every step rounded towards zero and clamped, so every machine gets the same numbers; no floats, no recursion past the nesting cap (32).
- **The format's version:** `FORMAT` is in every file. New optional fields and new files keep it; renaming a field or changing what one means bumps it, with the reason in `docs/architecture.md` (Design choices).
- **No Metin2 data:** tests use made-up names, numbers from 1001, maps like `test_map` and made-up skill formulas (never the client's skill table's). Imported content is the operator's and never enters the repo (`docs/source-policy.md`); files starting with `IMPORT_MARKER` fail `just no-assets`.
- **Walking is exact:** `clear` works in whole numbers, and `find_path`'s search is ordered by cost, then the estimate left, then the square, so every machine finds the same path. Its search box is capped (`MAX_PATH_SIDE`); callers never get unbounded work from a far point.
- **Dependencies:** `serde`, `toml` and `png` (walk masks) only; no workspace crate (`ALLOWED_DEPS`).

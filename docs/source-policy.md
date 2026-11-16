# Source policy

openm2 is written from scratch. These rules keep it free of code and data it has no right to, so it can stay open source.

## Allowed inputs
- **Behaviour we observe ourselves:**
  - captures of our own sessions against openm2's own server;
  - the bytes of files in the user's own client, read on the user's machine.
- **Facts from open-source projects with a clear license, written in our own words.** Facts are values, orders, sizes, formulas, file layouts. For example:
  - Quantum Core X (MPL-2.0): server behaviour and game rules;
  - LSLib (MIT): the Granny GR2 format;
  - liboodle (The Unlicense): the Oodle1 compression of GR2 sections;
  - Microsoft's DirectX documentation and the Khronos Data Format Specification: the DDS header and the DXT block formats.

  Note the source next to the fact (a line in the spec, e.g. "(QCX)" beside a server rule).
- **Code from permissively licensed projects that were not derived from leaked sources** (MIT, Apache-2.0, BSD, Zlib), ported with attribution in the file header; its license goes into the third-party notices (`just notices`).
- **Facts from the original client's and server's source:** how the client does something (which texture it draws with, what it multiplies, which rule picks an effect, which bone it attaches to), written in our own words, with the source named beside the fact ("the original client's source"). Never its code, names, structs, comments or file trees in the repo, its issues or its commits; openm2's code is written from the facts.
- **Community documentation (wikis, forum posts) for facts only.** Much of it was written from the leaked original sources, so take only facts from it and never copy its text or code.

## Forbidden
- Code, structs, header lists, comments or file trees from the **leaked original client or server sources**, or from tools built on them, even "just for reference" inside the repo. (Facts read from the source, as allowed above, are not code.)
- Game files of any kind:
  - archives (`.eix`, `.epk`, `.ecpk`), models (`.gr2`), textures, maps, sounds;
  - proto dumps, UI scripts.
- Screenshots or golden images rendered from real game files. Test images must come from synthetic scenes.
- Archive keys other than our own test keys. The user supplies their own `keys.toml`.
- Captures from other people's servers, or that contain someone else's credentials.

## Our own server
openm2's server follows the same rules as the client, with these on top:
- **Its behaviour comes from the allowed inputs above:** openm2's own protocol, facts from QCX and from the original's source in our own words. Where they leave a gap, the rule is openm2's own design, written down in the spec as openm2's choice.
- **Forbidden, as for the client:** code, configuration, quest scripts and data files of the leaked original server (its item, monster, drop, spawn and skill tables, in any form), even "just for reference", and also as a source of numbers and formulas.
- **Metin2 data only from the operator's own files, on the operator's machine:** the server reads its world from openm2's own content format; `openm2 content import` converts the maps, monsters, items, skills and spawn lists of the operator's own client into it there, as the client reads them on the player's machine. Every file the import writes starts with a marker line, and `just no-assets` refuses such files. No Metin2 data is committed, shipped with the server, or put into its database seeds or tests; tests use made-up content, made-up skill formulas included.
- **Spawns, drops and shops for the Metin2 maps are the operator's own:** written by them, or for spawns imported from the spawn lists their own client carries, never taken from the leaked server's tables (above), its group tables among them: the packs the imported groups name are the operator's to write. A starter world for the Metin2 maps stays in `local/`, like any other imported content.
- **The openm2 client only:** it speaks openm2's own protocol; no work aimed at making it accept or check the original client (its login checks, its quirks).
- No data taken from official servers, by capture or otherwise.

## Original content
The repo is to hold a small original sample world (`docs/roadmap.md`):
- Every model, texture, map, sound or text in it is made for openm2, or comes under a license that allows it here (CC0, CC-BY, MIT, Apache-2.0), with its author, source and license written beside it.
- Nothing in it is traced, copied, converted or recoloured from Metin2's files or art, and it uses none of Metin2's names, characters or logos.
- Trees are already openm2's own: grown by code from its recipes, with textures drawn by code. The client's SpeedTree files are never read, and nothing from SpeedTree's SDK, documentation or code is used. A tree file's name is read only to pick a recipe.
- Where the sample world lives and how `just no-assets` lets it through is decided with it; until then the repo holds no pictures, models or sounds at all.
- **Converting the player's own client** to the open formats (`openm2 assets import`) happens only on the player's machine, like `content import`. Every file it writes is marked as imported (`docs/specs/assets.md`, "Imported files"), and `just no-assets` refuses `.glb` and `.gltf` files and the TOML marker. Imported files are never committed, put in a release, or published on an update channel (`openm2 release manifest` refuses them), and nothing imported, even edited, becomes original content.

## Fixtures
- **Test data is made by us.** It is generated by our own writers (`m2-formats`) or written by hand: byte vectors, transcripts, synthetic archives with test keys.
- **Fixtures live in a `fixtures/` folder** (top level or `crates/<crate>/tests/fixtures/`) and stay small; `just no-assets` caps them at 64 KiB. Game format tests build their data at run time with our writers instead of committing binary files.
- **Tests that need real game files** read them from the path in `OPENM2_CLIENT_DIR` (or an import of it from `OPENM2_ASSETS_DIR`) and are skipped when it isn't set. They never copy those files into the repo.

## When unsure
Don't add it. Ask a maintainer.

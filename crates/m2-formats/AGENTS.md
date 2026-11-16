# m2-formats and m2-assets — rules for agents

`m2-formats` parses and writes the client's game files. **No I/O**: everything works on byte slices. `m2-assets` does the file I/O around it: the user's `keys.toml`, archives on disk, the VFS, extraction. The formats are specified in `docs/specs/formats/`; `docs/architecture.md` (Design choices) explains the choices.

## Layout
`crates/m2-formats/src/`:
- `reader.rs`: `Reader` (bounds-checked little-endian reads that name the structure in errors) and `Writer`.
- `raw_str.rs`: `RawStr`, a NUL-terminated string field kept as bytes, printed escaped.
- `error.rs`: `FormatError`.
- `keys.rs`: `Keys` and `KeyName`: which XTEA key a structure needs, named as in `keys.toml`.
- `crc32.rs`, `xtea.rs`, `lzo.rs`: the algorithms. `lzo.rs` decodes LZO1X and has a literal-only encoder for test data.
- `container.rs`: `MCOZ` containers.
- `eterpack.rs`: archive index and entries (`Entry::data_len` applies the real-client size rules), `ArchiveWriter`, `normalize_name`.
- `pack_list.rs`: the client's archive list, `pack/Index`.
- `proto.rs`: `item_proto` and `mob_proto`.
- `oodle1.rs`: Oodle1 decompression (GR2 sections) and an encoder for test data. The decay rule differs from its published description on purpose; see `docs/specs/formats/oodle1.md`.
- `gr2/`: GR2 files. `mod.rs` header, sections, relocations; `object.rs` walks the stored layouts (`Object`, `Value`, `Numbers`); `model.rs` copies out skeletons, meshes, materials, animations (`Gr2File`), samples curves, and gives the model layer its models (`Gr2File::model`) and animations (`Animation::to_animation`, `Curve::to_keys`); `write.rs` builds files for tests (`write_file`, `write_model`).
- `model.rs`: the model layer, what every model format is read into and renderers draw: `Transform`, `Skeleton`, `Bone`, `Vertices` (GR2's re-exports them), `Model`, `Mesh`, `Group`, `Texture` (a file or an image stored in the model's file, and an `Alpha` mode), `Animation`, `Track`, `Keys` (step, linear and cubic-spline keys, as glTF has them). It is not a file format: no reader or writer of its own.
- `gltf/`: glTF 2.0 files (`.glb`, `docs/specs/formats/gltf.md`). `mod.rs` the GLB container (`Gltf::parse`), limits, the frames (`Direction`: the renderer's centimetres and +Z up against glTF's metres and +Y up); `json.rs` the part of the JSON openm2 reads and writes (serde); `read.rs` a scene into a `Model`, the animations into the layer's, floors (nodes marked `openm2_floor`), embedded images; `write.rs` the layer and floors back (`write_glb`), with openm2's own `OPENM2_scale_shear` for shear and its `extras`; `math.rs` f64 matrices for nodes' rest transforms.
- `mde.rs`: mesh effects (`MeshEffect`): the two layouts, `write` for tests.
- `mdatr.rs`: model attributes (`ModelAttributes`): a building's collision shapes and height planes (floors), `write` for tests.
- `wav.rs`: WAV sounds (`Wave`): RIFF chunks, PCM samples, `write` for tests. MP3 is not a game format of ours: `m2-audio` decodes it with nanomp3.
- `dds.rs`: DDS textures: header, levels, DXT1/3/5 and bit-mask decoders, `write` for tests. `tga.rs`: TGA images: header, colour map, plain and run-length pixels, `write` and `encode` for tests. `jpeg.rs`: JPEG, baseline and progressive: markers, Huffman tables, progressive scans, the inverse DCT, chroma sampling; `encode` and `encode_progressive` for tests. `image.rs`: `RgbaImage`, the decoded pixels.
- `script/`: text scripts. `mod.rs` the `Group`/`List` syntax (`parse`, `write`, `Script` with lookups that ignore case); `race.rs` model scripts (`RaceData`: shapes, hair styles, skins, collision spheres, attached effects, smoke); `motion.rs` motion scripts (`MotionData`: the animation, the combo window, the attack data with its hits, the events) and motion lists. `environment.rs` environment scripts (`Environment`: light, material, fog); `effect.rs` effect scripts (`EffectData`: particle and mesh elements, `Curve`); `sound.rs` sound scripts (`SoundScript`: timed sounds, `script_path` from a motion's or effect's path); `npc_list.rs` the monster list (`npclist.txt`, a line format: which model each monster number uses); `item_list.rs` the item list (`item_list.txt`, tab-separated: each item number's icon and ground model); `sub.rs` sub-images (`SubImage`: a rectangle of an interface image). A new kind of script gets its own module that reads the `Script` tree and writes it back (`to_script`).
- `map/`: maps. `setting.rs` (`MapSetting`, a script), `grid.rs` (height, tile, shadow, attribute and water maps), `blocks.rs` (the editor's `Start`/`End` block files), `area.rs` (`AreaData`, placed objects), `ambience.rs` (`AreaAmbience`, sound spots), `texture_set.rs` (`TextureSet`, numbered textures), `property.rs` (`Property`, `YPRT` files; `ambience()` for sound spots), `atlas.rs` (`atlasinfo.txt`), `points.rs` (`MapPoint`, the atlas's NPCs and warps).
- `tests/`: one file per format, plus `common/` with our test keys.

`crates/m2-assets/src/`:
- `keys.rs`: `keys.toml`.
- `archive.rs`: `Archive`, an `.eix`/`.epk` pair on disk, read with positional reads.
- `vfs.rs`: `Vfs`.
- `path.rs`: `safe_relative_path`.
- `extract.rs`: `extract`.
- `texture.rs`: `texture_paths`/`find_texture`, where the textures a model names are; `sub_image_path`, the image a sub-image cuts from.
- `effect.rs`: where the files an effect names are (`find_effect_texture`, `find_effect_mesh`), `read_effect`, a weapon's item script (`item_script_path`).
- `map.rs`: a map read from the client (`read_setting`, `read_area`, `read_texture_set`, `map_folders`, `read_atlas`), `PropertyIndex` (property files by id), a model's attributes beside it (`read_model_attributes`).
- `sound.rs`: a motion's or effect's sound script (`read_sounds_of`), the music (`read_music`, `music_names`: the client's `BGM` folder, then `bgm/`), the original client's volumes (`metin2.cfg`: `read_sound_settings`).
- `actor.rs`: the model scripts of player characters (`player_script`, `player_motion`) and monsters (`read_npc_list`, `monster_script`); what worn items change (`body_shape`, `weapon_grip`, `hand_bones`); the names of the motions they fight with (`combo_motion`, `PLAYER_ATTACKS`, `MONSTER_ATTACK`, …).
- `folder.rs`: asset folders (`docs/specs/assets.md`): paths and references (`check_path`, `resolve`, `reference`, `relative`), `AssetFolder` (reading, never outside it), `write_file` (one folder level at a time, never through a link, replacing only when asked), `assets.toml` (`FolderFile`), the import's markers (`IMPORT_MARKER`, `IMPORT_TEXT`, `import_mark`).
- `actor_file.rs`: actor and motion files (`ActorFile`, `MotionFile`): TOML in metres, to and from the client's `RaceData`, `MotionData` and `SoundScript`.
- `map_file.rs`: map and area files (`MapFile`, `AreaFile`, `ObjectFile`): TOML in metres, to and from the client's map settings, texture set, environment and atlas points, and objects placed as the client places them; the areas' raster names (the rasters themselves are PNG, read and written in `m2-cli`'s `map_folder.rs`).
- `items.rs`: the locale setting (`locale.cfg`, a loose file in the client folder: `read_locale`) a locale's items (`read_items`: the item table joined with the item list's icons and models) and monsters (`read_monsters`: names and levels from the monster table).
- `tests/client.rs`, `tests/map.rs`, `tests/actor.rs`, `tests/items.rs`, `tests/sound.rs`: synthetic clients in temp folders; `tests/folder.rs`: asset folders' paths, writing, markers, and actor and motion files through TOML and back; `tests/map_file.rs`: map and area files through TOML and back.
- `tests/real_client.rs`: the human's client, only when `OPENM2_CLIENT_DIR` is set (archives, proto tables, a sample of GR2 and DDS files, model and motion scripts, motion lists, maps, every effect script and mesh effect, every model attribute file written back byte for byte, every sound, sound script and sound spot).

## Rules
- **No panics:** `unwrap`, `expect`, indexing and `panic!` are denied in `src/` of both crates. Use `Reader`, `get`, `split_first`, slice patterns. Tests may unwrap.
- **Untrusted sizes:**
  - Check every count and length against the bytes actually left before allocating.
  - Cap sizes that come from headers (`MAX_REAL_SIZE`, `MAX_INDEX_FILE`, …).
  - Use checked or saturating arithmetic on values from a file.
- **Every parser has a writer** that produces the same layout, so tests can build data. No real game files or keys in the repo, ever (`docs/source-policy.md`). Tests build their inputs at run time; do not commit binary fixtures for game formats.
- **Every structure gets a byte-exact test.** Write the bytes out field by field in the test, with a comment and an offset assert per group of fields, then check both directions: the writer produces them and the parser reads them.
- **Every parser gets property tests:**
  - random bytes never panic;
  - damaged real-shaped input (a valid encoding with bytes flipped) never panics;
  - round trips where they make sense.
- **Layouts are versioned:**
  - If a structure has a version or record size, match it against the layouts we know.
  - An unknown one is `FormatError::UnknownLayout`, naming what we know. Never guess.
  - A new layout gets its own record type or variant, its own vector, and a line in the spec.
- **Strings stay bytes** (`RawStr`). Convert them at the edge, never here.
- **Keys:**
  - Keys come in through `Keys`; `XteaKey` and `Keys` never print key words.
  - A missing key is `FormatError::MissingKey` naming the `keys.toml` slot.
  - A failed inner-magic check is `WrongKey`.
- **Paths on disk:** a name from a game file becomes a path only through `m2_assets::safe_relative_path`, a reference in an asset folder's file only through `m2_assets::folder::resolve`. Writing goes through `extract` or `folder::write_file`: one folder level at a time, never through links, never over an existing file unless asked (an import's `--force`).
- **Facts and sources:**
  - Every spec in `docs/specs/formats/` says where its facts come from and what it was verified against.
  - Facts from QCX or community docs, never code (`docs/source-policy.md`).
  - When the human's client disagrees with a spec, the client wins. Add a test that shows the difference (built with our writers), then fix the parser and the spec.

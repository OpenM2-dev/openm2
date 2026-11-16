---
name: verify-visually
description: Render a model, map or interface window of an import of the human's client to a PNG with `openm2 view model|map|ui|effect --screenshot` (or the game with `openm2 play --screenshot`) and look at it, to check that a renderer, animation, texture, terrain, actor, interface, import or format change draws correctly. Use when asked to check, render, screenshot, view or verify a model, animation, texture, map, character, monster, window or anything drawn.
---

# Verify visually

openm2 can draw any model, map, effect and window of an asset folder offscreen into a PNG. You can read PNGs, so look at the result instead of guessing. What should appear is in `docs/specs/render.md` (and `docs/specs/ui.md` for windows). Every command lists its flags with `--help`.

## 1. Set up
- Real models exist only in the human's client. You need its folder and their `keys.toml` (usually `local/keys.toml`). Ask for the folder if you don't know it; don't scan the disk for it.
- **`play` and the `view` commands read only asset folders.** Import the client first, in a release build (debug wgpu is slow); `--actor PATH`, `--map FOLDER` or `--effect PATH` import only some, faster. Import again after a change to the import (`--force`):
  ```sh
  cargo build --release -p m2-cli
  C=<client folder>
  target/release/openm2 assets import --client "$C" --keys local/keys.toml --out local/assets
  A=local/assets
  ```
- **Screenshots of real models show the human's assets.** Write them to `local/` (gitignored), never anywhere else in the repo.
- In the asset folder a game path loses its drive (`d:/ymir work/pc/warrior/warrior_novice.gr2` is `ymir work/pc/warrior/warrior_novice.glb`), the player races' actors are at the root (`warrior_m.actor.toml`), and `assets.toml` names every monster's actor. Paths in the client: `openm2 vfs ls`; a script's shapes, hair styles and motions: `openm2 script dump`.
- No GPU: `--software` uses WARP or llvmpipe (slower, same picture). Without `--screenshot` a window opens for the human; you cannot see it.

## 2. Render and check

### Models
```sh
target/release/openm2 view model --assets "$A" warrior_m.actor.toml --shape 4 --hair 2 \
  --motion general/run --frames 8 --size 192x256 --screenshot local/run.png
```
- A loose file: `view model x.glb --part hair.glb --anim wait.glb` (or GR2 with its textures beside it). A monster: its actor from `assets.toml` with `--motion RUN`.
- `--frames N` puts N poses of the animation in a grid (left to right, top to bottom): that is how to check motion in a still. `--yaw 90` shows the side (0 is the front), `--pitch 30` looks from above.
- `view model` prints notes for textures it could not find; a missing texture draws white.

### Maps
```sh
target/release/openm2 view map metin2_map_a1 --assets "$A" --pitch 90 --size 1024x1280 --screenshot local/a1.png
target/release/openm2 view map metin2_map_a1 --assets "$A" --at 650,590 --distance 60 --pitch 30 --screenshot local/spot.png
```
- `--at X,Y` is in metres from the map's top left corner (east, south); `map info` gives the size (an area is 256 m). `--pitch 90` looks straight down, north up.
- **Compare with the client's own minimaps:** every area has one (`minimap.png` in the area's folder of the import). Terrain, water and buildings (place and rotation) should match; trees stand where the minimap has them, but their shapes are openm2's own recipes, so compare where they stand, not their shapes.
- `--floors` draws the buildings' floors in magenta: they should lie on platforms, steps and bridge decks. `--fog`, `--no-objects`, `--radius M` help isolate a problem.
- Effects placed on the map play for `--effect-time` seconds first; `map info --client` lists where a map's effects are.
- The sky: `--above 10 --distance 8 --pitch -20` (`--yaw 180` faces south). Bands fade without steps, clouds drift overhead and vanish towards the horizon, a sky of pictures joins at its corners without seams.
- Where one may walk: `openm2 map attr <map> --client "$C" --bits 0x01 --out local/attr.png` tints blocked squares red. Buildings, cliffs and deep water red; roads, bridges and piers clear. The content import's walk masks should show the same.
- Trees in a wood (`--at X,Y --distance 60 --pitch 20 --fog`): trunks on the ground over their baked shadows, full crowns without dark cards, fall trees red or yellow, winter ones bare or snowy.

### Trees (no client needed)
```sh
target/release/openm2 view tree beech --variants 4 --size 320x400 --screenshot local/beech.png
```
- The trunk stands upright and carries the leaves; the crown is full from the side and from above; no dark fringes on leaves; `--season fall|winter|flowers` changes colours, bares branches or adds snow or blossoms. A tree file's name instead of a recipe shows which recipe and season it picks.

### Effects
```sh
target/release/openm2 view effect "ymir work/effect/hit/blow_1/blow_1.effect.toml" --assets "$A" \
  --frames 9 --duration 0.45 --size 256x256 --distance 3 --screenshot local/fx.png
```
- `--time S` draws one moment; `--seed` plays it another way. The printed line says how many particles are alive at the end and whether it finished.
- Particles come out where and when the script says (sparks fly, smoke rises), additive ones glow without dark squares, flat ones lie on the ground, meshes play their frames and fade, nothing white. What some numbers mean is openm2's reading (`docs/specs/formats/effect.md`): say what looks wrong.

### The interface
```sh
target/release/openm2 view ui --assets "$A" --screen game --value taskbar.health=0.7 \
  --line "chat.log=hello" --show chat.bar --focus chat.input --screenshot local/ui.png
```
- Boards and bars join without gaps or seams, gauges fill as far as their value, text is sharp, inside its box and readable. The `add-ui-window` skill has more.
- Scaled: `--ui-scale 2.5 --size 3200x2000` (and `--ui-filter sharp`). The same checks as at 1, and stretched art (gauges, board edges) runs straight. Crop and compare at full size.
- The inventory: `--show inventory --item SLOT=VNUM[xCOUNT] --quick INDEX=SLOT` (bag 0–89, worn from 90). Icons fill their cells, worn items sit over their places, counts show for stacks.
- The launcher's window, no client needed: `just launcher --dir DIR --screenshot local/launcher.png --show ready` (any stage, `--ui-scale 2`). Texts fit their buttons, the gauge fills as far as its value, the bottom row's buttons don't touch, the picture is 4 : 1 and sharp.

### The game (`play`)
`play` needs openm2's own server with a content folder (`docs/server.md`; ask before starting one, Postgres or Docker):
```sh
OPENM2_SERVER_KEY=sha256:... target/release/openm2 play --host 127.0.0.1 --provider <id> --assets "$A" \
  --walk 3,0 --stay 2 --screenshot local/play.png
```
- `--distance`, `--yaw` (0 looks north) and `--pitch` place the camera; `--screen select|login` draws those screens; `--no-interface` draws the world alone.
- The character stands on the ground (not floating or sunk), faces the way it walked, in its wait pose; monsters face the way the server said; buildings match `view map` of the same spot; the taskbar and chat show their values and lines.
- Worn items: dress the character with `openm2 bot --use SLOT`, then `--distance 4 --pitch 10 --yaw 270`. The weapon's grip is in the fist, the stance fits its type.
- Fights: `--fight` with a monster group near the start (target window, swings, damage numbers); `--fight --after-hit 0.08` catches a hit's flash at the side of the one hit.
- Items on the ground: drop some with `bot --drop 1 --drop-gold 30`, then `play --screenshot` near the spot (`--pick-up` picks them up). Weapons lie flat, the rest stand, names stacked and readable, a twinkle over each.

## 3. Look
Read the PNG and check, in order:
1. **Shape:** the whole model is in the picture, upright, facing the camera at yaw 0. No triangles stretch away to the edge (a wrong joint or palette).
2. **Textures:** in place, not upside down or mirrored; no holes (alpha used as coverage where it is a gloss mask), no white patches (missing textures; check the notes).
3. **Pose:** limbs bend at joints, nothing twisted; a `--frames` grid changes smoothly, and a loop's last cell leads back to the first.
4. **Parts:** hair and other parts sit on the body and move with it.
5. **Maps:** terrain blends without seams between areas, water lies in valleys, buildings stand on the ground and face their roads.

If something looks wrong, the usual suspects, in order:
- curve sampling (`m2_formats::gr2::model::Curve::sample`);
- the transform order (`m2_render::pose::transform_matrix`);
- joint remapping (`m2_render::model::Part::from_model`);
- the import: its report, `assets check`, and the converted file against the client's (`gr2 model`, `dds png`);
- texture lookup (`m2_assets::texture_paths`); skins and paths from an actor (`m2_assets::actor_file`, `PartList::resolve`);
- maps: the area grids (`m2_formats::map::grid`), the mesh (`m2_render::terrain::AreaMesh`), the tile blend (`shaders/terrain.wgsl`), object placement (`mapview::object_transform`);
- `play`: world to scene coordinates (`Game::to_scene`), the direction rule (`m2_core::Point::angle_to`), which model an entity gets (`m2_assets::actor`, `actors.rs`).

## 4. Report
In the handoff, say what you looked at (models, animations, angles, windows) and what you saw. Don't add the screenshots to the repo; describe them, or leave their `local/` path for the human. Ask the human to compare with the original client when only they can tell.

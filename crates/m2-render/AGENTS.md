# m2-render — rules for agents

`m2-render` draws the client's models, maps and effects with wgpu, and the interface's draw lists over them. It reads models and animations of the model layer (`m2_formats::model`, whatever file they came from), DDS, TGA, map and effect data from `m2-formats` and draw lists from `m2-ui`, owns no window, and draws offscreen or into a surface its caller made. What it draws and why is in `docs/specs/render.md`; the libraries in `docs/architecture.md` (Design choices).

## Layout
`src/`:
- `pose.rs`: CPU skeletons. `transform_matrix` (T × R × S, as Granny's), `Pose` (rest, animated from an animation's keys, following another skeleton by bone name, palette), `AnimationBinding`.
- `model.rs`: `Part` (one model of the model layer as a vertex list, joints into its own skeleton, draws per texture: `Part::from_model`), `Vertex`, `TextureData` (levels from DDS or TGA, alpha as coverage or not).
- `camera.rs`: `Camera` (orbit, framing a box, light direction, `ray` for picking), `Bounds`.
- `gpu.rs`: `Gpu` (adapter, device, queue; wgpu errors collected and returned by `check`; `GpuOptions`: software, low power), `block_on`.
- `surface.rs`: `WindowSurface`, a window's surface its caller made, configured for gamma space (a non-sRGB view), its frames; no winit.
- `terrain.rs`: `AreaMesh` (an area's mesh, tile map, shadows and water from its grids), `MapScale`, `TerrainLayer`, texture helpers (`layer_side`, `resample`, `mip_chain`), `TEXTURE_SPAN`.
- `effect.rs`: effects on the CPU. `EffectModel` (a script with its mesh files and texture numbers), `EffectInstance` (one playing: emitters, particles, mesh frames, seeded randomness; `advance`, `draw`, `finished`), `EffectGeometry` (world-space triangles in runs by texture and blending), `View`, `position_at`.
- `sky.rs`: `Sky` (colour bands, a box's six pictures, `Clouds`), the sky pipeline drawn first (one triangle over the target, each pixel's view ray picking its colour), `GpuSky`.
- `tree.rs`: trees grown from openm2's recipes. `TreeBook` (RON, every number capped; `pick` by the words of a tree file's name), `TreeRecipe`, `Season`, `grow` (seeded `Rng`; bark then leaves in one unskinned `Part`), `CROWN_NORMAL`.
- `tree_texture.rs`: tree textures drawn by code: `bark`, `leaves` (clusters, needles, tufts, fronds, blades; blossoms, snow), `coverage_mips` (mip levels that keep the alpha test's coverage), `textures` for a recipe and season.
- `effect_pipeline.rs`: the effect pipeline, one per blending made when first drawn (`blend_state` maps Direct3D's blend factor numbers), `GpuTexture`, `GpuEffects`.
- `renderer.rs`: `Renderer` (the model, terrain and water pipelines, the effect pipelines and the sky's; `upload_texture`, `upload_effects`, `upload_sky`), `GpuPart` and `Renderer::instance`, `GpuTerrainSet`, `GpuArea`, `World` (with the frame's effects, sky and seconds), `Attachments` (depth, MSAA), `Offscreen` (draw and read back, or show in the interface; RGBA8 or BGRA8; `for_interface`, for an interface alone with no renderer), `Scene`, `Fog`.
- `ui.rs`: `UiRenderer`, the interface pipeline: images by `m2_ui::ImageId`, pictures (an `Offscreen` target's view, blended premultiplied), glyph pages by version, batches with scissors; `clear`, for an interface with no world under it (the launcher's window).
- `shaders/model.wgsl`, `shaders/terrain.wgsl`, `shaders/effect.wgsl`, `shaders/sky.wgsl`, `shaders/ui.wgsl`: the shaders. `model.wgsl` lights normals longer than 4 (a tree crown's) as they are. Keep their `Frame` structs in step with `FrameUniform` in `renderer.rs`, `Layers` with `LayersUniform`, `Sky` with `SkyUniform` in `sky.rs`, and `Screen` with `ScreenUniform` in `ui.rs`.
- `png.rs`: PNG encode and decode.

`tests/`: `cpu.rs` (no GPU needed), `gpu.rs` (offscreen pixels, effects blended over them; like every test that draws, it fails without an adapter), `terrain.rs` (area meshes on the CPU, terrain and water on the GPU), `sky.rs` (the sky's bands, pictures and clouds), `tree.rs` (recipe books, growth and textures on the CPU, a tree on the GPU), `effect.rs` (effects played on the CPU), `ui.rs` (a draw list over a target, an interface alone over its own colour), `common/` (a synthetic quad model and animations).

## Rules
- **No panics:** `unwrap`, `expect`, indexing and `panic!` are denied in `src/`. Models come from game files: check counts, use `get`, saturating arithmetic. Tests may unwrap.
- **No validation errors left behind:** after creating GPU objects or drawing, call `gpu.check(…)`, so a wgpu error becomes a `RenderError` instead of a panic in wgpu's default handler.
- **CPU first:** put math (posing, framing, model building) in plain functions with CPU tests. GPU tests only check what needs pixels.
- **GPU tests must pass without a GPU:** start them with `let Some(gpu) = gpu() else { return };`. Use synthetic scenes only, compare pixels with a tolerance of ±2, and never use golden images of real models.
- **Look at the result:** a change to what is drawn gets a screenshot check on the human's client (the `verify-visually` skill). Say in the handoff what you looked at.
- **Gamma space:** textures upload as `Rgba8Unorm`, targets are non-sRGB. Don't switch to sRGB without a design choice recorded in `docs/architecture.md`.
- **Upgrades:** wgpu and winit releases break APIs. Upgrade one at a time, on their own branch.

# glTF 2.0 (`.glb`)

openm2's own format for models and animations: what an asset folder holds in place of GR2 files ([`../assets.md`](../assets.md)), and what any modelling tool that exports glTF can make for openm2. openm2 reads and writes the part of glTF it draws, binary files (`.glb`) only, into and from the model layer (`m2_formats::model`), as GR2 files are read ([gr2.md](gr2.md), "Into the model layer").

**Code:** `m2_formats::gltf` (`Gltf::parse`, `Gltf::model`, `Gltf::animations`, `Gltf::floors`, `Gltf::embedded_image`, `Gltf::mesh_effect`; `write_glb`, `write_mesh_effect`; the JSON in `gltf::json`). Commands: `openm2 view model x.glb` (with `--assets` a path in the folder), `openm2 assets import` writes them, `openm2 assets check` reads them all back.

**Source:** the glTF 2.0 specification (Khronos Group, published openly), for facts only. openm2's own additions are below ("openm2's own").

**Verified against:** our writer's files, read back (`crates/m2-formats/tests/gltf.rs`: a file byte for byte, models and animations of the layer, GR2 files through GLB, refusals, damaged files); a file laid out as other tools write them (an armature node, a skinned mesh beside it, a rigid mesh under a bone, quantised rotations, an image beside the file, a floor marked as Blender writes a custom property), built in the tests; and a TMP4 client through `openm2 assets import` (below). Not yet checked against a file Blender wrote.

## The container
Little-endian.

| Offset | Size | What |
|---|---|---|
| 0 | 4 | the magic `glTF` |
| 4 | 4 | `u32`: the version, 2 (others are refused) |
| 8 | 4 | `u32`: the whole file's length; it must not be more than the file |
| 12 | … | chunks: `u32` length, `u32` type, the data |

- **The first chunk is JSON** (type `0x4E4F534A`, `JSON`), at most 16 MiB, padded with spaces (zeros and line ends are trimmed too). A second chunk of type `0x004E4942` (`BIN\0`) is the binary buffer, padded with zeros. Chunks of other types are skipped.
- **The JSON** must say `asset.version` 2.x; a `minVersion` other than `2.0`, and any extension in `extensionsRequired` but openm2's own, are refused by name.
- **Buffers:** buffer 0 without a `uri` is the binary chunk; it may be up to 3 bytes shorter than the chunk (padding). A buffer in a file of its own (`uri`) is refused when an accessor needs it.
- **Accessors:** `SCALAR`, `VEC2` to `VEC4`, `MAT4`, `MAT3`; components of every type, floats or normalised integers where a float is meant (`TEXCOORD_0`, `WEIGHTS_0`, rotations), unsigned bytes, shorts and ints for indices and joints; matrices of floats only; any `byteStride` at least the element's size. An accessor without a buffer view is all zeros. Sparse accessors are refused by name. Each element must lie inside its view, and each view inside its buffer.
- **Limits:** at most 4,194,304 elements an accessor, and as many nodes, meshes, accessors and the like in one file.

## What is read, refused and ignored
- **Read:** scenes and nodes (`translation`, `rotation`, `scale` or `matrix`); meshes of triangles with `POSITION`, `NORMAL`, `TEXCOORD_0`, `JOINTS_0` and `WEIGHTS_0`, and their indices; skins with their joints and inverse bind matrices; animations of `translation`, `rotation` and `scale` with `STEP`, `LINEAR` or `CUBICSPLINE` samplers; a material's `pbrMetallicRoughness.baseColorTexture` and `alphaMode`; images in the binary chunk (`image/png`) or beside the file (a relative `uri`, `%XX` decoded); floors, nodes marked as openm2's ([below](#floors)).
- **Refused by name, when a file needs it:** an extension it requires; sparse accessors; morph targets (`targets`, `weights`, and channels moving `weights`); buffers in files of their own; more than four joints a vertex (`JOINTS_1`, `WEIGHTS_1`); primitives other than triangles (mode 4); skins of more than 256 joints, or joint numbers over 255 (the renderer's palettes); images that are not PNG; a base colour texture on other texture coordinates than `TEXCOORD_0`; `uri`s with a scheme (`data:`, `http:`) or an absolute path.
- **Ignored:** cameras, lights, other material values (the base colour factor, metal, roughness, normal maps), other vertex attributes (`TANGENT`, `COLOR_0`, `TEXCOORD_1`), `alphaCutoff` (the renderer tests at one half), samplers' filters and wrapping, channels of other paths (an extension's, only used), and extensions only used.

## Axes and units
glTF's are metres, +Y up and +Z forward; the renderer's are the client's: centimetres, +Z up and −Y forward (`docs/specs/render.md`). A point (x, y, z) of the renderer is (x, z, −y) / 100 in glTF, and read back the other way. Rotations turn their axis the same way; a 3×3 scale and shear `m` becomes `A m Aᵀ` (a diagonal stays a diagonal, its y and z swapped); inverse bind matrices turn their rotation part so and scale their translation. Normals and tangents of keys turn like directions or points. Everything is turned in f64, so a centimetre comes back within a float's rounding.

## Into the model layer
**A model** is one scene (`Gltf::model(index)`, the scene of that number; `model_count` is the number of scenes):
- **Its nodes** are walked from the scene's roots, parents before children. A node reached twice, a root with a parent, or a child with two parents is damaged.
- **Bones:** every node but a mesh's that ends a branch (no children) and is no skin's joint. A bone's parent is its node's parent; its rest transform is its node's (a `matrix` becomes a translation and a 3×3 part, no rotation). Which of translation, rotation and scale a node gives sets the transform's flags.
- **Inverse rest matrices:** a joint's comes from its skin's inverse bind matrices (the identity without them). A joint in two skins must have the same in both, or the file is refused. A bone that is no joint gets the inverse of its rest world matrix.
- **The skeleton's name:** the scene's `extras.skeleton`, else the first skin's name, else the first root's.
- **Meshes:** one mesh of the layer for each node with a mesh, in walking order, named by its mesh (else its node):
  - its primitives' vertices one after another, once for primitives that share all their attributes; a group per primitive, its indices offset (three per triangle; no indices: the vertices in order); every index inside its primitive's vertices;
  - **skinned** with a skin of two joints or more: `JOINTS_0` and `WEIGHTS_0` as they are, the mesh's bones the joints' names;
  - **a skin of one joint binds the mesh rigidly** to it (no weights);
  - **without a skin**, the mesh moves with its node when the node is a bone, else with its parent, else with nothing; the node's rest world matrix is put into its positions and normals (normals made unit length again). Animating such a node moves nothing.
- **Textures:** a primitive's material gives its group a texture of the layer: its base colour image, as a path relative to the file (`uri`, `%XX` decoded) or an image stored in it (`embedded`, by its number; the image's `name` for its name), and its alpha: `OPAQUE` draws opaque, `MASK` and `BLEND` test the alpha at one half (openm2 doesn't blend models). Each pair of image and alpha is listed once. A primitive without a material, or a material without a texture, draws untextured (white).
- **Names** are taken as the UTF-8 bytes they are.

**An animation** (`Gltf::animations`, every animation of the file):
- **Tracks** are per node a channel moves, named like it; translation fills its position, rotation its orientation, scale its scale and shear (a diagonal), or openm2's full matrices (below). A node's part moved twice is damaged.
- **Track groups:** as openm2 lists them in the animation's `extras.groups`, when it does and every node a channel moves is listed (tracks without keys keep their place, which matters: a track holds its bone even without keys, so a later group's track of the same name doesn't move it); else one group per root above the moved nodes, named by the root, in the order the channels first reach them.
- **Keys:** the sampler's input times (floats, never going down) and output values, one element a key (three for `CUBICSPLINE`: in-tangent, value, out-tangent); no interpolation is `LINEAR`. Rotations aren't normalised when read; the renderer normalises after sampling.
- **Its length:** `extras.duration`, else its last key's time.

## Floors
What characters stand on, a building's platform or a bridge's deck (`Gltf::floors`; the client's height planes, [mdatr.md](mdatr.md)), is a mesh of the model's file on a node whose `extras` has `"openm2_floor": true` (or a number other than 0: Blender writes an object's custom property `openm2_floor` = 1 there when exporting custom properties).
- **Never drawn:** a floor node's mesh is no mesh of the model. The node is a bone by the rule above (when it has children, or is a joint).
- **Read:** the triangles of every floor node of every scene (each node once), primitives of mode 4 (others are refused), placed by the node's rest world transform and turned into the renderer's frame. Other attributes and materials are ignored.

## Mesh effects
A mesh effect ([effect.md](effect.md), "Mesh effects"; the client's `.mde`) is small meshes played frame by frame: rings, flags, waterfalls. An effect file names one ([`../assets.md`](../assets.md), "Effects"), and says how long a frame shows. As glTF (`Gltf::mesh_effect`, `write_mesh_effect`):
- **A scene a frame,** in order (`frame 0`, `frame 1`, …).
- **A root node of each scene a mesh,** in order, the same number in every scene, named as the mesh (`Ring01`). Its mesh is the mesh in that frame, placed by the node's transform; a node without a mesh draws nothing that frame. A root node with children or a skin is refused, and so are scenes with other numbers of root nodes.
- **How visible** a mesh is in a frame is the node's `extras` `openm2_visibility` (1 when left out; below 0 is invisible, as the client's files have it).
- **Textures:** each mesh shows the first base colour texture its frames' meshes have, a PNG beside the file or in it. Its alpha mode doesn't matter: the effect file says how it blends.
- **Writing:** a corner's position and texture coordinate, numbered apart in the client's files, become one vertex; no normals (effects aren't lit).

Other tools see the frames as scenes: Blender imports each scene of the file as one of its scenes.

## openm2's own
Files stay valid glTF: other readers see a plain scale and skip the rest, and show floors as meshes.
- **`OPENM2_scale_shear`** (listed in `extensionsUsed` when a file has it, never required): the client's GR2 files shear (22,199 animation keys of 587 files, and 110 bones, in a TMP4 client), which glTF's three-number `scale` can't hold.
  - On a node: `{"matrix": [9 numbers]}`, the 3×3 scale and shear, column by column, used in place of `scale`, which holds its diagonal.
  - On a `scale` channel: `{"sampler": n}`, a sampler of the same animation whose outputs are `MAT3`s with the same times and interpolation, used in place of the channel's own, which holds their diagonals.
- **`extras`:** a scene's `skeleton` (the skeleton's name); an animation's `duration` (its length in seconds; a GR2 animation can last longer than its last key) and `groups` (`[{"node": root, "tracks": [nodes]}]`, every track group and track, those without keys too); a node's `openm2_floor` ([floors](#floors)) and a mesh effect's node's `openm2_visibility` ([mesh effects](#mesh-effects)).

## Writing
`write_glb` writes models of the layer, animations and PNG images:
- **Each model is a scene,** named like it, with `extras.skeleton`: its bones as nodes (a part of a rest transform written when the transform's flags set it or it isn't the default; shear in `OPENM2_scale_shear`), then a node per mesh at the root. A transform that isn't finite is refused.
- **A mesh** has one primitive per group of indexes (shorts when its vertices fit, else ints), sharing its vertex accessors (`POSITION` with its bounds, `NORMAL` and `TEXCOORD_0` when it has them). Meshes without vertices or triangles are left out (glTF has no empty meshes).
  - **Its skin:** a skinned mesh's bones as joints, with their inverse rest matrices; a mesh bound rigidly to a bone, a skin of that one joint, every vertex wholly on it; a mesh bound to no bone, no skin.
  - **Bones the skeleton lacks** are drawn as the identity, as the renderer does: a mesh naming no bone of its skeleton gets no skin, and a missing bone among others a node at rest at the root.
- **Materials,** one per image and alpha: the base colour texture, `MASK` or opaque (a texture whose alpha its own format decides is written opaque), metal 0, roughness 1, both sides. The images given come first (in the binary chunk, as `image/png`, each named by the first texture showing it); images beside the file after them, by `uri` (letters, digits and `-._~/` as they are, every other byte `%XX`).
- **Floors** are one mesh named `floor`, its triangles' corners one after another (no indices), on a node marked `openm2_floor` at the root of the first scene (a scene `floors` of their own when there is no model). A corner that isn't finite is refused.
- **Each animation** has nodes of its own, in no scene: a root per track group, named like it, and a child per track (the root itself for the first track named like the group), with `extras.duration` and `extras.groups`.
  - **Times go up,** as glTF needs: of keys sharing a time, the earlier ones move just before it by the smallest steps an f32 takes, so a jump happens within them; keys that would move below zero hold only before any animation plays and are left out.
- **Names** that are UTF-8 are written as they are; others with `%` and every byte from 0x80 as `%XX` (`RawStr::to_text`), so the same bytes always give the same name.
- **The generator** is what the caller gives: `openm2 assets import` marks its files there ([`../assets.md`](../assets.md), "Imported files").

**Through the import** (a TMP4 client): every GR2 file the actors use written and read back; the tracks' keys differ from the GR2 curves by at most 2.4 × 10⁻⁵ of the curve's largest control over 178 million values (rounding: the curves to keys, metres and back), and the skinned vertices of a warrior, a shaman, the magic gnoll (whose bones shear) and the wolf over their runs by at most 0.00005 cm.

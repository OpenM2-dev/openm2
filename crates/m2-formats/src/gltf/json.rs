//! The part of glTF 2.0's JSON that openm2 reads and writes
//! (`docs/specs/formats/gltf.md`). Members openm2 doesn't use are ignored
//! when read and never written; members it must refuse are kept as JSON
//! values, so the reader can say what a file needs.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The name of openm2's own extension: a bone's full 3×3 scale and shear,
/// which glTF's `scale` cannot hold.
pub const SCALE_SHEAR: &str = "OPENM2_scale_shear";

/// The key of a node's `extras` that makes its mesh a floor: what
/// characters stand on, never drawn (`true`, or a number other than 0, as
/// Blender writes a custom property).
pub const FLOOR: &str = "openm2_floor";

/// The key of a mesh effect's node's `extras` holding how visible its mesh
/// is in that frame (`docs/specs/formats/gltf.md`, "Mesh effects"); 1 when
/// left out.
pub const VISIBILITY: &str = "openm2_visibility";

fn is_false(value: &bool) -> bool {
    !*value
}

fn is_zero(value: &u64) -> bool {
    *value == 0
}

/// The whole document.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Document {
    /// What wrote it, and the glTF version.
    pub asset: Asset,
    /// Extensions the file uses.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extensions_used: Vec<String>,
    /// Extensions a reader must know to read the file.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extensions_required: Vec<String>,
    /// The scene shown by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scene: Option<usize>,
    /// Scenes: openm2 reads each as a model.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scenes: Vec<Scene>,
    /// Nodes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub nodes: Vec<Node>,
    /// Meshes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub meshes: Vec<Mesh>,
    /// Skins.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skins: Vec<Skin>,
    /// Animations.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub animations: Vec<Animation>,
    /// Materials.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub materials: Vec<Material>,
    /// Textures.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub textures: Vec<Texture>,
    /// Images.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<Image>,
    /// Accessors.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub accessors: Vec<Accessor>,
    /// Buffer views.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub buffer_views: Vec<BufferView>,
    /// Buffers.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub buffers: Vec<Buffer>,
}

/// `asset`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Asset {
    /// The glTF version, `2.0`.
    pub version: String,
    /// The oldest version a reader must know.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_version: Option<String>,
    /// What wrote the file. `openm2 assets import` marks its files here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generator: Option<String>,
}

/// A scene.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Scene {
    /// Name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Its root nodes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub nodes: Vec<usize>,
    /// openm2's: `skeleton`, the skeleton's name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extras: Option<Value>,
}

/// A node.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Node {
    /// Name; bones and tracks are matched by it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Child nodes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<usize>,
    /// The mesh it shows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mesh: Option<usize>,
    /// The skin its mesh is bound with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skin: Option<usize>,
    /// Translation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub translation: Option<[f32; 3]>,
    /// Rotation, a quaternion x y z w.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotation: Option<[f32; 4]>,
    /// Scale.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale: Option<[f32; 3]>,
    /// The whole transform, column-major, instead of the three above.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub matrix: Option<[f32; 16]>,
    /// Morph target weights: refused.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weights: Option<Value>,
    /// Extensions: [`SCALE_SHEAR`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extensions: Option<NodeExtensions>,
    /// openm2's: [`FLOOR`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extras: Option<Value>,
}

/// A node's extensions openm2 reads.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct NodeExtensions {
    /// [`SCALE_SHEAR`].
    #[serde(
        rename = "OPENM2_scale_shear",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub scale_shear: Option<NodeScaleShear>,
}

/// [`SCALE_SHEAR`] on a node: the scale and shear in place of `scale`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct NodeScaleShear {
    /// A 3×3 matrix, column-major.
    pub matrix: [f32; 9],
}

/// A mesh.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Mesh {
    /// Name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Its parts, each with a material.
    pub primitives: Vec<Primitive>,
    /// Morph target weights: refused.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weights: Option<Value>,
}

/// A part of a mesh.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Primitive {
    /// Vertex attributes by name (`POSITION`, `NORMAL`, …): accessors.
    pub attributes: BTreeMap<String, usize>,
    /// The index accessor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub indices: Option<usize>,
    /// The material.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub material: Option<usize>,
    /// The kind of primitive: 4 for triangles, the default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<u32>,
    /// Morph targets: refused.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub targets: Option<Value>,
}

/// A skin: the joints a mesh's vertices index.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Skin {
    /// Name: openm2 writes the skeleton's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// An accessor of one 4×4 matrix per joint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inverse_bind_matrices: Option<usize>,
    /// The joints' nodes.
    pub joints: Vec<usize>,
}

/// An animation.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Animation {
    /// Name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// What moves.
    pub channels: Vec<Channel>,
    /// How it moves.
    pub samplers: Vec<AnimationSampler>,
    /// openm2's: `duration`, the length in seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extras: Option<Value>,
}

/// One part of one node that an animation moves.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Channel {
    /// Its sampler.
    pub sampler: usize,
    /// What it moves.
    pub target: Target,
    /// Extensions: [`SCALE_SHEAR`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extensions: Option<ChannelExtensions>,
}

/// A channel's extensions openm2 reads.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ChannelExtensions {
    /// [`SCALE_SHEAR`].
    #[serde(
        rename = "OPENM2_scale_shear",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub scale_shear: Option<ChannelScaleShear>,
}

/// [`SCALE_SHEAR`] on a `scale` channel: the sampler with the full
/// matrices, in place of the channel's own.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ChannelScaleShear {
    /// A sampler of the same animation whose outputs are `MAT3`s.
    pub sampler: usize,
}

/// What a channel moves.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Target {
    /// The node.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node: Option<usize>,
    /// `translation`, `rotation`, `scale` or `weights`.
    pub path: String,
}

/// Keys: times and values.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AnimationSampler {
    /// An accessor of times in seconds.
    pub input: usize,
    /// An accessor of values.
    pub output: usize,
    /// `STEP`, `LINEAR` (the default) or `CUBICSPLINE`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interpolation: Option<String>,
}

/// A material.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Material {
    /// Name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The base colour.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pbr_metallic_roughness: Option<Pbr>,
    /// `OPAQUE` (the default), `MASK` or `BLEND`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alpha_mode: Option<String>,
    /// Drawn from both sides.
    #[serde(default, skip_serializing_if = "is_false")]
    pub double_sided: bool,
}

/// A material's metallic-roughness values.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Pbr {
    /// The base colour's texture.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_color_texture: Option<TextureInfo>,
    /// How metallic, 0 to 1 (1 when not given).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metallic_factor: Option<f32>,
    /// How rough, 0 to 1 (1 when not given).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub roughness_factor: Option<f32>,
}

/// A texture a material uses.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextureInfo {
    /// The texture.
    pub index: usize,
    /// Which texture coordinates: 0, `TEXCOORD_0`, the default.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub tex_coord: u64,
}

/// A texture: an image.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Texture {
    /// The image.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<usize>,
}

/// An image: a file beside the model, or bytes in its binary chunk.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Image {
    /// Name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// A path relative to the model's file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uri: Option<String>,
    /// The buffer view holding it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub buffer_view: Option<usize>,
    /// Its type, `image/png`, with a buffer view.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
}

/// An accessor: typed elements in a buffer view.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Accessor {
    /// The buffer view; without one every element is zero.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub buffer_view: Option<usize>,
    /// Where the first element starts in the view.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub byte_offset: u64,
    /// 5120 to 5126: byte, unsigned byte, short, unsigned short, unsigned
    /// int, float.
    pub component_type: u32,
    /// Integers that stand for 0 to 1 (or −1 to 1).
    #[serde(default, skip_serializing_if = "is_false")]
    pub normalized: bool,
    /// How many elements.
    pub count: u64,
    /// `SCALAR`, `VEC2`, `VEC3`, `VEC4`, `MAT2`, `MAT3` or `MAT4`.
    #[serde(rename = "type")]
    pub kind: String,
    /// The least value of each component.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<Vec<f32>>,
    /// The greatest value of each component.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<Vec<f32>>,
    /// Sparse storage: refused.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sparse: Option<Value>,
}

/// A buffer view: a range of a buffer.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BufferView {
    /// The buffer.
    pub buffer: usize,
    /// Where it starts.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub byte_offset: u64,
    /// How long it is.
    pub byte_length: u64,
    /// Bytes from one element to the next, when not packed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub byte_stride: Option<u64>,
}

/// A buffer: the GLB's binary chunk, or a file (refused).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Buffer {
    /// How long it is.
    pub byte_length: u64,
    /// A file holding it: refused.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uri: Option<String>,
}

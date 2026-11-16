//! openm2's renderer: plain wgpu, drawing the client's models, maps and the
//! interface offscreen (for screenshots and tests) or into a window's
//! surface.
//!
//! - [`pose`]: skeletons on the CPU. Rest poses, animations sampled at a time,
//!   parts (hair, armour) following a main skeleton by bone name, skinning
//!   matrices.
//! - [`model`]: a GR2 model turned into vertex and index lists ([`Part`]),
//!   with joints that index the part's own skeleton.
//! - [`gpu`]: the wgpu device ([`Gpu`]), headless or for a surface.
//! - [`terrain`]: a map's areas turned into meshes ([`AreaMesh`]), with
//!   their tile maps, baked shadows and water; terrain textures prepared for
//!   a texture array.
//! - [`renderer`]: the pipelines ([`Renderer`]): skinned models, terrain,
//!   water. Parts ([`GpuPart`]), texture sets ([`GpuTerrainSet`]) and areas
//!   ([`GpuArea`]) uploaded to the GPU; offscreen targets read back as
//!   images.
//! - [`ui`]: the interface pipeline ([`UiRenderer`]): an `m2_ui` draw list
//!   blended over a target, with its images and glyph pages.
//! - [`surface`]: a window's surface ([`WindowSurface`]), configured for
//!   drawing in gamma space, from a `wgpu::Surface` its caller made.
//! - [`sky`]: what is drawn behind the world ([`Sky`]): colour bands by
//!   height or a box's pictures, and drifting clouds.
//! - [`tree`]: trees grown from openm2's recipes ([`TreeBook`], [`grow`]),
//!   with textures drawn by code ([`tree_texture`]); never the client's
//!   SpeedTree files.
//! - [`effect`]: effects on the CPU: an effect script's particles and
//!   meshes over time ([`EffectInstance`]), as triangles for the effect
//!   pipeline ([`EffectGeometry`]); [`effect_pipeline`] draws them.
//! - [`camera`]: an orbit camera that frames a bounding box.
//! - [`png`]: PNG encoding for screenshots.
//!
//! The look follows the 2004-era original: textures and lighting in gamma
//! space (no sRGB conversion), one directional light plus ambient, alpha test.
//! The design is in `docs/architecture.md`, "Rendering", and
//! `docs/specs/render.md`.

#![forbid(unsafe_code)]
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

pub mod camera;
pub mod effect;
pub mod effect_pipeline;
pub mod error;
pub mod gpu;
pub mod model;
pub mod png;
pub mod pose;
pub mod renderer;
pub mod sky;
pub mod surface;
pub mod terrain;
pub mod tree;
pub mod tree_texture;
pub mod ui;

pub use camera::{Bounds, Camera};
pub use effect::{EffectGeometry, EffectInstance, EffectModel, ElementParts, View};
pub use effect_pipeline::{GpuEffects, GpuTexture};
pub use error::RenderError;
pub use gpu::{Gpu, GpuOptions};
pub use model::{Part, TextureData, Vertex};
pub use pose::{AnimationBinding, Pose};
pub use renderer::{
    Fog, GpuArea, GpuPart, GpuShineMap, GpuTerrainSet, Offscreen, Renderer, Scene, World,
};
pub use sky::{Clouds, GpuSky, Sky};
pub use surface::WindowSurface;
pub use terrain::{AreaInput, AreaMesh, MapScale, TerrainLayer};
pub use tree::{Season, TreeBook, TreeRecipe, grow};
pub use ui::{UiFilter, UiRenderer};

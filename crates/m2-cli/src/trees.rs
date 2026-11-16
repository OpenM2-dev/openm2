//! openm2's trees in the CLI: the built-in recipes, trees
//! grown for a map and placed on it, and `openm2 view tree`. What is grown
//! and why is in `docs/specs/render.md`, "Maps: trees".

use std::collections::HashMap;
use std::f32::consts::TAU;
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use clap::{Args, ValueEnum};
use glam::{Mat4, Vec3};
use m2_formats::RgbaImage;
use m2_render::renderer::OFFSCREEN_FORMAT;
use m2_render::tree::Rng;
use m2_render::tree_texture::textures;
use m2_render::{
    Bounds, Camera, Gpu, GpuOptions, Offscreen, Part, Renderer, Scene, Season, TreeBook, grow,
};

use crate::mapview::LoadedModel;
use crate::view::{MAX_SIDE, parse_size};

/// The built-in recipes.
pub(crate) const TREES: &str = include_str!("../trees.ron");

/// Trees grown of each recipe and season on a map; each spot takes one.
const VARIANTS: u64 = 4;
/// Most trees one `view tree` picture may hold.
const MAX_VARIANTS: u32 = 16;

/// The built-in recipes, read.
pub(crate) fn built_in() -> Result<TreeBook> {
    TreeBook::parse(TREES).context("the built-in trees (crates/m2-cli/trees.ron)")
}

/// A recipe's seed: the same name always grows the same trees.
fn seed_of(name: &str) -> u64 {
    name.bytes().fold(0xCBF2_9CE4_8422_2325, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01B3)
    })
}

/// The trees of a map: grown once per recipe, season and variant, placed
/// many times.
pub(crate) struct Forest {
    book: TreeBook,
    grown: HashMap<(String, Season), Vec<usize>>,
}

impl Forest {
    pub fn new(book: TreeBook) -> Self {
        Self {
            book,
            grown: HashMap::new(),
        }
    }

    /// The tree of `recipe` in `season` at `position`: its model, an index
    /// into `models` (where a recipe's trees are added the first time it is
    /// placed), and the turn and size this spot gives it, to apply before
    /// the object's own placement.
    pub fn place_recipe(
        &mut self,
        recipe: &str,
        season: Season,
        position: [f32; 3],
        models: &mut Vec<LoadedModel>,
    ) -> Result<(usize, Mat4)> {
        let key = (recipe.to_owned(), season);
        if !self.grown.contains_key(&key) {
            let recipe = self
                .book
                .recipe(&key.0)
                .with_context(|| format!("no tree recipe `{}`", key.0))?;
            let seed = seed_of(&key.0);
            let textures = textures(recipe, season, seed)?;
            let mut indexes = Vec::new();
            for variant in 1..=VARIANTS {
                let part = grow(recipe, season, seed.wrapping_add(variant));
                models.push(LoadedModel {
                    parts: vec![(part, textures.clone())],
                });
                indexes.push(models.len() - 1);
            }
            self.grown.insert(key.clone(), indexes);
        }
        let [x, y, z] = position.map(f32::to_bits);
        let mut rng = Rng::new(u64::from(x) << 32 ^ u64::from(y) ^ u64::from(z).rotate_left(17));
        let variant = (rng.next_u64() % VARIANTS) as usize;
        let index = self
            .grown
            .get(&key)
            .and_then(|indexes| indexes.get(variant))
            .copied()
            .context("a grown tree is missing")?;
        let turn = Mat4::from_rotation_z(rng.range(0.0, TAU));
        Ok((
            index,
            turn * Mat4::from_scale(Vec3::splat(rng.range(0.9, 1.1))),
        ))
    }
}

/// A season on the command line.
#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum SeasonArg {
    Summer,
    Fall,
    Winter,
    Flowers,
}

impl From<SeasonArg> for Season {
    fn from(season: SeasonArg) -> Self {
        match season {
            SeasonArg::Summer => Season::Summer,
            SeasonArg::Fall => Season::Fall,
            SeasonArg::Winter => Season::Winter,
            SeasonArg::Flowers => Season::Flowers,
        }
    }
}

/// `openm2 view tree`
#[derive(Debug, Args)]
pub struct TreeArgs {
    /// A recipe's name, or a tree file's name whose words pick the recipe
    /// and the season (`x_beech_fall.spt`)
    name: String,
    /// The season, instead of the one the name picks
    #[arg(long, value_enum)]
    season: Option<SeasonArg>,
    /// The first tree's seed; the next trees take the next numbers
    #[arg(long, default_value_t = 1)]
    seed: u64,
    /// This many trees of the recipe, in a grid
    #[arg(long, default_value_t = 1)]
    variants: u32,
    /// Recipes from this file instead of the built-in ones (RON, like
    /// crates/m2-cli/trees.ron)
    #[arg(long, value_name = "FILE")]
    trees: Option<PathBuf>,
    /// The PNG to write
    #[arg(long, value_name = "FILE")]
    screenshot: PathBuf,
    /// Size of the picture (of each tree with --variants), WIDTHxHEIGHT
    #[arg(long, default_value = "512x512", value_parser = parse_size)]
    size: (u32, u32),
    /// Camera angle around the tree in degrees
    #[arg(long, default_value_t = 0.0, allow_negative_numbers = true)]
    yaw: f32,
    /// Camera height angle in degrees; 90 looks straight down
    #[arg(long, default_value_t = 10.0, allow_negative_numbers = true)]
    pitch: f32,
    /// Camera distance in metres, instead of framing the trees
    #[arg(long)]
    distance: Option<f32>,
    /// Use a software adapter (no GPU needed; slower)
    #[arg(long)]
    software: bool,
}

/// The box around a part's vertices.
fn bounds(part: &Part) -> Option<Bounds> {
    let mut points = part.vertices.iter().map(|v| Vec3::from(v.position));
    let first = points.next()?;
    Some(points.fold(
        Bounds {
            min: first,
            max: first,
        },
        Bounds::including,
    ))
}

/// Runs `view tree`.
pub fn view_tree(args: TreeArgs) -> Result<ExitCode> {
    if args.variants == 0 || args.variants > MAX_VARIANTS {
        bail!("--variants must be from 1 to {MAX_VARIANTS}");
    }
    let (width, height) = args.size;
    let columns = (args.variants as f32).sqrt().ceil() as u32;
    let rows = args.variants.div_ceil(columns);
    if width * columns > MAX_SIDE || height * rows > MAX_SIDE {
        bail!("the grid of {columns}×{rows} trees is over {MAX_SIDE} pixels; use a smaller --size");
    }
    let book = match &args.trees {
        Some(path) => {
            let text = std::fs::read_to_string(path)
                .with_context(|| format!("reading {}", path.display()))?;
            TreeBook::parse(&text).with_context(|| format!("reading {}", path.display()))?
        }
        None => built_in()?,
    };
    let (name, picked) = if book.recipe(&args.name).is_some() {
        (args.name.clone(), Season::Summer)
    } else {
        let (name, season) = book.pick(args.name.as_bytes());
        eprintln!("`{}` picks `{name}` in {}", args.name, season.name());
        (name.to_owned(), season)
    };
    let season = args.season.map_or(picked, Season::from);
    let recipe = book
        .recipe(&name)
        .with_context(|| format!("no tree recipe `{name}`"))?;

    let seeds: Vec<u64> = (0..u64::from(args.variants))
        .map(|k| args.seed.wrapping_add(k))
        .collect();
    let parts: Vec<Part> = seeds.iter().map(|&s| grow(recipe, season, s)).collect();
    let all = parts
        .iter()
        .filter_map(bounds)
        .reduce(Bounds::union)
        .context("the trees have no vertices")?;

    let gpu = Gpu::headless(GpuOptions {
        software: args.software,
        ..GpuOptions::default()
    })?;
    let renderer = Renderer::new(&gpu, OFFSCREEN_FORMAT, gpu.sample_count(OFFSCREEN_FORMAT))?;
    let textures = textures(recipe, season, seed_of(&name))?;
    let target = Offscreen::new(&gpu, &renderer, width, height)?;
    let mut camera = Camera::framing(
        all,
        args.yaw.to_radians(),
        args.pitch.to_radians(),
        width as f32 / height as f32,
    );
    if let Some(metres) = args.distance {
        camera.distance = metres * 100.0;
    }
    let mut scene = Scene::new(camera);
    scene.background = [0.55, 0.66, 0.78, 1.0];
    let mut sheet = RgbaImage::filled(width * columns, height * rows, [140, 168, 199, 255])?;
    for (k, part) in (0u32..).zip(&parts) {
        let uploaded = renderer.upload(&gpu, part, &textures)?;
        let image = target.render(&gpu, &renderer, &scene, &[&uploaded])?;
        sheet.blit(&image, k % columns * width, k / columns * height);
    }
    std::fs::write(&args.screenshot, m2_render::png::encode(&sheet)?)
        .with_context(|| format!("writing {}", args.screenshot.display()))?;

    let vertices: usize = parts.iter().map(|p| p.vertices.len()).sum();
    let triangles: usize = parts.iter().map(|p| p.indices.len() / 3).sum();
    let seeds = match (seeds.first(), seeds.last()) {
        (Some(first), Some(last)) if first != last => format!("seeds {first} to {last}"),
        _ => format!("seed {}", args.seed),
    };
    println!(
        "wrote {} ({}×{}): `{name}` in {}, {seeds}: {} vertices, {} triangles, up to {:.1} m tall, with {}",
        args.screenshot.display(),
        sheet.width(),
        sheet.height(),
        season.name(),
        vertices / parts.len().max(1),
        triangles / parts.len().max(1),
        all.max.z / 100.0,
        gpu.adapter_name()
    );
    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use m2_render::tree::MAX_TREE_VERTICES;

    #[test]
    fn every_built_in_recipe_grows_in_every_season() {
        let book = built_in().unwrap();
        for (name, recipe) in &book.recipes {
            for season in Season::ALL {
                let part = grow(recipe, season, seed_of(name));
                assert!(!part.vertices.is_empty(), "{name} in {season:?}");
                assert!(part.vertices.len() <= MAX_TREE_VERTICES, "{name}");
                let textures = textures(recipe, season, seed_of(name)).unwrap();
                assert_eq!(textures.len(), 2, "{name}");
            }
        }
    }

    #[test]
    fn the_built_in_words_pick_recipes_and_seasons() {
        let book = built_in().unwrap();
        for (file, recipe, season) in [
            ("d:/ymir work/tree/x1_redoak_rt2.spt", "oak", Season::Summer),
            ("x2_coconutpalm_rt_01.spt", "palm", Season::Summer),
            ("x2_palmetto_rt_01.spt", "palmetto", Season::Summer),
            ("x1_beech_rt_winter02.spt", "beech", Season::Winter),
            (
                "x3_umbrellathorn_rt_flowers2.spt",
                "acacia",
                Season::Flowers,
            ),
            ("x2_japanesemaple_rt_fall.spt", "maple", Season::Fall),
            ("christmastree.spt", "spruce", Season::Summer),
            ("unknown.spt", "beech", Season::Summer),
        ] {
            assert_eq!(book.pick(file.as_bytes()), (recipe, season), "{file}");
        }
    }

    #[test]
    fn a_spot_always_gets_the_same_tree() {
        let mut forest = Forest::new(built_in().unwrap());
        let mut models = Vec::new();
        let spot = [100.0, 200.0, 0.0];
        let first = forest
            .place_recipe("beech", Season::Summer, spot, &mut models)
            .unwrap();
        let again = forest
            .place_recipe("beech", Season::Summer, spot, &mut models)
            .unwrap();
        assert_eq!(first, again);
        assert_eq!(models.len(), VARIANTS as usize);
        forest
            .place_recipe("beech", Season::Fall, [0.0; 3], &mut models)
            .unwrap();
        assert_eq!(models.len(), 2 * VARIANTS as usize);
    }
}

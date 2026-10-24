//! Name tables and path/reference rewriting for the 1.13 flattening.
//!
//! The rename tables come from ResourcePackConverter (`1_13` keys) plus
//! `data/flattening/renames.json`. Downgrade tables are inverted from the forward ones
//! because the shipped backwards tables are not true inverses.

use std::collections::{BTreeMap, HashMap};
use std::sync::OnceLock;

use serde::Deserialize;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Direction {
    /// 1.12 layout to 1.13 layout.
    Up,
    /// 1.13 layout to 1.12 layout.
    Down,
}

/// What a resource reference points at; decides which directory and table apply.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum RefKind {
    /// A texture path relative to `textures/`, e.g. `blocks/stone`.
    Texture,
    /// A model path relative to `models/`, e.g. `block/cube_all`.
    Model,
    /// A `model` value in a blockstate file. 1.12 resolves these relative to `models/block/`,
    /// 1.13 relative to `models/`.
    BlockstateModel,
}

#[derive(Deserialize)]
struct RpcNames {
    #[serde(rename = "1_13")]
    v1_13: BTreeMap<String, String>,
}

#[derive(Deserialize)]
struct RpcIds {
    regular: BTreeMap<String, String>,
}

#[derive(Deserialize)]
struct InverseOverrides {
    block_textures: BTreeMap<String, String>,
    block_models: BTreeMap<String, String>,
    item_textures: BTreeMap<String, String>,
    item_models: BTreeMap<String, String>,
}

#[derive(Deserialize)]
struct Supplement {
    dropped_block_entries: Vec<String>,
    blockstate_files: BTreeMap<String, String>,
    entity_textures: BTreeMap<String, String>,
    inverse_overrides: InverseOverrides,
}

fn parse<T: for<'de> Deserialize<'de>>(json: &str) -> T {
    serde_json::from_str(json).expect("vendored mapping data is valid JSON")
}

/// Inverts `forward`. Names that several old names merged into resolve to the explicit
/// override, else to the alphabetically first old name, so the result is deterministic.
fn invert(
    forward: &BTreeMap<String, String>,
    overrides: &BTreeMap<String, String>,
) -> HashMap<String, String> {
    let mut inverse = HashMap::new();
    for (old, new) in forward {
        inverse.entry(new.clone()).or_insert_with(|| old.clone());
    }
    inverse.extend(overrides.iter().map(|(k, v)| (k.clone(), v.clone())));
    inverse
}

fn to_hash(map: &BTreeMap<String, String>) -> HashMap<String, String> {
    map.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
}

/// Rename tables for one conversion direction. Keys are file stems (no directory, no extension),
/// except `entity_textures` whose keys are paths below `textures/entity/`.
pub struct Names {
    direction: Direction,
    block_textures: HashMap<String, String>,
    block_models: HashMap<String, String>,
    item_textures: HashMap<String, String>,
    item_models: HashMap<String, String>,
    blockstates: HashMap<String, String>,
    entity_textures: HashMap<String, String>,
}

pub fn names(direction: Direction) -> &'static Names {
    static UP: OnceLock<Names> = OnceLock::new();
    static DOWN: OnceLock<Names> = OnceLock::new();
    match direction {
        Direction::Up => UP.get_or_init(|| Names::build(Direction::Up)),
        Direction::Down => DOWN.get_or_init(|| Names::build(Direction::Down)),
    }
}

impl Names {
    fn build(direction: Direction) -> Self {
        let supplement: Supplement = parse(include_str!("../../../data/flattening/renames.json"));
        let mut blocks =
            parse::<RpcNames>(include_str!("../../../data/rpc/forwards/blocks.json")).v1_13;
        for dropped in &supplement.dropped_block_entries {
            blocks.remove(dropped);
        }
        let items = parse::<RpcNames>(include_str!("../../../data/rpc/forwards/items.json")).v1_13;
        let ov = &supplement.inverse_overrides;

        match direction {
            Direction::Up => {
                let mut blockstates = to_hash(&blocks);
                blockstates.extend(supplement.blockstate_files);
                Self {
                    direction,
                    block_textures: to_hash(&blocks),
                    block_models: to_hash(&blocks),
                    item_textures: to_hash(&items),
                    item_models: to_hash(&items),
                    blockstates,
                    entity_textures: to_hash(&supplement.entity_textures),
                }
            }
            Direction::Down => {
                let none = BTreeMap::new();
                let mut blockstates = invert(&blocks, &ov.block_models);
                blockstates.extend(invert(&supplement.blockstate_files, &none));
                Self {
                    direction,
                    block_textures: invert(&blocks, &ov.block_textures),
                    block_models: invert(&blocks, &ov.block_models),
                    item_textures: invert(&items, &ov.item_textures),
                    item_models: invert(&items, &ov.item_models),
                    blockstates,
                    entity_textures: invert(&supplement.entity_textures, &none),
                }
            }
        }
    }

    /// Directory names of block and item textures in the target layout.
    fn texture_dirs(&self) -> (&'static str, &'static str) {
        match self.direction {
            Direction::Up => ("block", "item"),
            Direction::Down => ("blocks", "items"),
        }
    }

    /// Where a pack file ends up. Paths outside `assets/` are returned unchanged.
    ///
    /// Upgrading also lowercases the path and strips parentheses, since 1.13 only allows
    /// `[a-z0-9/._-]` in resource locations. Only the `minecraft` namespace gets vanilla
    /// names remapped; other namespaces just follow the directory renames.
    pub fn retarget(&self, path: &str) -> String {
        let Some(tail) = path.strip_prefix("assets/") else {
            return path.to_string();
        };
        let tail = match self.direction {
            Direction::Up => normalize(tail),
            Direction::Down => tail.to_string(),
        };
        let Some((namespace, rest)) = tail.split_once('/') else {
            return path.to_string();
        };
        let mut rest = self.swap_directories(rest);
        if namespace == "minecraft" {
            rest = self.rename_leaf(&rest);
        }
        format!("assets/{namespace}/{rest}")
    }

    fn swap_directories(&self, rest: &str) -> String {
        const UP: [(&str, &str); 5] = [
            ("textures/blocks/", "textures/block/"),
            ("textures/items/", "textures/item/"),
            ("models/blocks/", "models/block/"),
            ("models/items/", "models/item/"),
            ("mcpatcher/", "optifine/"),
        ];
        const DOWN: [(&str, &str); 3] = [
            ("textures/block/", "textures/blocks/"),
            ("textures/item/", "textures/items/"),
            ("optifine/", "mcpatcher/"),
        ];
        let table: &[(&str, &str)] = match self.direction {
            Direction::Up => &UP,
            Direction::Down => &DOWN,
        };
        for (from, to) in table {
            if let Some(tail) = rest.strip_prefix(from) {
                return format!("{to}{tail}");
            }
        }
        rest.to_string()
    }

    /// Renames the vanilla file name of `rest` (a path below `assets/<ns>/`, directories already swapped).
    fn rename_leaf(&self, rest: &str) -> String {
        let (block_dir, item_dir) = self.texture_dirs();
        if let Some(entity) = rest.strip_prefix("textures/entity/") {
            let (dir, file) = entity.rsplit_once('/').unwrap_or(("", entity));
            let (stem, ext) = split_extension(file);
            let key = if dir.is_empty() {
                stem.to_string()
            } else {
                format!("{dir}/{stem}")
            };
            return match self.entity_textures.get(&key) {
                Some(new) => format!("textures/entity/{new}{ext}"),
                None => rest.to_string(),
            };
        }
        let Some((parent, file)) = rest.rsplit_once('/') else {
            return rest.to_string();
        };
        let table = if parent == format!("textures/{block_dir}") {
            &self.block_textures
        } else if parent == format!("textures/{item_dir}") {
            &self.item_textures
        } else if parent == "models/block" {
            &self.block_models
        } else if parent == "models/item" {
            &self.item_models
        } else if parent == "blockstates" {
            &self.blockstates
        } else {
            return rest.to_string();
        };
        let (stem, ext) = split_extension(file);
        match table.get(stem) {
            Some(new) => format!("{parent}/{new}{ext}"),
            None => rest.to_string(),
        }
    }

    /// Rewrites a resource reference (`#var` references are left alone) so it points at the
    /// file `retarget` produced.
    pub fn remap_ref(&self, reference: &str, kind: RefKind) -> String {
        if reference.starts_with('#') {
            return reference.to_string();
        }
        let reference = match self.direction {
            Direction::Up => normalize(reference),
            Direction::Down => reference.to_string(),
        };
        let (namespace, path) = match reference.split_once(':') {
            Some((namespace, path)) => (Some(namespace), path),
            None => (None, reference.as_str()),
        };
        let vanilla = namespace.is_none_or(|n| n == "minecraft");
        let path = if kind == RefKind::BlockstateModel {
            self.remap_blockstate_model(path, vanilla)
        } else {
            self.remap_path(path, vanilla, kind)
        };
        match namespace {
            Some(namespace) => format!("{namespace}:{path}"),
            None => path,
        }
    }

    fn remap_blockstate_model(&self, path: &str, vanilla: bool) -> String {
        let model = match self.direction {
            Direction::Up => {
                let path = path
                    .strip_prefix("blocks/")
                    .map_or(path.to_string(), |t| format!("block/{t}"));
                if path.starts_with("block/") {
                    path
                } else {
                    format!("block/{path}")
                }
            }
            Direction::Down => path.strip_prefix("block/").unwrap_or(path).to_string(),
        };
        let prefix_len = if self.direction == Direction::Up {
            "block/".len()
        } else {
            0
        };
        let (prefix, stem) = model.split_at(prefix_len);
        match self.block_models.get(stem) {
            Some(new) if vanilla && !stem.contains('/') => format!("{prefix}{new}"),
            _ => model,
        }
    }

    fn remap_path(&self, path: &str, vanilla: bool, kind: RefKind) -> String {
        let Some((dir, tail)) = path.split_once('/') else {
            return path.to_string();
        };
        let (block_dir, item_dir) = self.texture_dirs();
        let (new_dir, table) = match (kind, dir) {
            (RefKind::Texture, "blocks" | "block") => (block_dir, &self.block_textures),
            (RefKind::Texture, "items" | "item") => (item_dir, &self.item_textures),
            (RefKind::Texture, "entity") => {
                return match self.entity_textures.get(tail) {
                    Some(new) if vanilla => format!("entity/{new}"),
                    _ => path.to_string(),
                };
            }
            (RefKind::Model, "blocks" | "block") => ("block", &self.block_models),
            (RefKind::Model, "items" | "item") => ("item", &self.item_models),
            _ => return path.to_string(),
        };
        let tail = match table.get(tail) {
            Some(new) if vanilla && !tail.contains('/') => new,
            _ => tail,
        };
        format!("{new_dir}/{tail}")
    }
}

/// Splits `stone.png.mcmeta` into (`stone`, `.png.mcmeta`).
fn split_extension(file: &str) -> (&str, &str) {
    file.find('.').map_or((file, ""), |i| file.split_at(i))
}

/// 1.13 resource locations must be lowercase and cannot contain parentheses.
pub fn normalize(name: &str) -> String {
    name.to_lowercase().replace(['(', ')'], "")
}

/// Old `tile.x.name` language keys to `block.minecraft.x` (up) or back (down).
pub fn lang_keys(direction: Direction) -> &'static HashMap<String, String> {
    static UP: OnceLock<HashMap<String, String>> = OnceLock::new();
    static DOWN: OnceLock<HashMap<String, String>> = OnceLock::new();
    let forward = || parse::<RpcNames>(include_str!("../../../data/rpc/forwards/lang.json")).v1_13;
    match direction {
        Direction::Up => UP.get_or_init(|| to_hash(&forward())),
        Direction::Down => DOWN.get_or_init(|| invert(&forward(), &BTreeMap::new())),
    }
}

/// Numeric OptiFine ids (`1:2`) to namespaced ids (up), or namespaced ids to numeric (down).
pub fn block_ids(direction: Direction) -> &'static HashMap<String, String> {
    static UP: OnceLock<HashMap<String, String>> = OnceLock::new();
    static DOWN: OnceLock<HashMap<String, String>> = OnceLock::new();
    match direction {
        Direction::Up => UP.get_or_init(|| {
            to_hash(&parse::<RpcIds>(include_str!("../../../data/rpc/forwards/ids.json")).regular)
        }),
        Direction::Down => DOWN.get_or_init(|| {
            to_hash(&parse::<RpcIds>(include_str!("../../../data/rpc/backwards/ids.json")).regular)
        }),
    }
}

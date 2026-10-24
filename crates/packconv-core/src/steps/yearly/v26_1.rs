//! 26.1: entity textures were reorganised into one folder per mob (84 renames, see
//! `data/yearly/rename_26_1_textures.tsv`). Texture references in code are unaffected, so
//! only files and the pack's own references to those textures change.

use std::cell::Cell;
use std::sync::OnceLock;

use serde_json::Value;

use super::common::{asset_files, edit_json_files, move_files, uses_item_model_type, warn_if_shaders, RefMap, MINECRAFT};
use crate::error::Result;
use crate::step::{Context, Step};
use crate::version::Version;
use crate::vfs::Pack;

/// `(old, new)` texture ids below `textures/`, without extension.
fn renames() -> &'static [(&'static str, &'static str)] {
    static TABLE: OnceLock<Vec<(&'static str, &'static str)>> = OnceLock::new();
    TABLE.get_or_init(|| {
        include_str!("../../../data/yearly/rename_26_1_textures.tsv")
            .lines()
            .filter_map(|line| {
                let (old, new) = line.trim_end().split_once('\t')?;
                Some((old.strip_suffix(".png")?, new.strip_suffix(".png")?))
            })
            .collect()
    })
}

pub struct EntityFolders;

/// Since 26.1 a model texture may be `{"sprite": ..., "force_translucent": ...}`; older games
/// only accept plain strings. Returns whether anything changed; `lost_flag` records a
/// `force_translucent: true` that had to be dropped.
fn flatten_texture_objects(model: &mut Value, lost_flag: &Cell<bool>) -> bool {
    let Some(textures) = model.get_mut("textures").and_then(Value::as_object_mut) else { return false };
    let mut changed = false;
    for texture in textures.values_mut() {
        let Some(sprite) = texture.get("sprite").and_then(Value::as_str).map(str::to_string) else { continue };
        if texture.get("force_translucent").and_then(Value::as_bool) == Some(true) {
            lost_flag.set(true);
        }
        *texture = Value::String(sprite);
        changed = true;
    }
    changed
}

fn convert(pack: &mut Pack, ctx: &mut Context, up: bool) {
    let pairs: Vec<_> = renames().iter().map(|&(old, new)| if up { (old, new) } else { (new, old) }).collect();

    let moves: Vec<(String, String)> =
        pairs.iter().map(|(from, to)| (format!("textures/{from}.png"), format!("textures/{to}.png"))).collect();
    move_files(pack, ctx, &moves);

    let refs = RefMap::new(&pairs);
    let mut files = Vec::new();
    for dir in ["models", "items", "equipment", "atlases"] {
        files.extend(asset_files(pack, dir, &[".json"]));
    }
    let lost_flag = Cell::new(false);
    edit_json_files(pack, ctx, files, |json| {
        let renamed = refs.rewrite_all(json);
        renamed | (!up && flatten_texture_objects(json, &lost_flag))
    });
    if lost_flag.get() {
        ctx.warn("force_translucent in block model textures does not exist before 26.1 and was dropped");
    }

    if up {
        let demo_background = format!("{MINECRAFT}textures/gui/demo_background.png");
        if pack.remove(&demo_background).is_some() {
            pack.remove(&format!("{demo_background}.mcmeta"));
            ctx.warn("gui/demo_background.png was removed in 26.1 (the demo screen uses the popup/background sprite); dropped it");
        }
        if uses_item_model_type(pack, &["minecraft:special", "special"]) {
            ctx.warn("26.1 moved the built-in transformations of special item models into the vanilla item definitions; add a `transformation` if yours render misplaced");
        }
    } else if pack.paths().any(|p| p.starts_with(MINECRAFT) && p.contains("textures/entity/") && (p.contains("_baby") || p.contains("/baby/"))) {
        ctx.warn("baby mob textures only exist since 26.1 and are ignored by older versions; left in the pack");
    }
    warn_if_shaders(pack, ctx, "26.1 rewrote the lightmap uniforms and split item rendering out of core/entity");
}

impl Step for EntityFolders {
    fn name(&self) -> &'static str {
        "entity texture folders"
    }

    fn version(&self) -> Version {
        Version::new(26, 1, 0)
    }

    fn upgrade(&self, pack: &mut Pack, ctx: &mut Context) -> Result<()> {
        convert(pack, ctx, true);
        Ok(())
    }

    fn downgrade(&self, pack: &mut Pack, ctx: &mut Context) -> Result<()> {
        convert(pack, ctx, false);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::steps::yearly::common::testing::*;

    #[test]
    fn table_has_all_renames_and_is_injective() {
        assert_eq!(renames().len(), 84);
        let news: std::collections::HashSet<_> = renames().iter().map(|r| r.1).collect();
        assert_eq!(news.len(), 84);
        let olds: std::collections::HashSet<_> = renames().iter().map(|r| r.0).collect();
        assert!(olds.is_disjoint(&news));
    }

    fn sample() -> Pack {
        let mut pack = Pack::new();
        pack.insert("assets/minecraft/textures/entity/cat/black.png", vec![1]);
        pack.insert("assets/minecraft/textures/entity/cat/black.png.mcmeta", vec![2]);
        pack.insert("assets/minecraft/textures/entity/bat.png", vec![3]);
        pack.insert("assets/minecraft/textures/entity/rabbit/white.png", vec![4]);
        pack.insert("assets/minecraft/textures/entity/cat/ocelot.png", vec![5]);
        pack.insert("assets/minecraft/models/item/x.json", br##"{"textures":{"layer0":"minecraft:entity/bat","p":"#layer0"}}"##.to_vec());
        pack.insert("assets/minecraft/textures/gui/demo_background.png", vec![7]);
        pack
    }

    #[test]
    fn upgrade_moves_files_and_references() {
        let mut pack = sample();
        let mut ctx = context();
        EntityFolders.upgrade(&mut pack, &mut ctx).unwrap();
        for path in [
            "textures/entity/cat/cat_black.png",
            "textures/entity/cat/cat_black.png.mcmeta",
            "textures/entity/bat/bat.png",
            "textures/entity/rabbit/rabbit_white.png",
            "textures/entity/cat/ocelot.png",
        ] {
            assert!(pack.contains(&format!("assets/minecraft/{path}")), "{path}");
        }
        assert!(!pack.contains("assets/minecraft/textures/entity/bat.png"));
        assert!(!pack.contains("assets/minecraft/textures/gui/demo_background.png"));
        let model = String::from_utf8_lossy(pack.get("assets/minecraft/models/item/x.json").unwrap()).into_owned();
        assert!(model.contains("minecraft:entity/bat/bat") && model.contains("#layer0"));
        assert_eq!(ctx.into_warnings().len(), 1);
    }

    #[test]
    fn existing_destination_is_kept() {
        let mut pack = Pack::new();
        pack.insert("assets/minecraft/textures/entity/bat.png", vec![1]);
        pack.insert("assets/minecraft/textures/entity/bat/bat.png", vec![2]);
        let mut ctx = context();
        EntityFolders.upgrade(&mut pack, &mut ctx).unwrap();
        assert_eq!(pack.get("assets/minecraft/textures/entity/bat/bat.png"), Some(&[2][..]));
        assert!(!pack.contains("assets/minecraft/textures/entity/bat.png"));
        assert_eq!(ctx.into_warnings().len(), 1);
    }

    #[test]
    fn roundtrip_restores_files() {
        let original = sample();
        let mut pack = original.clone();
        let mut ctx = context();
        EntityFolders.upgrade(&mut pack, &mut ctx).unwrap();
        EntityFolders.downgrade(&mut pack, &mut ctx).unwrap();
        for path in original.paths().filter(|p| !p.ends_with("demo_background.png")) {
            assert!(same_content(pack.get(path).unwrap_or_else(|| panic!("lost {path}")), original.get(path).unwrap()), "{path}");
        }
    }

    #[test]
    fn downgrade_flattens_texture_objects_and_warns_about_baby_textures() {
        let mut pack = Pack::new();
        pack.insert(
            "assets/minecraft/models/block/glass_like.json",
            br#"{"textures":{"all":{"sprite":"minecraft:block/glass","force_translucent":true},"p":"block/stone"}}"#.to_vec(),
        );
        pack.insert("assets/minecraft/textures/entity/cat/cat_black_baby.png", vec![1]);
        let mut ctx = context();
        EntityFolders.downgrade(&mut pack, &mut ctx).unwrap();
        let model: Value = serde_json::from_slice(pack.get("assets/minecraft/models/block/glass_like.json").unwrap()).unwrap();
        assert_eq!(model["textures"]["all"], "minecraft:block/glass");
        assert_eq!(ctx.into_warnings().len(), 2);
    }

    #[test]
    fn warns_about_special_models_and_shaders() {
        let mut pack = Pack::new();
        pack.insert("assets/minecraft/items/shield.json", br#"{"model":{"type":"minecraft:special","base":"x","model":{"type":"minecraft:shield"}}}"#.to_vec());
        pack.insert("assets/minecraft/shaders/core/item.vsh", vec![]);
        let mut ctx = context();
        EntityFolders.upgrade(&mut pack, &mut ctx).unwrap();
        assert_eq!(ctx.into_warnings().len(), 2);
    }
}

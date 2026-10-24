//! Renames of `minecraft` resources (textures, models, blockstates, lang keys) together with
//! every reference to them, shared by the steps of this era.

use std::collections::HashMap;

use serde_json::{Map, Value};

use crate::step::Context;
use crate::steps::support::Renamer;
use crate::util::{read_json, write_json};
use crate::vfs::Pack;

/// `(old, new)` pairs. Texture and model ids are written without namespace and extension
/// (`block/grass_path_top`), blockstates are block ids, lang entries are translation keys.
pub type Pairs = &'static [(&'static str, &'static str)];

/// Everything one game version renamed. The new names must be unique so that the
/// downgrade can invert them.
pub struct Renames {
    pub textures: Pairs,
    pub models: Pairs,
    pub blockstates: Pairs,
    pub lang: Pairs,
    /// Textures whose old name stays valid in the new version (the old item is still there
    /// but a new one was split off it). Upgrading copies, downgrading drops the new name.
    pub texture_copies: Pairs,
    /// Same as `texture_copies` for models.
    pub model_copies: Pairs,
}

type RefMap<'a> = HashMap<&'a str, &'a str>;

impl Renames {
    pub const NONE: Renames = Renames {
        textures: &[],
        models: &[],
        blockstates: &[],
        lang: &[],
        texture_copies: &[],
        model_copies: &[],
    };

    pub fn upgrade(&self, pack: &mut Pack, ctx: &mut Context) {
        let mut renamer = Renamer::default();
        for &(old, new) in self.textures {
            move_texture(pack, &mut renamer, old, new);
        }
        for &(old, new) in self.models {
            renamer.rename(pack, &model_path(old), &model_path(new));
        }
        for &(old, new) in self.blockstates {
            renamer.rename(pack, &blockstate_path(old), &blockstate_path(new));
        }
        renamer.report(ctx);
        for &(old, new) in self.texture_copies {
            copy_texture(pack, old, new);
        }

        let textures: RefMap = self.textures.iter().copied().collect();
        let models: RefMap = self.models.iter().copied().collect();
        let copied_textures: RefMap = self.texture_copies.iter().copied().collect();
        let copied_models: RefMap = self.model_copies.iter().copied().collect();
        // A copied model must point at the copied texture, the original keeps its own.
        for &(old, new) in self.model_copies {
            let Some(mut model) = load_json(pack, &model_path(old), ctx) else { continue };
            rewrite_model(&mut model, &copied_textures, &copied_models);
            if !pack.contains(&model_path(new)) {
                write_json(pack, &model_path(new), &model);
            }
        }
        rewrite_references(pack, ctx, &textures, &models);
        rename_lang_keys(pack, ctx, self.lang.iter().copied().collect());
    }

    pub fn downgrade(&self, pack: &mut Pack, ctx: &mut Context) {
        let mut renamer = Renamer::default();
        for &(old, new) in self.textures {
            move_texture(pack, &mut renamer, new, old);
        }
        for &(old, new) in self.models {
            renamer.rename(pack, &model_path(new), &model_path(old));
        }
        for &(old, new) in self.blockstates {
            renamer.rename(pack, &blockstate_path(new), &blockstate_path(old));
        }
        renamer.report(ctx);
        for &(old, new) in self.texture_copies {
            drop_copy(pack, &texture_files(old), &texture_files(new));
        }
        for &(old, new) in self.model_copies {
            drop_copy(pack, &[model_path(old)], &[model_path(new)]);
        }

        let textures: RefMap =
            self.textures.iter().chain(self.texture_copies).map(|&(o, n)| (n, o)).collect();
        let models: RefMap =
            self.models.iter().chain(self.model_copies).map(|&(o, n)| (n, o)).collect();
        rewrite_references(pack, ctx, &textures, &models);
        rename_lang_keys(pack, ctx, self.lang.iter().map(|&(o, n)| (n, o)).collect());
    }
}

fn texture_files(id: &str) -> [String; 2] {
    let png = format!("assets/minecraft/textures/{id}.png");
    let mcmeta = format!("{png}.mcmeta");
    [png, mcmeta]
}

fn model_path(id: &str) -> String {
    format!("assets/minecraft/models/{id}.json")
}

fn blockstate_path(id: &str) -> String {
    format!("assets/minecraft/blockstates/{id}.json")
}

/// Moves a texture together with its `.mcmeta`; an existing target is kept.
pub fn move_texture(pack: &mut Pack, renamer: &mut Renamer, from: &str, to: &str) {
    for (from, to) in texture_files(from).iter().zip(texture_files(to)) {
        renamer.rename(pack, from, &to);
    }
}

/// Copies a texture together with its `.mcmeta`, keeping an already existing target.
fn copy_texture(pack: &mut Pack, from: &str, to: &str) {
    for (from, to) in texture_files(from).iter().zip(texture_files(to)) {
        if let (Some(data), false) = (pack.get(from), pack.contains(&to)) {
            let data = data.to_vec();
            pack.insert(to, data);
        }
    }
}

/// Undoes a copy: the new file becomes the old one if that is missing, otherwise it is dropped.
fn drop_copy(pack: &mut Pack, old: &[String], new: &[String]) {
    for (old, new) in old.iter().zip(new) {
        if pack.contains(old) {
            pack.remove(new);
        } else {
            pack.rename(new, old);
        }
    }
}

/// Parses a JSON file, warning and returning `None` when it is malformed.
pub fn load_json(pack: &Pack, path: &str, ctx: &mut Context) -> Option<Value> {
    match read_json(pack, path)? {
        Ok(value) => Some(value),
        Err(err) => {
            ctx.warn(format!("{err}; file skipped"));
            None
        }
    }
}

/// Maps a resource reference such as `block/x` or `minecraft:block/x`, keeping the prefix style.
fn remap(reference: &str, map: &RefMap) -> Option<String> {
    let (prefix, id) = match reference.strip_prefix("minecraft:") {
        Some(id) => ("minecraft:", id),
        None => ("", reference),
    };
    map.get(id).map(|new| format!("{prefix}{new}"))
}

fn remap_in_place(reference: &mut Value, map: &RefMap) -> bool {
    let Some(new) = reference.as_str().and_then(|s| remap(s, map)) else { return false };
    *reference = Value::String(new);
    true
}

/// Rewrites texture variables, `parent` and override targets of a model.
fn rewrite_model(model: &mut Value, textures: &RefMap, models: &RefMap) -> bool {
    let mut changed = false;
    if let Some(Value::Object(vars)) = model.get_mut("textures") {
        for value in vars.values_mut() {
            changed |= remap_in_place(value, textures);
        }
    }
    if let Some(parent) = model.get_mut("parent") {
        changed |= remap_in_place(parent, models);
    }
    if let Some(Value::Array(overrides)) = model.get_mut("overrides") {
        for target in overrides.iter_mut().filter_map(|o| o.get_mut("model")) {
            changed |= remap_in_place(target, models);
        }
    }
    changed
}

/// Rewrites the model of every variant and multipart case; each may be an object or a list.
fn rewrite_blockstate(state: &mut Value, models: &RefMap) -> bool {
    fn apply(entry: &mut Value, models: &RefMap) -> bool {
        match entry {
            Value::Array(list) => list.iter_mut().fold(false, |acc, e| apply(e, models) | acc),
            other => other.get_mut("model").is_some_and(|m| remap_in_place(m, models)),
        }
    }
    let mut changed = false;
    if let Some(Value::Object(variants)) = state.get_mut("variants") {
        for entry in variants.values_mut() {
            changed |= apply(entry, models);
        }
    }
    if let Some(Value::Array(cases)) = state.get_mut("multipart") {
        for apply_to in cases.iter_mut().filter_map(|c| c.get_mut("apply")) {
            changed |= apply(apply_to, models);
        }
    }
    changed
}

/// Applies the reference rewrites to the models and blockstates of every namespace.
fn rewrite_references(pack: &mut Pack, ctx: &mut Context, textures: &RefMap, models: &RefMap) {
    if textures.is_empty() && models.is_empty() {
        return;
    }
    for path in pack.paths_with_prefix("assets/") {
        let is_model = path.split('/').nth(2) == Some("models");
        let is_state = path.split('/').nth(2) == Some("blockstates");
        if !(is_model || is_state) || !path.ends_with(".json") {
            continue;
        }
        let Some(mut json) = load_json(pack, &path, ctx) else { continue };
        let changed = if is_model {
            rewrite_model(&mut json, textures, models)
        } else {
            rewrite_blockstate(&mut json, models)
        };
        if changed {
            write_json(pack, &path, &json);
        }
    }
}

/// Renames translation keys in every `lang/*.json` file, keeping the key order.
fn rename_lang_keys(pack: &mut Pack, ctx: &mut Context, pairs: RefMap) {
    if pairs.is_empty() {
        return;
    }
    for path in pack.paths_with_prefix("assets/") {
        if path.split('/').nth(2) != Some("lang") || !path.ends_with(".json") {
            continue;
        }
        let Some(Value::Object(entries)) = load_json(pack, &path, ctx) else { continue };
        if !entries.keys().any(|key| pairs.contains_key(key.as_str())) {
            continue;
        }
        let renamed: Map<String, Value> = entries
            .into_iter()
            .map(|(key, value)| (pairs.get(key.as_str()).map_or(key, |new| new.to_string()), value))
            .collect();
        write_json(pack, &path, &Value::Object(renamed));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::version::Version;
    use serde_json::json;

    fn ctx() -> Context {
        Context::new(Version::new(1, 13, 0), Version::new(1, 14, 0))
    }

    fn put(pack: &mut Pack, path: &str, value: Value) {
        write_json(pack, path, &value);
    }

    fn get(pack: &Pack, path: &str) -> Value {
        read_json(pack, path).expect("file exists").expect("valid json")
    }

    const RENAMES: Renames = Renames {
        textures: &[("block/old", "block/new")],
        models: &[("block/old", "block/new")],
        blockstates: &[("old", "new")],
        lang: &[("block.minecraft.old", "block.minecraft.new")],
        texture_copies: &[("item/ink", "item/black")],
        model_copies: &[("item/ink", "item/black")],
    };

    fn sample_pack() -> Pack {
        let mut pack = Pack::new();
        pack.insert("assets/minecraft/textures/block/old.png", vec![1]);
        pack.insert("assets/minecraft/textures/block/old.png.mcmeta", vec![2]);
        pack.insert("assets/minecraft/textures/item/ink.png", vec![3]);
        put(
            &mut pack,
            "assets/minecraft/models/block/old.json",
            json!({"textures": {"all": "block/old"}}),
        );
        put(
            &mut pack,
            "assets/minecraft/models/item/ink.json",
            json!({"textures": {"layer0": "item/ink"}}),
        );
        put(
            &mut pack,
            "assets/minecraft/blockstates/old.json",
            json!({"variants": {"": [{"model": "minecraft:block/old"}]}}),
        );
        put(
            &mut pack,
            "assets/other/models/item/x.json",
            json!({"parent": "minecraft:block/old", "textures": {"a": "block/old", "b": "other:block/old", "c": "#a"}}),
        );
        put(
            &mut pack,
            "assets/minecraft/lang/en_us.json",
            json!({"a": "1", "block.minecraft.old": "Old"}),
        );
        pack
    }

    #[test]
    fn upgrade_moves_and_rewrites() {
        let mut pack = sample_pack();
        RENAMES.upgrade(&mut pack, &mut ctx());
        assert!(pack.contains("assets/minecraft/textures/block/new.png"));
        assert!(pack.contains("assets/minecraft/textures/block/new.png.mcmeta"));
        assert!(!pack.contains("assets/minecraft/textures/block/old.png"));
        assert!(pack.contains("assets/minecraft/models/block/new.json"));
        assert!(pack.contains("assets/minecraft/blockstates/new.json"));
        assert_eq!(
            get(&pack, "assets/minecraft/models/block/new.json")["textures"]["all"],
            "block/new"
        );
        let foreign = get(&pack, "assets/other/models/item/x.json");
        assert_eq!(foreign["parent"], "minecraft:block/new");
        assert_eq!(
            foreign["textures"],
            json!({"a": "block/new", "b": "other:block/old", "c": "#a"})
        );
        assert_eq!(
            get(&pack, "assets/minecraft/blockstates/new.json")["variants"][""][0]["model"],
            "minecraft:block/new"
        );
        let lang = get(&pack, "assets/minecraft/lang/en_us.json");
        assert_eq!(lang, json!({"a": "1", "block.minecraft.new": "Old"}));
    }

    #[test]
    fn upgrade_copies_keep_the_original() {
        let mut pack = sample_pack();
        RENAMES.upgrade(&mut pack, &mut ctx());
        assert_eq!(pack.get("assets/minecraft/textures/item/black.png"), Some(&[3u8][..]));
        assert!(pack.contains("assets/minecraft/textures/item/ink.png"));
        assert_eq!(
            get(&pack, "assets/minecraft/models/item/ink.json")["textures"]["layer0"],
            "item/ink"
        );
        assert_eq!(
            get(&pack, "assets/minecraft/models/item/black.json")["textures"]["layer0"],
            "item/black"
        );
    }

    #[test]
    fn roundtrip_restores_files() {
        let mut pack = sample_pack();
        RENAMES.upgrade(&mut pack, &mut ctx());
        RENAMES.downgrade(&mut pack, &mut ctx());
        let original = sample_pack();
        assert_eq!(pack.paths().collect::<Vec<_>>(), original.paths().collect::<Vec<_>>());
        assert_eq!(
            get(&pack, "assets/minecraft/models/block/old.json"),
            get(&original, "assets/minecraft/models/block/old.json")
        );
        assert_eq!(
            get(&pack, "assets/other/models/item/x.json"),
            get(&original, "assets/other/models/item/x.json")
        );
        assert_eq!(
            get(&pack, "assets/minecraft/lang/en_us.json"),
            get(&original, "assets/minecraft/lang/en_us.json")
        );
    }

    #[test]
    fn downgrade_renames_copy_when_original_is_missing() {
        let mut pack = Pack::new();
        pack.insert("assets/minecraft/textures/item/black.png", vec![9]);
        RENAMES.downgrade(&mut pack, &mut ctx());
        assert_eq!(pack.get("assets/minecraft/textures/item/ink.png"), Some(&[9u8][..]));
        assert!(!pack.contains("assets/minecraft/textures/item/black.png"));
    }

    #[test]
    fn malformed_json_is_reported_and_skipped() {
        let mut pack = sample_pack();
        pack.insert("assets/minecraft/models/block/bad.json", b"{ nope".to_vec());
        let mut ctx = ctx();
        RENAMES.upgrade(&mut pack, &mut ctx);
        assert!(pack.contains("assets/minecraft/models/block/new.json"));
        let warnings = ctx.into_warnings();
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("models/block/bad.json"));
    }

    #[test]
    fn existing_destination_is_kept_and_reported_once() {
        let mut pack = sample_pack();
        pack.insert("assets/minecraft/textures/block/new.png", vec![9]);
        put(&mut pack, "assets/minecraft/models/block/new.json", json!({"parent": "mine"}));
        let mut ctx = ctx();
        RENAMES.upgrade(&mut pack, &mut ctx);
        assert_eq!(pack.get("assets/minecraft/textures/block/new.png"), Some(&[9][..]));
        assert_eq!(get(&pack, "assets/minecraft/models/block/new.json")["parent"], "mine");
        assert!(!pack.contains("assets/minecraft/textures/block/old.png"));
        assert_eq!(ctx.into_warnings().len(), 1);
    }
}

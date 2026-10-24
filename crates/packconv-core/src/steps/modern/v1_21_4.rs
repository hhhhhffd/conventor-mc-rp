//! 1.21.4: item model definitions (`items/<id>.json`) replaced model `overrides`, equipment
//! definitions moved from `models/equipment/` to `equipment/`, and empty-slot sprites moved
//! into the GUI atlas.

use std::collections::BTreeSet;

use serde_json::{json, Value};

use super::common::{move_textures, parse_pairs, rename_ids, reversed};
use super::item_defs::{definition_to_overrides, overrides_to_definition, same_model};
use crate::error::Result;
use crate::step::{Context, Step};
use crate::util::{namespaces, read_json, write_json};
use crate::version::Version;
use crate::vfs::Pack;

const SLOT_SPRITES: &str = include_str!("../../../data/modern/slot-sprites-1.21.4.tsv");
const ELYTRA: &[(&str, &str)] = &[("item/broken_elytra", "item/elytra_broken")];

pub struct V1_21_4;

impl Step for V1_21_4 {
    fn name(&self) -> &'static str {
        "1.21.4 item model definitions"
    }

    fn version(&self) -> Version {
        Version::new(1, 21, 4)
    }

    fn upgrade(&self, pack: &mut Pack, ctx: &mut Context) -> Result<()> {
        let mut notes = BTreeSet::new();
        for ns in namespaces(pack) {
            generate_definitions(pack, ctx, &ns, &mut notes);
            pack.rename_dir(&format!("assets/{ns}/models/equipment"), &format!("assets/{ns}/equipment"));
        }
        move_textures(pack, &parse_pairs(SLOT_SPRITES));
        rename_ids(pack, ctx, ELYTRA);
        notes.into_iter().for_each(|n| ctx.warn(n));
        Ok(())
    }

    fn downgrade(&self, pack: &mut Pack, ctx: &mut Context) -> Result<()> {
        let mut notes = BTreeSet::new();
        for ns in namespaces(pack) {
            restore_overrides(pack, ctx, &ns, &mut notes);
            pack.rename_dir(&format!("assets/{ns}/equipment"), &format!("assets/{ns}/models/equipment"));
        }
        move_textures(pack, &reversed(&parse_pairs(SLOT_SPRITES)));
        rename_ids(pack, ctx, &reversed(ELYTRA));
        notes.into_iter().for_each(|n| ctx.warn(n));
        Ok(())
    }
}

/// Top-level `<dir>/<name>.json` files of a namespace, as `(path, name)`.
fn top_level_json(pack: &Pack, ns: &str, dir: &str) -> Vec<(String, String)> {
    let prefix = format!("assets/{ns}/{dir}/");
    pack.paths_with_prefix(&prefix)
        .into_iter()
        .filter_map(|path| {
            let name = path[prefix.len()..].strip_suffix(".json")?;
            (!name.contains('/')).then(|| (path.clone(), name.to_string()))
        })
        .collect()
}

/// Writes `items/<id>.json` for every item model that has `overrides`, and strips the overrides.
/// Vanilla definitions cover everything else, so models without overrides need nothing.
fn generate_definitions(pack: &mut Pack, ctx: &mut Context, ns: &str, notes: &mut BTreeSet<String>) {
    for (path, name) in top_level_json(pack, ns, "models/item") {
        let mut model = match read_json(pack, &path) {
            Some(Ok(model)) => model,
            Some(Err(e)) => {
                ctx.warn(format!("{e}; item model overrides not converted"));
                continue;
            }
            None => continue,
        };
        let Some(overrides) = model.get("overrides").and_then(Value::as_array).filter(|o| !o.is_empty()).cloned() else {
            continue;
        };
        let definition_path = format!("assets/{ns}/items/{name}.json");
        if pack.contains(&definition_path) {
            continue;
        }
        // builtin/entity models (shield, banner, chest, trident...) need a `special` model type.
        if model.get("parent").and_then(Value::as_str).is_some_and(|p| p.trim_start_matches("minecraft:") == "builtin/entity") {
            notes.insert("overrides of builtin/entity item models (shield, banner, trident, ...) cannot be converted to item definitions".into());
            continue;
        }
        let definition = overrides_to_definition(&name, &format!("{ns}:item/{name}"), &overrides, notes);
        write_json(pack, &definition_path, &definition);
        if let Some(fields) = model.as_object_mut() {
            fields.remove("overrides");
        }
        write_json(pack, &path, &model);
    }
}

/// Converts top-level item definitions back into model overrides and removes them (the
/// directory does not exist before 1.21.4).
fn restore_overrides(pack: &mut Pack, ctx: &mut Context, ns: &str, notes: &mut BTreeSet<String>) {
    if pack.paths_with_prefix(&format!("assets/{ns}/items/")).iter().any(|p| p[format!("assets/{ns}/items/").len()..].contains('/')) {
        notes.insert("item definitions in subdirectories (items/<dir>/<id>.json) have no equivalent before 1.21.4 and were left in place".into());
    }
    for (path, name) in top_level_json(pack, ns, "items") {
        let definition = match read_json(pack, &path) {
            Some(Ok(definition)) => definition,
            Some(Err(e)) => {
                ctx.warn(format!("{e}; item definition not converted"));
                continue;
            }
            None => continue,
        };
        pack.remove(&path);
        let Some(flat) = definition_to_overrides(&name, &definition, notes) else { continue };
        let model_path = format!("assets/{ns}/models/item/{name}.json");
        let own_id = format!("{ns}:item/{name}");
        if same_model(&flat.base, &own_id) {
            if flat.overrides.is_empty() {
                continue;
            }
            match read_json(pack, &model_path) {
                Some(Ok(mut model)) => {
                    if let Some(fields) = model.as_object_mut() {
                        fields.insert("overrides".into(), Value::Array(flat.overrides));
                        write_json(pack, &model_path, &model);
                    }
                }
                Some(Err(e)) => ctx.warn(format!("{e}; overrides not restored")),
                None => {
                    notes.insert("item definitions pointing at a missing item model were skipped".into());
                }
            }
        } else {
            // The definition uses another model as base: the item model becomes a redirect to it.
            let mut model = json!({"parent": flat.base});
            if !flat.overrides.is_empty() {
                model["overrides"] = Value::Array(flat.overrides);
            }
            write_json(pack, &model_path, &model);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::steps::modern::common::testing::{put_png, solid};

    fn ctx() -> Context {
        Context::new(Version::new(1, 21, 3), Version::new(1, 21, 4))
    }

    fn json_at(pack: &Pack, path: &str) -> Value {
        serde_json::from_slice(pack.get(path).unwrap_or_else(|| panic!("{path} missing"))).unwrap()
    }

    fn bow_pack() -> Pack {
        let mut pack = Pack::new();
        pack.insert(
            "assets/minecraft/models/item/bow.json",
            br#"{"parent":"item/generated","textures":{"layer0":"item/bow"},"overrides":[
                {"predicate":{"pulling":1},"model":"item/bow_pulling_0"},
                {"predicate":{"pulling":1,"pull":0.65},"model":"item/bow_pulling_1"}]}"#
                .to_vec(),
        );
        pack
    }

    #[test]
    fn generates_definitions_only_for_models_with_overrides() {
        let mut pack = bow_pack();
        pack.insert("assets/minecraft/models/item/stick.json", br#"{"parent":"item/generated"}"#.to_vec());
        V1_21_4.upgrade(&mut pack, &mut ctx()).unwrap();
        assert!(!pack.contains("assets/minecraft/items/stick.json"));
        let definition = json_at(&pack, "assets/minecraft/items/bow.json");
        assert_eq!(definition["model"]["type"], "minecraft:condition");
        assert!(json_at(&pack, "assets/minecraft/models/item/bow.json").get("overrides").is_none());
    }

    #[test]
    fn overrides_roundtrip() {
        let mut pack = bow_pack();
        V1_21_4.upgrade(&mut pack, &mut ctx()).unwrap();
        V1_21_4.downgrade(&mut pack, &mut ctx()).unwrap();
        assert!(!pack.contains("assets/minecraft/items/bow.json"));
        let model = json_at(&pack, "assets/minecraft/models/item/bow.json");
        assert_eq!(model["parent"], "item/generated");
        assert_eq!(
            model["overrides"],
            json!([
                {"predicate": {"pulling": 1}, "model": "minecraft:item/bow_pulling_0"},
                {"predicate": {"pulling": 1, "pull": 0.65}, "model": "minecraft:item/bow_pulling_1"},
            ])
        );
    }

    #[test]
    fn custom_namespace_items_use_their_namespace() {
        let mut pack = Pack::new();
        pack.insert(
            "assets/mymod/models/item/wand.json",
            br#"{"parent":"item/handheld","overrides":[{"predicate":{"custom_model_data":7},"model":"mymod:item/wand_glow"}]}"#.to_vec(),
        );
        V1_21_4.upgrade(&mut pack, &mut ctx()).unwrap();
        let definition = json_at(&pack, "assets/mymod/items/wand.json");
        assert_eq!(definition["model"]["fallback"]["model"], "mymod:item/wand");
    }

    #[test]
    fn downgrade_redirects_foreign_base_model_and_warns_for_composites() {
        let mut pack = Pack::new();
        pack.insert(
            "assets/minecraft/items/stone_sword.json",
            br#"{"model":{"type":"minecraft:model","model":"mypack:item/blade"}}"#.to_vec(),
        );
        pack.insert("assets/minecraft/items/shield.json", br#"{"model":{"type":"minecraft:special","base":"a","model":{"type":"x"}}}"#.to_vec());
        let mut ctx = ctx();
        V1_21_4.downgrade(&mut pack, &mut ctx).unwrap();
        assert_eq!(json_at(&pack, "assets/minecraft/models/item/stone_sword.json")["parent"], "mypack:item/blade");
        assert!(!pack.contains("assets/minecraft/items/shield.json"));
        assert_eq!(ctx.into_warnings().len(), 1);
    }

    #[test]
    fn moves_equipment_slots_and_elytra() {
        let mut pack = Pack::new();
        let mut ctx = ctx();
        pack.insert("assets/minecraft/models/equipment/leather.json", b"{\"layers\":{}}".to_vec());
        put_png(&mut pack, "assets/minecraft/textures/item/empty_slot_sword.png", &solid(16, 16, [1, 1, 1, 255]));
        put_png(&mut pack, "assets/minecraft/textures/item/broken_elytra.png", &solid(16, 16, [2, 2, 2, 255]));
        pack.insert(
            "assets/minecraft/models/item/broken_elytra.json",
            br#"{"parent":"item/generated","textures":{"layer0":"item/broken_elytra"}}"#.to_vec(),
        );
        V1_21_4.upgrade(&mut pack, &mut ctx).unwrap();
        assert!(pack.contains("assets/minecraft/equipment/leather.json"));
        assert!(pack.contains("assets/minecraft/textures/gui/sprites/container/slot/sword.png"));
        assert!(pack.contains("assets/minecraft/textures/item/elytra_broken.png"));
        assert_eq!(json_at(&pack, "assets/minecraft/models/item/elytra_broken.json")["textures"]["layer0"], "item/elytra_broken");
        V1_21_4.downgrade(&mut pack, &mut ctx).unwrap();
        assert!(pack.contains("assets/minecraft/models/equipment/leather.json"));
        assert!(pack.contains("assets/minecraft/textures/item/empty_slot_sword.png"));
        assert!(pack.contains("assets/minecraft/textures/item/broken_elytra.png"));
    }
}

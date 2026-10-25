//! 26.3: armor trim palettes moved to `textures/palettes/` and are referenced by palette id,
//! block model elements replaced `shade` with `shade_direction_override`, maps lost their
//! tinted marking overlay (`minecraft:map_color`, `item/filled_map_markings.png`) and two map
//! item textures were renamed.

use serde_json::{json, Value};

use super::common::{asset_files, directed, edit_json_files, move_files, Pairs, RefMap, MINECRAFT};
use crate::error::Result;
use crate::step::{Context, Step};
use crate::version::Version;
use crate::vfs::Pack;

/// Texture locations (below `textures/`) of the trim palettes before 26.3.
const OLD_PALETTES: &str = "trims/color_palettes/";
const OLD_BASE_PALETTE: &str = "trims/color_palettes/trim_palette";
/// Palette ids since 26.3; an id `<path>` resolves to `textures/palettes/<path>.png`.
const NEW_PALETTES: &str = "trim/";
const NEW_BASE_PALETTE: &str = "trim_base";

const MAP_TEXTURES: &Pairs = &[
    ("item/trial_chamber_map", "item/buried_trial_chambers_map"),
    ("item/jungle_temple_map", "item/jungle_pyramid_map"),
];

const MAP_MARKINGS: &str = "textures/item/filled_map_markings.png";

pub struct PalettesAndShade;

/// Palette files as `(from, to)` paths below `assets/minecraft/` for the requested direction.
fn palette_moves(pack: &Pack, up: bool) -> Vec<(String, String)> {
    let (dir, base) = if up {
        (format!("textures/{OLD_PALETTES}"), format!("textures/{OLD_BASE_PALETTE}.png"))
    } else {
        (format!("textures/palettes/{NEW_PALETTES}"), format!("textures/palettes/{NEW_BASE_PALETTE}.png"))
    };
    let mut moves = Vec::new();
    for path in pack.paths_with_prefix(&format!("{MINECRAFT}{dir}")) {
        let rel = &path[MINECRAFT.len()..];
        let Some(name) = rel.strip_prefix(dir.as_str()).and_then(|n| n.strip_suffix(".png")) else { continue };
        if rel == base {
            continue;
        }
        let to = if up { format!("textures/palettes/{NEW_PALETTES}{name}.png") } else { format!("textures/{OLD_PALETTES}{name}.png") };
        moves.push((rel.to_string(), to));
    }
    if pack.contains(&format!("{MINECRAFT}{base}")) {
        let to = if up { format!("textures/palettes/{NEW_BASE_PALETTE}.png") } else { format!("textures/{OLD_BASE_PALETTE}.png") };
        moves.push((base, to));
    }
    moves
}

/// Texture location (before 26.3) → palette id. Locations outside `textures/palettes/` have no
/// id unless they are the vanilla trim palettes, which this step moves.
fn location_to_palette_id(value: &str) -> Option<String> {
    let (namespace, path) = match value.split_once(':') {
        Some((namespace, path)) => (Some(namespace), path),
        None => (None, value),
    };
    let vanilla = namespace.is_none_or(|n| n == "minecraft");
    let id = if vanilla && path == OLD_BASE_PALETTE {
        NEW_BASE_PALETTE.to_string()
    } else if let Some(name) = path.strip_prefix(OLD_PALETTES).filter(|_| vanilla) {
        format!("{NEW_PALETTES}{name}")
    } else {
        path.strip_prefix("palettes/")?.to_string()
    };
    Some(match namespace {
        Some(namespace) => format!("{namespace}:{id}"),
        None => id,
    })
}

/// Palette id → texture location (before 26.3). Always possible: an id is a location below
/// `textures/palettes/`.
fn palette_id_to_location(value: &str) -> String {
    let (namespace, path) = match value.split_once(':') {
        Some((namespace, path)) => (Some(namespace), path),
        None => (None, value),
    };
    let vanilla = namespace.is_none_or(|n| n == "minecraft");
    let location = if vanilla && path == NEW_BASE_PALETTE {
        OLD_BASE_PALETTE.to_string()
    } else if let Some(name) = path.strip_prefix(NEW_PALETTES).filter(|_| vanilla) {
        format!("{OLD_PALETTES}{name}")
    } else {
        format!("palettes/{path}")
    };
    match namespace {
        Some(namespace) => format!("{namespace}:{location}"),
        None => location,
    }
}

/// Rewrites `palette_key` and `permutations` of every `paletted_permutations` atlas source.
/// Returns whether anything changed; `unmapped` counts palettes left as they were.
fn rewrite_palette_refs(atlas: &mut Value, up: bool, unmapped: &mut usize) -> bool {
    let Some(sources) = atlas.get_mut("sources").and_then(Value::as_array_mut) else { return false };
    let mut changed = false;
    for source in sources {
        let kind = source.get("type").and_then(Value::as_str).map(|t| t.trim_start_matches("minecraft:"));
        if kind != Some("paletted_permutations") {
            continue;
        }
        let mut rewrite = |value: &mut Value| {
            let Some(old) = value.as_str() else { return };
            let new = if up { location_to_palette_id(old) } else { Some(palette_id_to_location(old)) };
            match new {
                Some(new) => {
                    changed |= new != old;
                    *value = Value::String(new);
                }
                None => *unmapped += 1,
            }
        };
        if let Some(key) = source.get_mut("palette_key") {
            rewrite(key);
        }
        if let Some(permutations) = source.get_mut("permutations").and_then(Value::as_object_mut) {
            permutations.values_mut().for_each(&mut rewrite);
        }
    }
    changed
}

/// `shade: false` became `shade_direction_override: "up"` (every face is lit like a top face,
/// i.e. unshaded). Returns whether anything changed; `lost` counts overrides that have no
/// `shade` equivalent.
fn convert_shade(model: &mut Value, up: bool, lost: &mut usize) -> bool {
    let Some(elements) = model.get_mut("elements").and_then(Value::as_array_mut) else { return false };
    let mut changed = false;
    for element in elements.iter_mut().filter_map(Value::as_object_mut) {
        if up {
            let Some(shade) = element.remove("shade") else { continue };
            if shade == json!(false) {
                element.insert("shade_direction_override".into(), json!("up"));
            }
            changed = true;
        } else {
            let Some(direction) = element.remove("shade_direction_override") else { continue };
            if direction == json!("up") {
                element.insert("shade".into(), json!(false));
            } else {
                *lost += 1;
            }
            changed = true;
        }
    }
    changed
}

/// Replaces `minecraft:map_color` tint sources by an untinted constant, keeping the indices
/// of the other tints. Returns whether anything changed.
fn drop_map_color(value: &mut Value) -> bool {
    match value {
        Value::Object(map) => {
            let is_map_color =
                map.get("type").and_then(Value::as_str).is_some_and(|t| t.trim_start_matches("minecraft:") == "map_color");
            if is_map_color {
                *value = json!({"type": "minecraft:constant", "value": -1});
                true
            } else {
                map.values_mut().fold(false, |changed, v| drop_map_color(v) | changed)
            }
        }
        Value::Array(items) => items.iter_mut().fold(false, |changed, v| drop_map_color(v) | changed),
        _ => false,
    }
}

fn convert(pack: &mut Pack, ctx: &mut Context, up: bool) {
    let moves = palette_moves(pack, up);
    move_files(pack, ctx, &moves);
    let mut unmapped = 0;
    let atlases = asset_files(pack, "atlases", &[".json"]);
    edit_json_files(pack, ctx, atlases, |json| rewrite_palette_refs(json, up, &mut unmapped));
    if unmapped > 0 {
        ctx.warn(format!(
            "{unmapped} armor trim palette(s) in atlases are outside textures/palettes/, which 26.3 requires; left unchanged"
        ));
    }

    let map_textures = directed(MAP_TEXTURES, up);
    let texture_moves: Vec<_> =
        map_textures.iter().map(|(from, to)| (format!("textures/{from}.png"), format!("textures/{to}.png"))).collect();
    move_files(pack, ctx, &texture_moves);
    let refs = RefMap::new(&map_textures);
    let mut lost_shade = 0;
    let models = asset_files(pack, "models", &[".json"]);
    edit_json_files(pack, ctx, models, |json| refs.rewrite_model_textures(json) | convert_shade(json, up, &mut lost_shade));
    if lost_shade > 0 {
        ctx.warn(format!(
            "{lost_shade} block model element(s) use a shade_direction_override other than \"up\", which does not exist before 26.3; dropped"
        ));
    }
    let atlases = asset_files(pack, "atlases", &[".json"]);
    edit_json_files(pack, ctx, atlases, |json| refs.rewrite_all(json));

    if up {
        let markings = format!("{MINECRAFT}{MAP_MARKINGS}");
        if pack.remove(&markings).is_some() {
            pack.remove(&format!("{markings}.mcmeta"));
            ctx.warn("item/filled_map_markings.png was removed in 26.3 (maps are no longer drawn with a tinted overlay); dropped it");
        }
        let mut dropped_tint = false;
        let items = asset_files(pack, "items", &[".json"]);
        edit_json_files(pack, ctx, items, |json| {
            let changed = drop_map_color(json);
            dropped_tint |= changed;
            changed
        });
        if dropped_tint {
            ctx.warn("the minecraft:map_color tint source was removed in 26.3; replaced it with an untinted constant");
        }
    } else {
        let mut dropped_overrides = false;
        let equipment = asset_files(pack, "equipment", &[".json"]);
        edit_json_files(pack, ctx, equipment, |json| {
            let changed = json.as_object_mut().is_some_and(|map| map.remove("trim_overrides").is_some());
            dropped_overrides |= changed;
            changed
        });
        if dropped_overrides {
            ctx.warn("trim_overrides in equipment assets only exist since 26.3 (older versions use override_armor_assets in data pack trim materials); dropped");
        }
    }
}

impl Step for PalettesAndShade {
    fn name(&self) -> &'static str {
        "trim palettes, model shade and maps"
    }

    fn version(&self) -> Version {
        Version::new(26, 3, 0)
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

    fn sample() -> Pack {
        let mut pack = Pack::new();
        pack.insert("assets/minecraft/textures/trims/color_palettes/trim_palette.png", vec![1]);
        pack.insert("assets/minecraft/textures/trims/color_palettes/quartz.png", vec![2]);
        pack.insert("assets/minecraft/textures/trims/color_palettes/gold_darker.png", vec![3]);
        pack.insert("assets/minecraft/textures/item/jungle_temple_map.png", vec![4]);
        pack.insert(
            "assets/minecraft/atlases/armor_trims.json",
            br#"{"sources":[{"type":"paletted_permutations","textures":["trims/items/helmet_trim"],
                "palette_key":"trims/color_palettes/trim_palette",
                "permutations":{"quartz":"minecraft:trims/color_palettes/quartz","gold_darker":"trims/color_palettes/gold_darker"}}]}"#
                .to_vec(),
        );
        pack.insert(
            "assets/minecraft/models/block/lamp.json",
            br#"{"textures":{"all":"block/stone"},"elements":[{"from":[0,0,0],"to":[16,16,16],"shade":false},{"from":[0,0,0],"to":[1,1,1]}]}"#.to_vec(),
        );
        pack.insert("assets/minecraft/models/item/jungle_map.json", br#"{"parent":"item/generated","textures":{"layer0":"minecraft:item/jungle_temple_map"}}"#.to_vec());
        pack
    }

    fn json(pack: &Pack, path: &str) -> Value {
        serde_json::from_slice(pack.get(path).unwrap_or_else(|| panic!("missing {path}"))).unwrap()
    }

    #[test]
    fn upgrade_moves_palettes_and_rewrites_references() {
        let mut pack = sample();
        let mut ctx = context();
        PalettesAndShade.upgrade(&mut pack, &mut ctx).unwrap();
        for path in [
            "textures/palettes/trim_base.png",
            "textures/palettes/trim/quartz.png",
            "textures/palettes/trim/gold_darker.png",
            "textures/item/jungle_pyramid_map.png",
        ] {
            assert!(pack.contains(&format!("assets/minecraft/{path}")), "{path}");
        }
        assert!(pack.paths_with_prefix("assets/minecraft/textures/trims/color_palettes/").is_empty());
        let atlas = json(&pack, "assets/minecraft/atlases/armor_trims.json");
        assert_eq!(atlas["sources"][0]["palette_key"], "trim_base");
        assert_eq!(atlas["sources"][0]["permutations"]["quartz"], "minecraft:trim/quartz");
        assert_eq!(atlas["sources"][0]["textures"][0], "trims/items/helmet_trim");
        let model = json(&pack, "assets/minecraft/models/block/lamp.json");
        assert_eq!(model["elements"][0]["shade_direction_override"], "up");
        assert!(model["elements"][0].get("shade").is_none());
        assert!(model["elements"][1].get("shade_direction_override").is_none());
        let item = json(&pack, "assets/minecraft/models/item/jungle_map.json");
        assert_eq!(item["textures"]["layer0"], "minecraft:item/jungle_pyramid_map");
        assert!(ctx.into_warnings().is_empty());
    }

    #[test]
    fn roundtrip_restores_pack() {
        let original = sample();
        let mut pack = original.clone();
        let mut ctx = context();
        PalettesAndShade.upgrade(&mut pack, &mut ctx).unwrap();
        PalettesAndShade.downgrade(&mut pack, &mut ctx).unwrap();
        for path in original.paths() {
            let after = pack.get(path).unwrap_or_else(|| panic!("lost {path}"));
            assert!(same_content(original.get(path).unwrap(), after), "{path}");
        }
        assert_eq!(pack.len(), original.len());
        assert!(ctx.into_warnings().is_empty());
    }

    #[test]
    fn upgrade_drops_map_markings() {
        let mut pack = Pack::new();
        pack.insert("assets/minecraft/textures/item/filled_map_markings.png", vec![1]);
        pack.insert(
            "assets/minecraft/items/filled_map.json",
            br#"{"model":{"type":"minecraft:model","model":"minecraft:item/filled_map","tints":[{"type":"minecraft:constant","value":-1},{"type":"minecraft:map_color","default":4603950}]}}"#.to_vec(),
        );
        let mut ctx = context();
        PalettesAndShade.upgrade(&mut pack, &mut ctx).unwrap();
        assert!(!pack.contains("assets/minecraft/textures/item/filled_map_markings.png"));
        let item = json(&pack, "assets/minecraft/items/filled_map.json");
        assert_eq!(item["model"]["tints"][1], json!({"type": "minecraft:constant", "value": -1}));
        assert_eq!(ctx.into_warnings().len(), 2);
    }

    #[test]
    fn downgrade_maps_palette_ids_and_drops_new_fields() {
        let mut pack = Pack::new();
        pack.insert(
            "assets/minecraft/atlases/armor_trims.json",
            br#"{"sources":[{"type":"minecraft:paletted_permutations","textures":[],"palette_key":"trim_base","permutations":{"x":"custom/x"}}]}"#.to_vec(),
        );
        pack.insert(
            "assets/minecraft/models/block/x.json",
            br#"{"elements":[{"from":[0,0,0],"to":[1,1,1],"shade_direction_override":"north"}]}"#.to_vec(),
        );
        pack.insert(
            "assets/minecraft/equipment/iron.json",
            br#"{"layers":{},"trim_overrides":[{"when":{"material":"minecraft:iron"},"palette":"minecraft:trim/iron_darker"}]}"#.to_vec(),
        );
        let mut ctx = context();
        PalettesAndShade.downgrade(&mut pack, &mut ctx).unwrap();
        let atlas = json(&pack, "assets/minecraft/atlases/armor_trims.json");
        assert_eq!(atlas["sources"][0]["palette_key"], "trims/color_palettes/trim_palette");
        assert_eq!(atlas["sources"][0]["permutations"]["x"], "palettes/custom/x");
        assert!(json(&pack, "assets/minecraft/models/block/x.json")["elements"][0].get("shade_direction_override").is_none());
        assert!(json(&pack, "assets/minecraft/equipment/iron.json").get("trim_overrides").is_none());
        assert_eq!(ctx.into_warnings().len(), 2);
    }

    #[test]
    fn warns_about_palettes_outside_the_palette_folder() {
        let mut pack = Pack::new();
        pack.insert(
            "assets/minecraft/atlases/armor_trims.json",
            br#"{"sources":[{"type":"paletted_permutations","textures":[],"palette_key":"trims/color_palettes/trim_palette","permutations":{"x":"custom/x"}}]}"#.to_vec(),
        );
        let mut ctx = context();
        PalettesAndShade.upgrade(&mut pack, &mut ctx).unwrap();
        assert_eq!(json(&pack, "assets/minecraft/atlases/armor_trims.json")["sources"][0]["permutations"]["x"], "custom/x");
        assert_eq!(ctx.into_warnings().len(), 1);
    }
}

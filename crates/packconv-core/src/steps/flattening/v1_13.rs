//! 1.13 "The Flattening": lowercase flat resource names, singular `block`/`item` directories,
//! `.lang` replaced by JSON, `mcpatcher` renamed `optifine`, and the pixel changes in `images`.

use serde_json::{Map, Value};

use super::maps::{names, Direction, Names, RefKind};
use super::{images, lang, optifine};
use crate::error::Result;
use crate::step::{Context, Step};
use crate::util::{read_json, write_json};
use crate::version::Version;
use crate::vfs::Pack;

pub struct Flattening;

impl Step for Flattening {
    fn name(&self) -> &'static str {
        "flattening-1.13"
    }

    fn version(&self) -> Version {
        Version::new(1, 13, 0)
    }

    fn upgrade(&self, pack: &mut Pack, ctx: &mut Context) -> Result<()> {
        convert(pack, ctx, Direction::Up)
    }

    fn downgrade(&self, pack: &mut Pack, ctx: &mut Context) -> Result<()> {
        convert(pack, ctx, Direction::Down)
    }
}

fn convert(pack: &mut Pack, ctx: &mut Context, direction: Direction) -> Result<()> {
    let names = names(direction);
    move_files(pack, ctx, names, direction);
    rewrite_json(pack, ctx, "models/", |json| rewrite_model(json, names));
    rewrite_json(pack, ctx, "blockstates/", |json| {
        rewrite_blockstate(json, names, direction)
    });
    if direction == Direction::Up {
        rewrite_json(pack, ctx, "sounds.json", lowercase_sounds);
    }
    if !pack
        .paths_with_prefix("assets/minecraft/blockstates/")
        .is_empty()
    {
        ctx.warn("blockstates were only renamed and re-pathed: block states that the flattening split into separate blocks are not converted");
    }
    lang::convert(pack, ctx, direction);
    optifine::convert(pack, ctx, direction);
    images::convert(pack, ctx, direction)
}

/// Moves every file to the path `Names::retarget` gives it. All sources are removed before
/// anything is written, so name swaps like `grass` -> `grass_block` and `tallgrass` -> `grass`
/// cannot clobber each other.
fn move_files(pack: &mut Pack, ctx: &mut Context, names: &Names, direction: Direction) {
    let mut moved = Vec::new();
    for path in pack.paths_with_prefix("assets/") {
        let target = names.retarget(&path);
        if target != path {
            if let Some(data) = pack.remove(&path) {
                moved.push((path, target, data));
            }
        }
    }
    let mut collision = None;
    let renamed = moved
        .iter()
        .any(|(source, target, _)| source.rsplit('/').next() != target.rsplit('/').next());
    for (source, target, data) in moved {
        if pack.contains(&target) {
            collision = Some((source, target.clone()));
        }
        pack.insert(target, data);
    }
    if let Some((source, target)) = collision {
        ctx.warn(format!(
            "several files map to {target} (for example {source}); the last one was kept"
        ));
    }
    if direction == Direction::Down && renamed {
        ctx.warn(
            "1.13 names that merged several 1.12 names were mapped back to one canonical 1.12 name",
        );
    }
}

/// Applies `edit` to every JSON file whose path below `assets/<ns>/` starts with `section`,
/// rewriting only the files that changed.
fn rewrite_json(pack: &mut Pack, ctx: &mut Context, section: &str, edit: impl Fn(&mut Value)) {
    let paths = pack
        .paths_with_prefix("assets/")
        .into_iter()
        .filter(|path| {
            let in_section = path
                .splitn(3, '/')
                .nth(2)
                .is_some_and(|rest| rest.starts_with(section));
            in_section && path.ends_with(".json")
        });
    for path in paths.collect::<Vec<_>>() {
        let mut json = match read_json(pack, &path) {
            Some(Ok(json)) => json,
            Some(Err(e)) => {
                ctx.warn(format!("skipping malformed JSON: {e}"));
                continue;
            }
            None => continue,
        };
        let before = json.clone();
        edit(&mut json);
        if json != before {
            write_json(pack, &path, &json);
        }
    }
}

fn remap_string(value: &mut Value, names: &Names, kind: RefKind) {
    if let Value::String(reference) = value {
        *reference = names.remap_ref(reference, kind);
    }
}

fn rewrite_model(model: &mut Value, names: &Names) {
    if let Some(parent) = model.get_mut("parent") {
        remap_string(parent, names, RefKind::Model);
    }
    if let Some(Value::Object(textures)) = model.get_mut("textures") {
        for texture in textures.values_mut() {
            remap_string(texture, names, RefKind::Texture);
        }
    }
    if let Some(Value::Array(overrides)) = model.get_mut("overrides") {
        for entry in overrides {
            if let Some(target) = entry.get_mut("model") {
                remap_string(target, names, RefKind::Model);
            }
        }
    }
}

fn rewrite_blockstate(state: &mut Value, names: &Names, direction: Direction) {
    // The variant without properties is called `normal` in 1.12 and `` in 1.13.
    let (from_key, to_key) = match direction {
        Direction::Up => ("normal", ""),
        Direction::Down => ("", "normal"),
    };
    if let Some(Value::Object(variants)) = state.get_mut("variants") {
        // Rebuilt rather than patched in place to keep the key order.
        *variants = std::mem::take(variants)
            .into_iter()
            .map(|(key, value)| {
                (
                    if key == from_key {
                        to_key.to_string()
                    } else {
                        key
                    },
                    value,
                )
            })
            .collect::<Map<_, _>>();
        for variant in variants.values_mut() {
            rewrite_variant(variant, names);
        }
    }
    if let Some(Value::Array(parts)) = state.get_mut("multipart") {
        for part in parts {
            if let Some(apply) = part.get_mut("apply") {
                rewrite_variant(apply, names);
            }
        }
    }
}

/// A variant is one model object or an array of them.
fn rewrite_variant(variant: &mut Value, names: &Names) {
    match variant {
        Value::Array(models) => models
            .iter_mut()
            .for_each(|model| rewrite_variant(model, names)),
        model => {
            if let Some(reference) = model.get_mut("model") {
                remap_string(reference, names, RefKind::BlockstateModel);
            }
        }
    }
}

/// Sound event names and sound paths must be lowercase in 1.13 (the files themselves were
/// lowercased by `move_files`).
fn lowercase_sounds(sounds: &mut Value) {
    let Value::Object(events) = sounds else {
        return;
    };
    *events = std::mem::take(events)
        .into_iter()
        .map(|(event, mut definition)| {
            if let Some(Value::Array(entries)) = definition.get_mut("sounds") {
                for entry in entries {
                    match entry {
                        Value::String(name) => *name = name.to_lowercase(),
                        Value::Object(object) => {
                            if let Some(Value::String(name)) = object.get_mut("name") {
                                *name = name.to_lowercase();
                            }
                        }
                        _ => {}
                    }
                }
            }
            (event.to_lowercase(), definition)
        })
        .collect();
}

#[cfg(test)]
mod tests {
    use image::{Rgba, RgbaImage};
    use serde_json::json;

    use super::*;
    use crate::util::{read_png, write_png};

    fn text(pack: &Pack, path: &str) -> String {
        String::from_utf8(
            pack.get(path)
                .unwrap_or_else(|| panic!("missing {path}"))
                .to_vec(),
        )
        .unwrap()
    }

    fn json_at(pack: &Pack, path: &str) -> Value {
        read_json(pack, path)
            .unwrap_or_else(|| panic!("missing {path}"))
            .unwrap()
    }

    fn put_json(pack: &mut Pack, path: &str, value: Value) {
        write_json(pack, path, &value);
    }

    fn put_png(pack: &mut Pack, path: &str, size: u32, color: [u8; 4]) {
        write_png(pack, path, &RgbaImage::from_pixel(size, size, Rgba(color))).unwrap();
    }

    fn ctx() -> Context {
        Context::new(Version::new(1, 12, 2), Version::new(1, 13, 0))
    }

    /// A small 1.12 pack touching every part of the step.
    fn pack_1_12() -> Pack {
        let mut pack = Pack::new();
        put_png(
            &mut pack,
            "assets/minecraft/textures/blocks/planks_oak.png",
            2,
            [1, 2, 3, 255],
        );
        pack.insert(
            "assets/minecraft/textures/blocks/planks_oak.png.mcmeta",
            b"{}".to_vec(),
        );
        put_png(
            &mut pack,
            "assets/minecraft/textures/items/apple_golden.png",
            2,
            [4, 5, 6, 255],
        );
        put_png(
            &mut pack,
            "assets/minecraft/textures/blocks/Custom(Tex).png",
            2,
            [7, 7, 7, 255],
        );
        put_png(
            &mut pack,
            "assets/minecraft/textures/entity/endercrystal/endercrystal.png",
            2,
            [8, 8, 8, 255],
        );
        put_png(
            &mut pack,
            "assets/mymod/textures/blocks/bar.png",
            2,
            [9, 9, 9, 255],
        );
        put_json(
            &mut pack,
            "assets/minecraft/models/block/half_slab_stone.json",
            json!({"parent": "block/cube_all", "textures": {"all": "blocks/planks_oak", "side": "#all", "t": "mymod:blocks/bar"}}),
        );
        put_json(
            &mut pack,
            "assets/minecraft/models/item/golden.json",
            json!({"parent": "builtin/generated", "textures": {"layer0": "minecraft:items/apple_golden"},
                   "overrides": [{"predicate": {"pull": 1}, "model": "item/apple_golden"}]}),
        );
        put_json(
            &mut pack,
            "assets/minecraft/blockstates/planks.json",
            json!({"variants": {"normal": [{"model": "half_slab_stone"}, {"model": "half_slab_stone", "y": 90}]}}),
        );
        put_json(
            &mut pack,
            "assets/minecraft/blockstates/fence.json",
            json!({"multipart": [{"apply": {"model": "planks_oak"}}, {"when": {"north": "true"}, "apply": {"model": "planks_oak"}}]}),
        );
        pack.insert(
            "assets/minecraft/lang/En_US.lang",
            b"# comment\ntile.stone.stone.name=Stone\nmod.key=Hi = there\n".to_vec(),
        );
        put_json(
            &mut pack,
            "assets/minecraft/sounds.json",
            json!({"Foo.Bar": {"sounds": ["Mob/Cow", {"name": "Mob/Pig", "volume": 0.5}]}}),
        );
        pack.insert(
            "assets/minecraft/mcpatcher/ctm/stone/stone.properties",
            b"matchBlocks=1 minecraft:dirt 5:1\nmetadata=0\ntiles=0-3\ntexture=textures/blocks/planks_oak.png\nname=Keep Me\n".to_vec(),
        );
        put_png(
            &mut pack,
            "assets/minecraft/textures/blocks/water_still.png",
            1,
            [40, 60, 240, 200],
        );
        put_png(
            &mut pack,
            "assets/minecraft/textures/particle/particles.png",
            4,
            [1, 1, 1, 255],
        );
        let mut icons = RgbaImage::new(32, 32);
        icons.put_pixel(9, 1, Rgba([10, 20, 30, 255])); // cell 1
        icons.put_pixel(9, 17, Rgba([40, 50, 60, 255])); // cell 9
        write_png(
            &mut pack,
            "assets/minecraft/textures/map/map_icons.png",
            &icons,
        )
        .unwrap();
        pack
    }

    #[test]
    fn upgrades_a_1_12_pack() {
        let mut pack = pack_1_12();
        let mut ctx = ctx();
        Flattening.upgrade(&mut pack, &mut ctx).unwrap();

        assert!(pack.contains("assets/minecraft/textures/block/oak_planks.png"));
        assert!(pack.contains("assets/minecraft/textures/block/oak_planks.png.mcmeta"));
        assert!(pack.contains("assets/minecraft/textures/item/golden_apple.png"));
        assert!(pack.contains("assets/minecraft/textures/block/customtex.png"));
        assert!(pack.contains("assets/minecraft/textures/entity/end_crystal/end_crystal.png"));
        assert!(pack.contains("assets/mymod/textures/block/bar.png"));
        assert!(!pack.contains("assets/minecraft/textures/blocks/planks_oak.png"));

        let model = json_at(&pack, "assets/minecraft/models/block/half_slab_stone.json");
        assert_eq!(model["parent"], "block/cube_all");
        assert_eq!(model["textures"]["all"], "block/oak_planks");
        assert_eq!(model["textures"]["side"], "#all");
        assert_eq!(model["textures"]["t"], "mymod:block/bar");
        let item = json_at(&pack, "assets/minecraft/models/item/golden.json");
        assert_eq!(item["textures"]["layer0"], "minecraft:item/golden_apple");
        assert_eq!(item["overrides"][0]["model"], "item/golden_apple");

        let state = json_at(&pack, "assets/minecraft/blockstates/planks.json");
        assert_eq!(state["variants"][""][1]["model"], "block/half_slab_stone");
        assert!(state["variants"].get("normal").is_none());
        let fence = json_at(&pack, "assets/minecraft/blockstates/fence.json");
        assert_eq!(fence["multipart"][1]["apply"]["model"], "block/oak_planks");

        let lang = json_at(&pack, "assets/minecraft/lang/en_us.json");
        assert_eq!(lang["block.minecraft.stone"], "Stone");
        assert_eq!(lang["mod.key"], "Hi = there");
        assert!(lang.get("tile.stone.stone.name").is_none());
        assert!(!pack.contains("assets/minecraft/lang/en_us.lang"));

        let sounds = json_at(&pack, "assets/minecraft/sounds.json");
        assert_eq!(sounds["foo.bar"]["sounds"][0], "mob/cow");
        assert_eq!(sounds["foo.bar"]["sounds"][1]["name"], "mob/pig");

        let properties = text(
            &pack,
            "assets/minecraft/optifine/ctm/stone/stone.properties",
        );
        assert_eq!(
            properties,
            "matchBlocks=minecraft:stone minecraft:dirt minecraft:spruce_planks\ntiles=0-3\ntexture=textures/block/oak_planks.png\nname=Keep Me\n"
        );

        let water = read_png(&pack, "assets/minecraft/textures/block/water_still.png")
            .unwrap()
            .unwrap();
        let [r, g, b, a] = water.get_pixel(0, 0).0;
        assert!(r == g && g == b && a == 200);

        let particles = read_png(&pack, "assets/minecraft/textures/particle/particles.png")
            .unwrap()
            .unwrap();
        assert_eq!(
            (
                particles.width(),
                particles.get_pixel(3, 3).0,
                particles.get_pixel(4, 4).0
            ),
            (8, [1, 1, 1, 255], [0, 0, 0, 0])
        );

        let icons = read_png(&pack, "assets/minecraft/textures/map/map_icons.png")
            .unwrap()
            .unwrap();
        assert_eq!(icons.width(), 128);
        assert_eq!(icons.get_pixel(8 + 1, 1).0, [10, 20, 30, 255]);
        assert_eq!(icons.get_pixel(72 + 1, 1).0, [40, 50, 60, 255]);
    }

    #[test]
    fn swapped_names_do_not_clobber_each_other() {
        let mut pack = Pack::new();
        put_png(
            &mut pack,
            "assets/minecraft/textures/blocks/grass.png",
            1,
            [1, 0, 0, 255],
        );
        put_png(
            &mut pack,
            "assets/minecraft/textures/blocks/tallgrass.png",
            1,
            [2, 0, 0, 255],
        );
        Flattening.upgrade(&mut pack, &mut ctx()).unwrap();
        let red = |path: &str| read_png(&pack, path).unwrap().unwrap().get_pixel(0, 0).0[0];
        assert_eq!(red("assets/minecraft/textures/block/grass_block.png"), 1);
        assert_eq!(red("assets/minecraft/textures/block/grass.png"), 2);
    }

    #[test]
    fn downgrade_inverts_upgrade() {
        let mut pack = pack_1_12();
        let mut ctx = ctx();
        Flattening.upgrade(&mut pack, &mut ctx).unwrap();
        Flattening.downgrade(&mut pack, &mut ctx).unwrap();

        assert!(pack.contains("assets/minecraft/textures/blocks/planks_oak.png"));
        assert!(pack.contains("assets/minecraft/textures/blocks/planks_oak.png.mcmeta"));
        assert!(pack.contains("assets/minecraft/textures/items/apple_golden.png"));
        assert!(pack.contains("assets/minecraft/textures/entity/endercrystal/endercrystal.png"));
        assert!(pack.contains("assets/mymod/textures/blocks/bar.png"));

        let model = json_at(&pack, "assets/minecraft/models/block/half_slab_stone.json");
        assert_eq!(model["textures"]["all"], "blocks/planks_oak");
        assert_eq!(model["textures"]["t"], "mymod:blocks/bar");
        let item = json_at(&pack, "assets/minecraft/models/item/golden.json");
        assert_eq!(item["textures"]["layer0"], "minecraft:items/apple_golden");
        assert_eq!(item["overrides"][0]["model"], "item/apple_golden");

        let state = json_at(&pack, "assets/minecraft/blockstates/planks.json");
        assert_eq!(state["variants"]["normal"][0]["model"], "half_slab_stone");
        let fence = json_at(&pack, "assets/minecraft/blockstates/fence.json");
        assert_eq!(fence["multipart"][0]["apply"]["model"], "planks_oak");

        let lang = text(&pack, "assets/minecraft/lang/en_us.lang");
        assert_eq!(lang, "tile.stone.stone.name=Stone\nmod.key=Hi = there\n");
        assert!(!pack.contains("assets/minecraft/lang/en_us.json"));

        let properties = text(
            &pack,
            "assets/minecraft/mcpatcher/ctm/stone/stone.properties",
        );
        assert!(
            properties.starts_with("matchBlocks=1 3 5:1\n"),
            "{properties}"
        );
        assert!(properties.contains("texture=textures/blocks/planks_oak.png\n"));

        let particles = read_png(&pack, "assets/minecraft/textures/particle/particles.png")
            .unwrap()
            .unwrap();
        assert_eq!(particles.width(), 4);
        let icons = read_png(&pack, "assets/minecraft/textures/map/map_icons.png")
            .unwrap()
            .unwrap();
        assert_eq!(icons.width(), 32);
        assert_eq!(icons.get_pixel(9, 1).0, [10, 20, 30, 255]);
        assert_eq!(icons.get_pixel(9, 17).0, [40, 50, 60, 255]);

        let water = read_png(&pack, "assets/minecraft/textures/blocks/water_still.png")
            .unwrap()
            .unwrap();
        let [r, g, b, a] = water.get_pixel(0, 0).0;
        assert!(b > g && g > r && a == 200);
    }

    #[test]
    fn downgrade_picks_a_canonical_name_for_merged_names() {
        let mut pack = Pack::new();
        put_png(
            &mut pack,
            "assets/minecraft/textures/block/oak_pressure_plate.png",
            1,
            [1, 1, 1, 255],
        );
        put_png(
            &mut pack,
            "assets/minecraft/textures/item/ink_sac.png",
            1,
            [1, 1, 1, 255],
        );
        let mut ctx = Context::new(Version::new(1, 13, 0), Version::new(1, 12, 2));
        Flattening.downgrade(&mut pack, &mut ctx).unwrap();
        assert!(pack.contains("assets/minecraft/textures/blocks/wooden_pressure_plate.png"));
        assert!(pack.contains("assets/minecraft/textures/items/dye_powder_black.png"));
        assert_eq!(ctx.into_warnings().len(), 1);
    }

    #[test]
    fn bad_files_are_skipped_with_a_warning() {
        let mut pack = Pack::new();
        pack.insert(
            "assets/minecraft/models/block/broken.json",
            b"{not json".to_vec(),
        );
        pack.insert(
            "assets/minecraft/textures/block/water_still.png",
            b"not a png".to_vec(),
        );
        pack.insert("assets/minecraft/lang/xx_yy.json", b"[]".to_vec());
        let mut ctx = Context::new(Version::new(1, 13, 0), Version::new(1, 12, 2));
        Flattening.downgrade(&mut pack, &mut ctx).unwrap();
        let warnings = ctx.into_warnings();
        assert_eq!(warnings.len(), 3, "{warnings:?}");
        assert!(pack.contains("assets/minecraft/models/block/broken.json"));
    }
}

//! 1.21.11: celestial textures moved into `environment/celestial/` (the moon sheet became
//! eight files), and item sprites got their own `items` texture atlas.

use image::{imageops, RgbaImage};
use serde_json::{json, Value};

use super::common::{move_files, MINECRAFT};
use crate::error::Result;
use crate::step::{Context, Step};
use crate::steps::support::{canvas, load_png, save_png, PixelBudget};
use crate::util::{namespaces, read_json, write_json};
use crate::version::Version;
use crate::vfs::Pack;

const MOON_SHEET: &str = "textures/environment/moon_phases.png";

/// Moon tiles in the order of the old 4x2 sheet (row-major); verified against the vanilla
/// 1.21.10 sheet and the 1.21.11 tile files.
const MOON_TILES: [&str; 8] = [
    "full_moon",
    "waning_gibbous",
    "third_quarter",
    "waning_crescent",
    "new_moon",
    "waxing_crescent",
    "first_quarter",
    "waxing_gibbous",
];
const SHEET_COLUMNS: u32 = 4;
const SHEET_ROWS: u32 = 2;

const CELESTIAL_FILES: [(&str, &str); 2] = [
    ("textures/environment/sun.png", "textures/environment/celestial/sun.png"),
    ("textures/environment/end_flash.png", "textures/environment/celestial/end_flash.png"),
];

const EFFECT_SPRITES_OLD: [&str; 2] = [
    "textures/gui/sprites/container/inventory/effect_background_large.png",
    "textures/gui/sprites/container/inventory/effect_background_small.png",
];
const EFFECT_SPRITES_NEW: [&str; 2] = [
    "textures/gui/sprites/container/inventory/effect_background.png",
    "textures/gui/sprites/container/inventory/effect_background_ambient.png",
];

pub struct Celestials;

fn moon_tile_path(name: &str) -> String {
    format!("textures/environment/celestial/moon/{name}.png")
}

fn full(path: &str) -> String {
    format!("{MINECRAFT}{path}")
}

fn split_moon_sheet(pack: &mut Pack, ctx: &mut Context) {
    let sheet_path = full(MOON_SHEET);
    let Some(sheet) = load_png(pack, ctx, &sheet_path) else { return };
    let (tile_w, tile_h) = (sheet.width() / SHEET_COLUMNS, sheet.height() / SHEET_ROWS);
    if tile_w == 0 || tile_h == 0 || sheet.width() % SHEET_COLUMNS != 0 || sheet.height() % SHEET_ROWS != 0 {
        ctx.warn(format!("{sheet_path} is not a 4x2 tile sheet; moon phases not split"));
        return;
    }
    // The sheet is only removed when every tile exists afterwards.
    let mut complete = true;
    for (i, name) in (0u32..).zip(MOON_TILES) {
        let tile_path = full(&moon_tile_path(name));
        if pack.contains(&tile_path) {
            continue;
        }
        let (column, row) = (i % SHEET_COLUMNS, i / SHEET_COLUMNS);
        let tile = imageops::crop_imm(&sheet, column * tile_w, row * tile_h, tile_w, tile_h).to_image();
        complete &= save_png(pack, ctx, &tile_path, &tile);
    }
    if complete {
        pack.remove(&sheet_path);
        pack.remove(&format!("{sheet_path}.mcmeta"));
    }
}

fn join_moon_tiles(pack: &mut Pack, ctx: &mut Context) {
    let mut tiles: Vec<RgbaImage> = Vec::new();
    let mut budget = PixelBudget::new();
    for name in MOON_TILES {
        let path = full(&moon_tile_path(name));
        if let Some(tile) = load_png(pack, ctx, &path) {
            if budget.take(&tile) {
                tiles.push(tile);
            } else {
                ctx.warn(format!("{path}: too large to join into the moon sheet"));
            }
        }
    }
    if tiles.is_empty() {
        return;
    }
    let (tile_w, tile_h) = tiles[0].dimensions();
    if tiles.len() != MOON_TILES.len() || tiles.iter().any(|t| t.dimensions() != (tile_w, tile_h)) {
        // A sheet with holes would hide the missing phases instead of falling back to vanilla.
        ctx.warn("moon phases need all 8 tiles of equal size to build moon_phases.png; tiles left unconverted");
        return;
    }
    if pack.contains(&full(MOON_SHEET)) {
        ctx.warn("moon_phases.png already exists and was kept; the moon tiles were left unconverted");
        return;
    }
    let Some(mut sheet) = canvas(ctx, MOON_SHEET, tile_w.checked_mul(SHEET_COLUMNS), tile_h.checked_mul(SHEET_ROWS)) else {
        return;
    };
    for (i, tile) in (0u32..).zip(&tiles) {
        imageops::replace(&mut sheet, tile, i64::from(i % SHEET_COLUMNS * tile_w), i64::from(i / SHEET_COLUMNS * tile_h));
    }
    if !save_png(pack, ctx, &full(MOON_SHEET), &sheet) {
        return;
    }
    for name in MOON_TILES {
        let path = full(&moon_tile_path(name));
        pack.remove(&path);
        pack.remove(&format!("{path}.mcmeta"));
    }
}

/// Atlas sources that put item sprites into the stitched atlas: the item folder itself and
/// the armor trim permutations (which only feed item models).
fn is_item_source(source: &Value) -> bool {
    let is_item_path = |key: &str| {
        source
            .get(key)
            .and_then(Value::as_str)
            .map(|s| s.trim_start_matches("minecraft:"))
            .is_some_and(|s| s == "item" || s.starts_with("item/"))
    };
    match source.get("type").and_then(Value::as_str).map(|t| t.trim_start_matches("minecraft:")) {
        Some("paletted_permutations") => true,
        Some("directory") => is_item_path("source"),
        Some("single") => is_item_path("resource"),
        _ => false,
    }
}

fn sources_of(json: &mut Value) -> Option<&mut Vec<Value>> {
    json.get_mut("sources").and_then(Value::as_array_mut)
}

/// Reads `atlases/<name>.json` of `namespace`; malformed files are reported and skipped.
fn read_atlas(pack: &Pack, ctx: &mut Context, path: &str) -> Option<Value> {
    match read_json(pack, path)? {
        Ok(json) => Some(json),
        Err(e) => {
            ctx.warn(format!("{e}; atlas left unchanged"));
            None
        }
    }
}

/// Moves item sources out of custom `blocks` atlases into the new `items` atlas. Atlases of
/// different packs are merged by the game, so splitting a pack's own list is lossless.
fn split_item_atlas(pack: &mut Pack, ctx: &mut Context) {
    let mut moved = false;
    for namespace in namespaces(pack) {
        let blocks_path = format!("assets/{namespace}/atlases/blocks.json");
        let items_path = format!("assets/{namespace}/atlases/items.json");
        let Some(mut blocks) = read_atlas(pack, ctx, &blocks_path) else { continue };
        let Some(sources) = sources_of(&mut blocks) else { continue };
        let (items, rest): (Vec<Value>, Vec<Value>) = std::mem::take(sources).into_iter().partition(is_item_source);
        if items.is_empty() {
            continue;
        }
        *sources = rest;
        let mut atlas = read_atlas(pack, ctx, &items_path).unwrap_or_else(|| json!({ "sources": [] }));
        match sources_of(&mut atlas) {
            Some(existing) => existing.extend(items),
            None => atlas = json!({ "sources": items }),
        }
        write_json(pack, &items_path, &atlas);
        if blocks["sources"].as_array().is_some_and(Vec::is_empty) {
            pack.remove(&blocks_path);
        } else {
            write_json(pack, &blocks_path, &blocks);
        }
        moved = true;
    }
    if moved {
        ctx.warn("item sources of atlases/blocks.json were moved to atlases/items.json (1.21.11 split the item atlas)");
    }
}

/// Folds a custom `items` atlas back into `blocks`, which stitched item sprites before 1.21.11.
fn merge_item_atlas(pack: &mut Pack, ctx: &mut Context) {
    for namespace in namespaces(pack) {
        let blocks_path = format!("assets/{namespace}/atlases/blocks.json");
        let items_path = format!("assets/{namespace}/atlases/items.json");
        let Some(mut items) = read_atlas(pack, ctx, &items_path) else { continue };
        let Some(item_sources) = sources_of(&mut items) else { continue };
        let item_sources = std::mem::take(item_sources);
        let mut blocks = read_atlas(pack, ctx, &blocks_path).unwrap_or_else(|| json!({ "sources": [] }));
        let Some(sources) = sources_of(&mut blocks) else { continue };
        for source in item_sources {
            if !sources.contains(&source) {
                sources.push(source);
            }
        }
        write_json(pack, &blocks_path, &blocks);
        pack.remove(&items_path);
    }
}

fn warn_if_any(pack: &Pack, ctx: &mut Context, paths: &[&str], message: &str) {
    if paths.iter().any(|p| pack.contains(&full(p))) {
        ctx.warn(message);
    }
}

impl Step for Celestials {
    fn name(&self) -> &'static str {
        "celestial textures and item atlas"
    }

    fn version(&self) -> Version {
        Version::new(1, 21, 11)
    }

    fn upgrade(&self, pack: &mut Pack, ctx: &mut Context) -> Result<()> {
        move_files(pack, ctx, &CELESTIAL_FILES);
        split_moon_sheet(pack, ctx);
        split_item_atlas(pack, ctx);
        warn_if_any(
            pack,
            ctx,
            &EFFECT_SPRITES_OLD,
            "effect_background_large/small were replaced by nine-sliced effect_background/effect_background_ambient; the old sprites are unused and were not converted",
        );
        Ok(())
    }

    fn downgrade(&self, pack: &mut Pack, ctx: &mut Context) -> Result<()> {
        let reversed: Vec<_> = CELESTIAL_FILES.iter().map(|&(old, new)| (new, old)).collect();
        move_files(pack, ctx, &reversed);
        join_moon_tiles(pack, ctx);
        merge_item_atlas(pack, ctx);
        warn_if_any(
            pack,
            ctx,
            &EFFECT_SPRITES_NEW,
            "effect_background/effect_background_ambient have no pre-1.21.11 equivalent (large/small sprites); they were not converted",
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::steps::yearly::common::testing::*;

    fn moon_sheet() -> RgbaImage {
        // Each 2x2 tile gets a distinct color so a wrong tile order is detected.
        let mut sheet = RgbaImage::new(8, 4);
        for y in 0..4u32 {
            for x in 0..8u32 {
                let tile = (x / 2 + 4 * (y / 2)) as u8;
                sheet.put_pixel(x, y, image::Rgba([tile * 10, 0, 0, 255]));
            }
        }
        sheet
    }

    #[test]
    fn moon_sheet_splits_in_row_major_order_and_joins_back() {
        let mut pack = Pack::new();
        insert_png(&mut pack, "assets/minecraft/textures/environment/moon_phases.png", &moon_sheet());
        pack.insert("assets/minecraft/textures/environment/sun.png", vec![9]);
        let mut ctx = context();
        Celestials.upgrade(&mut pack, &mut ctx).unwrap();
        assert!(!pack.contains("assets/minecraft/textures/environment/moon_phases.png"));
        assert!(pack.contains("assets/minecraft/textures/environment/celestial/sun.png"));
        let tile = |name: &str| png(&pack, &format!("assets/minecraft/textures/environment/celestial/moon/{name}.png"));
        assert_eq!(tile("full_moon").dimensions(), (2, 2));
        assert_eq!(tile("full_moon").get_pixel(0, 0).0[0], 0);
        assert_eq!(tile("waning_crescent").get_pixel(0, 0).0[0], 30);
        assert_eq!(tile("new_moon").get_pixel(0, 0).0[0], 40);
        assert_eq!(tile("waxing_gibbous").get_pixel(0, 0).0[0], 70);

        Celestials.downgrade(&mut pack, &mut ctx).unwrap();
        assert_eq!(png(&pack, "assets/minecraft/textures/environment/moon_phases.png"), moon_sheet());
        assert!(pack.contains("assets/minecraft/textures/environment/sun.png"));
        assert!(pack.paths().all(|p| !p.contains("celestial")));
        assert!(ctx.into_warnings().is_empty());
    }

    #[test]
    fn incomplete_moon_tiles_are_not_joined() {
        let mut pack = Pack::new();
        insert_png(&mut pack, "assets/minecraft/textures/environment/celestial/moon/new_moon.png", &solid(2, 2, [1, 2, 3, 255]));
        let mut ctx = context();
        Celestials.downgrade(&mut pack, &mut ctx).unwrap();
        assert!(!pack.contains("assets/minecraft/textures/environment/moon_phases.png"));
        assert!(pack.contains("assets/minecraft/textures/environment/celestial/moon/new_moon.png"));
        assert_eq!(ctx.into_warnings().len(), 1);
    }

    fn read(pack: &Pack, path: &str) -> Value {
        read_json(pack, path).unwrap().unwrap()
    }

    #[test]
    fn item_sources_move_between_atlases() {
        let blocks = json!({"sources": [
            {"type": "minecraft:directory", "source": "block", "prefix": "block/"},
            {"type": "minecraft:directory", "source": "item", "prefix": "item/"},
            {"type": "minecraft:paletted_permutations", "textures": [], "palette_key": "k", "permutations": {}},
            {"type": "minecraft:single", "resource": "minecraft:item/custom"}
        ]});
        let mut pack = Pack::new();
        write_json(&mut pack, "assets/minecraft/atlases/blocks.json", &blocks);
        let mut ctx = context();
        Celestials.upgrade(&mut pack, &mut ctx).unwrap();
        assert_eq!(read(&pack, "assets/minecraft/atlases/blocks.json")["sources"].as_array().unwrap().len(), 1);
        assert_eq!(read(&pack, "assets/minecraft/atlases/items.json")["sources"].as_array().unwrap().len(), 3);
        assert_eq!(ctx.into_warnings().len(), 1);

        let mut ctx = context();
        Celestials.downgrade(&mut pack, &mut ctx).unwrap();
        assert!(!pack.contains("assets/minecraft/atlases/items.json"));
        assert_eq!(read(&pack, "assets/minecraft/atlases/blocks.json")["sources"].as_array().unwrap().len(), 4);
    }

    #[test]
    fn blocks_atlas_without_item_sources_is_untouched_and_empty_one_is_removed() {
        let mut pack = Pack::new();
        let only_items = json!({"sources": [{"type": "minecraft:directory", "source": "item", "prefix": "item/"}]});
        write_json(&mut pack, "assets/minecraft/atlases/blocks.json", &only_items);
        write_json(&mut pack, "assets/mod/atlases/blocks.json", &json!({"sources": [{"type": "minecraft:directory", "source": "block", "prefix": ""}]}));
        let before = pack.get("assets/mod/atlases/blocks.json").unwrap().to_vec();
        let mut ctx = context();
        Celestials.upgrade(&mut pack, &mut ctx).unwrap();
        assert!(!pack.contains("assets/minecraft/atlases/blocks.json"));
        assert_eq!(pack.get("assets/mod/atlases/blocks.json").unwrap(), before.as_slice());
    }

    #[test]
    fn bad_png_atlas_and_effect_sprites_warn() {
        let mut pack = Pack::new();
        pack.insert("assets/minecraft/textures/environment/moon_phases.png", b"nope".to_vec());
        pack.insert("assets/minecraft/atlases/blocks.json", b"{".to_vec());
        pack.insert("assets/minecraft/textures/gui/sprites/container/inventory/effect_background_small.png", vec![]);
        let mut ctx = context();
        Celestials.upgrade(&mut pack, &mut ctx).unwrap();
        assert_eq!(ctx.into_warnings().len(), 3);
        assert!(pack.contains("assets/minecraft/textures/environment/moon_phases.png"));
    }

    #[test]
    fn oversized_moon_sheet_is_kept() {
        let mut pack = Pack::new();
        pack.insert("assets/minecraft/textures/environment/moon_phases.png", crate::steps::support::oversized_png());
        let mut ctx = context();
        Celestials.upgrade(&mut pack, &mut ctx).unwrap();
        assert!(pack.contains("assets/minecraft/textures/environment/moon_phases.png"));
        assert_eq!(ctx.into_warnings().len(), 1);
    }
}

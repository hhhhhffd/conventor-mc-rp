//! 1.19.3: texture atlases became configurable, and the creative inventory tabs got narrower.
//!
//! Before 1.19.3 the block atlas contained every texture a model referenced. Since then it only
//! holds what `atlases/blocks.json` lists, and the built-in list covers `block/`, `item/` and a
//! few entity textures. Packs are merged with the built-in list, so a pack only has to add the
//! textures it uses from other folders.

use std::collections::BTreeSet;

use serde_json::{json, Value};

use crate::error::Result;
use crate::step::{Context, Step};
use crate::util::{write_json, write_png};
use crate::version::Version;
use crate::vfs::Pack;

use super::rename::load_json;
use super::sheet::{copy_area, load_png, scale_factor, Area, Flip};

const BLOCKS_ATLAS: &str = "assets/minecraft/atlases/blocks.json";
const TABS: &str = "assets/minecraft/textures/gui/container/creative_inventory/tabs.png";
const TABS_DESIGN: (u32, u32) = (256, 256);

/// Source tab (1.19.2 has seven) of each of the eight 1.19.3 tabs. The new tab 5 repeats tab 4.
const TAB_SOURCES: [u32; 8] = [0, 1, 2, 3, 4, 4, 5, 6];
/// Width of one tab before and after 1.19.3.
const OLD_TAB: u32 = 28;
const NEW_TAB: u32 = 26;
/// Only the upper part of the sheet holds tabs.
const TAB_ROWS: u32 = 160;

/// The copies that build the 1.19.3 sheet out of the 1.19.2 one, as `(source area, destination)`.
/// A tab loses two pixels of its flat middle: its left 14 columns go to the start of the new
/// tab and its right 14 columns end the new tab (overlapping the left part by two columns).
/// The scroller sprite in the top right corner stays where it is.
fn tab_copies(k: u32) -> Vec<(Area, (u32, u32))> {
    let mut copies = Vec::new();
    for (new, &old) in TAB_SOURCES.iter().enumerate() {
        let (new, old) = (new as u32, old);
        copies.push(((old * OLD_TAB, 0, 14, TAB_ROWS), (new * NEW_TAB, 0)));
        copies.push(((old * OLD_TAB + 14, 0, 14, TAB_ROWS), (new * NEW_TAB + 12, 0)));
    }
    copies.push(((232, 0, 24, 15), (232, 0)));
    copies
        .into_iter()
        .map(|((x, y, w, h), (dx, dy))| ((x * k, y * k, w * k, h * k), (dx * k, dy * k)))
        .collect()
}

/// Model texture references that need an atlas source: everything
/// outside `block/` and `item/`, in `namespace:path` form without the default namespace.
fn atlas_textures(pack: &Pack, ctx: &mut Context) -> BTreeSet<String> {
    let mut textures = BTreeSet::new();
    for path in pack.paths_with_prefix("assets/") {
        if path.split('/').nth(2) != Some("models") || !path.ends_with(".json") {
            continue;
        }
        let Some(model) = load_json(pack, &path, ctx) else { continue };
        let Some(Value::Object(vars)) = model.get("textures") else { continue };
        for reference in vars.values().filter_map(Value::as_str).filter(|r| !r.starts_with('#')) {
            let reference = reference.strip_prefix("minecraft:").unwrap_or(reference);
            let relative = reference.split_once(':').map_or(reference, |(_, path)| path);
            if !relative.starts_with("block/") && !relative.starts_with("item/") {
                textures.insert(reference.to_string());
            }
        }
    }
    textures
}

pub struct V1_19_3;

impl Step for V1_19_3 {
    fn name(&self) -> &'static str {
        "1.19.3 texture atlases and creative tabs"
    }

    fn version(&self) -> Version {
        Version::new(1, 19, 3)
    }

    fn upgrade(&self, pack: &mut Pack, ctx: &mut Context) -> Result<()> {
        let textures = atlas_textures(pack, ctx);
        if !textures.is_empty() {
            add_atlas_sources(pack, ctx, textures);
        }
        rearrange_tabs(pack, ctx, false)
    }

    fn downgrade(&self, pack: &mut Pack, ctx: &mut Context) -> Result<()> {
        let mut unsupported = false;
        for path in pack.paths_with_prefix("assets/") {
            if path.split('/').nth(2) != Some("atlases") {
                continue;
            }
            // Directory and single sources are what older versions do anyway; the other
            // source types (filter, unstitch, palette permutations) have no equivalent.
            if let Some(Value::Object(atlas)) = load_json(pack, &path, ctx) {
                unsupported |= atlas.get("sources").and_then(Value::as_array).is_some_and(|list| {
                    list.iter().any(|source| {
                        !matches!(source["type"].as_str(), Some("single" | "directory"))
                    })
                });
            }
            pack.remove(&path);
        }
        if unsupported {
            ctx.warn("atlas configs were removed, but they used source types that older versions cannot express (filter, unstitch or palette permutations)");
        }

        rearrange_tabs(pack, ctx, true)
    }
}

/// Adds a `single` source per texture to the pack's block atlas, creating the atlas if needed.
fn add_atlas_sources(pack: &mut Pack, ctx: &mut Context, textures: BTreeSet<String>) {
    let mut atlas = if pack.contains(BLOCKS_ATLAS) {
        match load_json(pack, BLOCKS_ATLAS, ctx) {
            Some(atlas) => atlas,
            None => return,
        }
    } else {
        json!({"sources": []})
    };
    let Some(Value::Array(sources)) = atlas.get_mut("sources") else {
        ctx.warn(format!("{BLOCKS_ATLAS}: has no \"sources\" list; textures outside block/ and item/ may be missing from the block atlas"));
        return;
    };
    for texture in textures {
        let source = json!({"type": "single", "resource": texture});
        if !sources.contains(&source) {
            sources.push(source);
        }
    }
    write_json(pack, BLOCKS_ATLAS, &atlas);
}

/// Builds the 1.19.3 tab sheet from the 1.19.2 one, or the other way round when `back` is set.
/// Going back swaps source and destination of every copy; the repeated tab 4 is then read
/// from the later of its two copies.
fn rearrange_tabs(pack: &mut Pack, ctx: &mut Context, back: bool) -> Result<()> {
    let Some(tabs) = load_png(pack, TABS, ctx) else { return Ok(()) };
    let Some(k) = scale_factor(&tabs, TABS_DESIGN) else {
        ctx.warn(format!(
            "{TABS}: unexpected size {}x{}; left unconverted",
            tabs.width(),
            tabs.height()
        ));
        return Ok(());
    };
    let mut out = image::RgbaImage::new(tabs.width(), tabs.height());
    for ((x, y, w, h), (dx, dy)) in tab_copies(k) {
        if back {
            copy_area(&mut out, &tabs, (dx, dy, w, h), (x, y), Flip::None);
        } else {
            copy_area(&mut out, &tabs, (x, y, w, h), (dx, dy), Flip::None);
        }
    }
    write_png(pack, TABS, &out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::read_png;
    use image::{Rgba, RgbaImage};

    fn ctx() -> Context {
        Context::new(Version::new(1, 19, 2), Version::new(1, 19, 3))
    }

    fn json_of(pack: &Pack, path: &str) -> Value {
        serde_json::from_slice(pack.get(path).expect("file exists")).expect("valid json")
    }

    #[test]
    fn atlas_lists_textures_outside_the_builtin_folders() {
        let mut pack = Pack::new();
        pack.insert(
            "assets/minecraft/models/block/a.json",
            br##"{"textures":{"all":"block/stone","top":"minecraft:custom/top","side":"#all","p":"mymod:deep/dir/p"}}"##.to_vec(),
        );
        pack.insert(
            "assets/mymod/models/item/b.json",
            br#"{"textures":{"layer0":"item/x","l1":"custom/top","l2":"mymod:block/y"}}"#.to_vec(),
        );
        V1_19_3.upgrade(&mut pack, &mut ctx()).unwrap();
        let sources = json_of(&pack, BLOCKS_ATLAS)["sources"].clone();
        assert_eq!(
            sources,
            json!([
                {"type": "single", "resource": "custom/top"},
                {"type": "single", "resource": "mymod:deep/dir/p"},
            ])
        );
    }

    #[test]
    fn existing_atlas_is_extended_without_duplicates() {
        let mut pack = Pack::new();
        pack.insert(BLOCKS_ATLAS, br#"{"sources":[{"type":"single","resource":"custom/top"},{"type":"filter","pattern":{}}]}"#.to_vec());
        pack.insert(
            "assets/minecraft/models/block/a.json",
            br#"{"textures":{"a":"custom/top","b":"custom/b"}}"#.to_vec(),
        );
        V1_19_3.upgrade(&mut pack, &mut ctx()).unwrap();
        let sources = json_of(&pack, BLOCKS_ATLAS)["sources"].clone();
        assert_eq!(sources.as_array().unwrap().len(), 3);
        assert_eq!(sources[2], json!({"type": "single", "resource": "custom/b"}));
    }

    #[test]
    fn no_atlas_is_written_when_models_stay_inside_block_and_item() {
        let mut pack = Pack::new();
        pack.insert(
            "assets/minecraft/models/block/a.json",
            br#"{"textures":{"all":"block/stone"}}"#.to_vec(),
        );
        V1_19_3.upgrade(&mut pack, &mut ctx()).unwrap();
        assert!(!pack.contains(BLOCKS_ATLAS));
    }

    #[test]
    fn downgrade_removes_atlases_and_warns_only_about_unsupported_sources() {
        let mut pack = Pack::new();
        pack.insert(BLOCKS_ATLAS, br#"{"sources":[{"type":"single","resource":"a"},{"type":"directory","source":"b","prefix":"b/"}]}"#.to_vec());
        let mut ctx = ctx();
        V1_19_3.downgrade(&mut pack, &mut ctx).unwrap();
        assert!(!pack.contains(BLOCKS_ATLAS));
        assert!(ctx.into_warnings().is_empty());

        pack.insert(
            "assets/minecraft/atlases/signs.json",
            br#"{"sources":[{"type":"unstitch","resource":"x","regions":[]}]}"#.to_vec(),
        );
        let mut ctx = Context::new(Version::new(1, 19, 3), Version::new(1, 19, 2));
        V1_19_3.downgrade(&mut pack, &mut ctx).unwrap();
        assert!(!pack.contains("assets/minecraft/atlases/signs.json"));
        assert_eq!(ctx.into_warnings().len(), 1);
    }

    /// Each 28-pixel tab of the old sheet gets its own colour per column.
    fn old_tabs(k: u32) -> RgbaImage {
        RgbaImage::from_fn(256 * k, 256 * k, |x, y| {
            if y < 160 * k && x < 7 * 28 * k {
                Rgba([(x / k) as u8, (y / k) as u8, 1, 255])
            } else {
                Rgba([0, 0, 0, 0])
            }
        })
    }

    #[test]
    fn tabs_become_narrower_and_back() {
        for k in [1, 2] {
            let mut pack = Pack::new();
            let old = old_tabs(k);
            write_png(&mut pack, TABS, &old).unwrap();
            V1_19_3.upgrade(&mut pack, &mut ctx()).unwrap();
            let new = read_png(&pack, TABS).unwrap().unwrap();
            // New tab 1 starts with the first column of old tab 1, ends with its last column.
            assert_eq!(new.get_pixel(26 * k, 3 * k), old.get_pixel(28 * k, 3 * k));
            assert_eq!(
                new.get_pixel(26 * k + 25 * k, 3 * k),
                old.get_pixel(28 * k + 27 * k, 3 * k)
            );
            // New tabs 4 and 5 are both old tab 4.
            assert_eq!(new.get_pixel(4 * 26 * k + 3 * k, 0), new.get_pixel(5 * 26 * k + 3 * k, 0));
            V1_19_3.downgrade(&mut pack, &mut ctx()).unwrap();
            let back = read_png(&pack, TABS).unwrap().unwrap();
            for (x, y, _) in old.enumerate_pixels() {
                // Columns 12 and 13 of a tab are overwritten by the right part in the narrower
                // sheet and come back as copies of columns 14 and 15 (they are flat in vanilla).
                let column = x / k % 28;
                let source_x =
                    if (12..14).contains(&column) && x < 7 * 28 * k { x + 2 * k } else { x };
                assert_eq!(back.get_pixel(x, y), old.get_pixel(source_x, y), "({x},{y})");
            }
        }
    }
}

//! Helpers shared by the modern-era steps: table-driven file moves and resource id renames.

use std::collections::HashMap;

use image::{imageops, RgbaImage};
use serde_json::Value;

use crate::step::Context;
use crate::util::{namespaces, read_json, write_json};
use crate::vfs::Pack;

const MINECRAFT: &str = "assets/minecraft";

/// Parses a tab separated `from<TAB>to` table, skipping blank and `#` lines.
pub fn parse_pairs(table: &'static str) -> Vec<(&'static str, &'static str)> {
    table
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| l.split_once('\t'))
        .collect()
}

/// Swaps source and destination of every pair, for the downgrade direction.
pub fn reversed<'a>(pairs: &[(&'a str, &'a str)]) -> Vec<(&'a str, &'a str)> {
    pairs.iter().map(|&(from, to)| (to, from)).collect()
}

/// Moves a file together with its `.mcmeta` sibling. Returns whether the file existed.
pub fn move_with_mcmeta(pack: &mut Pack, from: &str, to: &str) -> bool {
    pack.rename(&format!("{from}.mcmeta"), &format!("{to}.mcmeta"));
    pack.rename(from, to)
}

/// Moves `assets/minecraft/textures/<from>` to `<to>` for every pair (paths include `.png`).
pub fn move_textures(pack: &mut Pack, pairs: &[(&str, &str)]) {
    for (from, to) in pairs {
        move_with_mcmeta(
            pack,
            &format!("{MINECRAFT}/textures/{from}"),
            &format!("{MINECRAFT}/textures/{to}"),
        );
    }
}

/// Renames vanilla resource ids such as `item/scute` -> `item/turtle_scute`: the texture
/// (with its `.mcmeta`), the model, the blockstate of `block/` ids, and every reference to
/// the id inside models, blockstates and item definitions of all namespaces.
pub fn rename_ids(pack: &mut Pack, ctx: &mut Context, pairs: &[(&str, &str)]) {
    for (from, to) in pairs {
        move_with_mcmeta(
            pack,
            &format!("{MINECRAFT}/textures/{from}.png"),
            &format!("{MINECRAFT}/textures/{to}.png"),
        );
        pack.rename(
            &format!("{MINECRAFT}/models/{from}.json"),
            &format!("{MINECRAFT}/models/{to}.json"),
        );
        if let (Some(from), Some(to)) = (from.strip_prefix("block/"), to.strip_prefix("block/")) {
            pack.rename(
                &format!("{MINECRAFT}/blockstates/{from}.json"),
                &format!("{MINECRAFT}/blockstates/{to}.json"),
            );
        }
    }
    let map: HashMap<&str, &str> = pairs.iter().copied().collect();
    for ns in namespaces(pack) {
        for dir in ["models", "blockstates", "items"] {
            let prefix = format!("assets/{ns}/{dir}/");
            for path in pack.paths_with_prefix(&prefix) {
                if !path.ends_with(".json") {
                    continue;
                }
                match read_json(pack, &path) {
                    Some(Ok(mut json)) => {
                        if rewrite_refs(&mut json, &map) {
                            write_json(pack, &path, &json);
                        }
                    }
                    Some(Err(e)) => ctx.warn(format!("{e}; references to renamed resources were not updated")),
                    None => {}
                }
            }
        }
    }
}

/// Replaces string values that are (optionally `minecraft:`-prefixed) renamed ids.
fn rewrite_refs(value: &mut Value, map: &HashMap<&str, &str>) -> bool {
    match value {
        Value::String(s) => {
            let (prefix, bare) = match s.strip_prefix("minecraft:") {
                Some(bare) => ("minecraft:", bare),
                None => ("", s.as_str()),
            };
            match map.get(bare) {
                Some(to) => {
                    *s = format!("{prefix}{to}");
                    true
                }
                None => false,
            }
        }
        Value::Array(items) => items.iter_mut().fold(false, |c, v| rewrite_refs(v, map) | c),
        Value::Object(fields) => fields.values_mut().fold(false, |c, v| rewrite_refs(v, map) | c),
        _ => false,
    }
}

/// Crops `(x, y, w, h)`; returns `None` when the rectangle does not fit inside the image.
pub fn crop(img: &RgbaImage, x: u32, y: u32, w: u32, h: u32) -> Option<RgbaImage> {
    if w == 0 || h == 0 || x + w > img.width() || y + h > img.height() {
        return None;
    }
    Some(imageops::crop_imm(img, x, y, w, h).to_image())
}

/// Nearest-neighbour resize, the only scaling that keeps pixel art crisp.
pub fn resize(img: &RgbaImage, w: u32, h: u32) -> RgbaImage {
    if img.dimensions() == (w, h) {
        return img.clone();
    }
    imageops::resize(img, w, h, imageops::FilterType::Nearest)
}

/// Writes `sprite` over `canvas` at `(x, y)`, replacing the pixels underneath (also transparent ones).
pub fn paste(canvas: &mut RgbaImage, sprite: &RgbaImage, x: u32, y: u32) {
    imageops::replace(canvas, sprite, i64::from(x), i64::from(y));
}

#[cfg(test)]
pub mod testing {
    use image::{Rgba, RgbaImage};

    use crate::util::{read_png, write_png};
    use crate::vfs::Pack;

    pub fn solid(w: u32, h: u32, color: [u8; 4]) -> RgbaImage {
        RgbaImage::from_pixel(w, h, Rgba(color))
    }

    pub fn put_png(pack: &mut Pack, path: &str, img: &RgbaImage) {
        write_png(pack, path, img).unwrap();
    }

    pub fn png(pack: &Pack, path: &str) -> RgbaImage {
        read_png(pack, path).unwrap_or_else(|| panic!("{path} missing")).unwrap()
    }
}

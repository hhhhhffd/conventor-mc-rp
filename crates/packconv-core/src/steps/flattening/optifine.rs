//! OptiFine/MCPatcher `.properties` files: numeric block and item ids became namespaced ids in
//! 1.13, and texture paths follow the `blocks`/`items` to `block`/`item` directory renames.

use super::maps::{block_ids, names, normalize, Direction, RefKind};
use crate::step::Context;
use crate::vfs::Pack;

/// Keys whose value is a list of block or item ids.
const ID_KEYS: [&str; 4] = ["matchBlocks", "matchItems", "items", "blocks"];

/// Things that could not be carried over; reported once per conversion.
#[derive(Default)]
struct Lost {
    unmapped_ids: bool,
    metadata: bool,
}

pub fn convert(pack: &mut Pack, ctx: &mut Context, direction: Direction) {
    let directory = match direction {
        Direction::Up => "optifine",
        Direction::Down => "mcpatcher",
    };
    let mut lost = Lost::default();
    for path in pack.paths_with_prefix("assets/") {
        let in_directory = path.split('/').nth(2) == Some(directory);
        if !in_directory || !path.ends_with(".properties") {
            continue;
        }
        let Some(bytes) = pack.get(&path) else {
            continue;
        };
        let Ok(text) = std::str::from_utf8(bytes) else {
            ctx.warn(format!(
                "{path}: not UTF-8, OptiFine properties left unconverted"
            ));
            continue;
        };
        let converted = convert_properties(text, direction, &mut lost);
        if converted != text {
            pack.insert(path, converted.into_bytes());
        }
    }
    if lost.unmapped_ids {
        ctx.warn("OptiFine properties: block/item ids without a 1.12/1.13 equivalent were left as they are");
    }
    if lost.metadata {
        ctx.warn(
            "OptiFine properties: `metadata` lines were dropped, 1.13 no longer has block metadata",
        );
    }
}

fn convert_properties(text: &str, direction: Direction, lost: &mut Lost) -> String {
    let mut out = String::with_capacity(text.len());
    for line in text.split_inclusive('\n') {
        let content = line.trim_end_matches(['\r', '\n']);
        let ending = &line[content.len()..];
        if let Some(converted) = convert_line(content, direction, lost) {
            out.push_str(&converted);
            out.push_str(ending);
        }
    }
    out
}

/// Returns the rewritten line, or `None` when the line must be dropped.
fn convert_line(line: &str, direction: Direction, lost: &mut Lost) -> Option<String> {
    let trimmed = line.trim_start();
    if trimmed.starts_with('#') || trimmed.starts_with('!') {
        return Some(line.to_string());
    }
    let Some((key, value)) = line.split_once('=') else {
        return Some(line.to_string());
    };
    let key = key.trim();
    let value = value.trim();

    let converted = if ID_KEYS.contains(&key) {
        convert_ids(value, direction, lost)
    } else if key == "metadata" {
        if direction == Direction::Up {
            lost.metadata = true;
            return None;
        }
        return Some(line.to_string());
    } else if key == "matchTiles" {
        let names = names(direction);
        // Tile names are block texture names.
        value
            .split_whitespace()
            .map(|tile| match direction {
                Direction::Up => names.remap_ref(&format!("blocks/{tile}"), RefKind::Texture),
                Direction::Down => names.remap_ref(&format!("block/{tile}"), RefKind::Texture),
            })
            .map(|path| {
                path.rsplit_once('/')
                    .map_or(path.clone(), |(_, tile)| tile.to_string())
            })
            .collect::<Vec<_>>()
            .join(" ")
    } else if key.starts_with("texture") || key.starts_with("tiles") || key.starts_with("model") {
        convert_paths(value, direction)
    } else {
        return Some(line.to_string());
    };
    Some(if converted == value {
        line.to_string()
    } else {
        format!("{key}={converted}")
    })
}

/// Rewrites the `textures/blocks/...` style paths inside a value. Paths are lowercased for 1.13.
fn convert_paths(value: &str, direction: Direction) -> String {
    let value = match direction {
        Direction::Up => normalize(value),
        Direction::Down => value.to_string(),
    };
    value
        .split_whitespace()
        .map(|token| {
            let Some(at) = token.find("textures/") else {
                return token.to_string();
            };
            let (prefix, texture) = token.split_at(at + "textures/".len());
            let (path, extension) = match texture.rsplit_once('.') {
                Some((path, extension)) => (path, format!(".{extension}")),
                None => (texture, String::new()),
            };
            format!(
                "{prefix}{}{extension}",
                names(direction).remap_ref(path, RefKind::Texture)
            )
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn convert_ids(value: &str, direction: Direction, lost: &mut Lost) -> String {
    let ids = block_ids(direction);
    value
        .split_whitespace()
        .map(|token| {
            let token = token.trim_matches('"');
            let key = match direction {
                Direction::Up if token.starts_with(|c: char| c.is_ascii_digit()) => {
                    token.to_string()
                }
                Direction::Down if token.starts_with("minecraft:") => token.to_string(),
                Direction::Down if token.starts_with(|c: char| c.is_ascii_lowercase()) => {
                    format!("minecraft:{token}")
                }
                _ => return token.to_string(),
            };
            match ids.get(&key) {
                Some(mapped) => mapped.clone(),
                None => {
                    lost.unmapped_ids = true;
                    token.to_string()
                }
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

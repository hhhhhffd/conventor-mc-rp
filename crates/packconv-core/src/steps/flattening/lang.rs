//! `lang/*.lang` (key=value lines) and `lang/*.json` (flat object) conversion.

use serde_json::{Map, Value};

use super::maps::{lang_keys, Direction};
use crate::step::Context;
use crate::util::read_json;
use crate::vfs::Pack;

/// Language files directly inside `assets/<ns>/lang/` with the given extension.
fn language_files(pack: &Pack, extension: &str) -> Vec<String> {
    pack.paths_with_prefix("assets/")
        .into_iter()
        .filter(|path| {
            let parts: Vec<&str> = path.split('/').collect();
            parts.len() == 4 && parts[2] == "lang" && parts[3].ends_with(extension)
        })
        .collect()
}

pub fn convert(pack: &mut Pack, ctx: &mut Context, direction: Direction) {
    let keys = lang_keys(direction);
    let remap = |key: &str| keys.get(key).cloned().unwrap_or_else(|| key.to_string());

    // A converted file never replaces one that already exists in the target format.
    let mut dropped = 0;
    match direction {
        Direction::Up => {
            for path in language_files(pack, ".lang") {
                let Some(bytes) = pack.remove(&path) else {
                    continue;
                };
                let text = String::from_utf8_lossy(&bytes);
                let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
                let mut entries = Map::new();
                for line in text.lines() {
                    // Minecraft 1.12 reads plain UTF-8 `key=value` lines: no escapes, `#` comments.
                    if line.starts_with('#') {
                        continue;
                    }
                    if let Some((key, value)) = line.split_once('=') {
                        entries.insert(remap(key), Value::String(value.to_string()));
                    }
                }
                let json_path = format!("{}.json", path.trim_end_matches(".lang"));
                if pack.contains(&json_path) {
                    dropped += 1;
                    continue;
                }
                let data = serde_json::to_vec_pretty(&Value::Object(entries))
                    .expect("JSON value serializes");
                pack.insert(json_path, data);
            }
        }
        Direction::Down => {
            for path in language_files(pack, ".json") {
                let entries = match read_json(pack, &path) {
                    Some(Ok(Value::Object(entries))) => entries,
                    Some(Ok(_)) => {
                        ctx.warn(format!(
                            "{path}: language file is not a JSON object, left unconverted"
                        ));
                        continue;
                    }
                    Some(Err(e)) => {
                        ctx.warn(format!("skipping malformed language file: {e}"));
                        continue;
                    }
                    None => continue,
                };
                let mut text = String::new();
                for (key, value) in entries {
                    if let Value::String(value) = value {
                        // A line break would end the entry early in the line-based format.
                        text.push_str(&format!("{}={}\n", remap(&key), value.replace('\n', " ")));
                    }
                }
                pack.remove(&path);
                let lang_path = format!("{}.lang", path.trim_end_matches(".json"));
                if pack.contains(&lang_path) {
                    dropped += 1;
                    continue;
                }
                pack.insert(lang_path, text.into_bytes());
            }
        }
    }
    if dropped > 0 {
        ctx.warn(format!(
            "{dropped} language file(s) were dropped because a file in the target format already exists"
        ));
    }
}

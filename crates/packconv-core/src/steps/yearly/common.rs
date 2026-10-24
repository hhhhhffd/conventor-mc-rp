//! Helpers shared by the yearly steps: file moves, reference rewriting and JSON editing.
//!
//! All file paths handed to these helpers are relative to `assets/minecraft/`.

use std::collections::HashMap;

use serde_json::Value;

use crate::step::Context;
use crate::util::{read_json, write_json};
use crate::vfs::Pack;

pub const MINECRAFT: &str = "assets/minecraft/";

/// `(old, new)` pairs. Walking them backwards (see [`directed`]) gives the downgrade.
pub type Pairs = [(&'static str, &'static str)];

/// Returns the pairs as `(from, to)` for the requested direction.
pub fn directed(pairs: &Pairs, up: bool) -> Vec<(&'static str, &'static str)> {
    pairs.iter().map(|&(old, new)| if up { (old, new) } else { (new, old) }).collect()
}

/// Moves files (plus the `.mcmeta` sibling of every PNG). All sources are taken out before
/// anything is written, so renames that swap or chain names stay correct.
pub fn move_files<A: AsRef<str>, B: AsRef<str>>(pack: &mut Pack, pairs: &[(A, B)]) {
    let mut moved = Vec::new();
    for (from, to) in pairs {
        let (from, to) = (format!("{MINECRAFT}{}", from.as_ref()), format!("{MINECRAFT}{}", to.as_ref()));
        if let Some(data) = pack.remove(&from) {
            moved.push((to.clone(), data));
        }
        if from.ends_with(".png") {
            if let Some(data) = pack.remove(&format!("{from}.mcmeta")) {
                moved.push((format!("{to}.mcmeta"), data));
            }
        }
    }
    for (path, data) in moved {
        pack.insert(path, data);
    }
}

/// Resource id rewriting table. Matches ids with or without the `minecraft:` namespace, with
/// or without a leading `textures/` and trailing `.png`, so one table serves model
/// `textures`, item definitions, atlases and OptiFine CEM models alike.
pub struct RefMap(HashMap<&'static str, &'static str>);

impl RefMap {
    pub fn new(pairs: &[(&'static str, &'static str)]) -> Self {
        Self(pairs.iter().copied().collect())
    }

    pub fn apply(&self, value: &str) -> Option<String> {
        let (namespace, rest) = match value.strip_prefix("minecraft:") {
            Some(rest) => ("minecraft:", rest),
            None => ("", value),
        };
        let (folder, rest) = match rest.strip_prefix("textures/") {
            Some(rest) => ("textures/", rest),
            None => ("", rest),
        };
        let (id, extension) = match rest.strip_suffix(".png") {
            Some(id) => (id, ".png"),
            None => (rest, ""),
        };
        self.0.get(id).map(|new| format!("{namespace}{folder}{new}{extension}"))
    }

    /// Rewrites every string value in `value`. Returns whether anything changed.
    pub fn rewrite_all(&self, value: &mut Value) -> bool {
        match value {
            Value::String(s) => match self.apply(s) {
                Some(new) => {
                    *s = new;
                    true
                }
                None => false,
            },
            Value::Array(items) => items.iter_mut().fold(false, |changed, v| self.rewrite_all(v) | changed),
            Value::Object(map) => map.values_mut().fold(false, |changed, v| self.rewrite_all(v) | changed),
            _ => false,
        }
    }

    /// Rewrites only the values under a block model's top-level `textures` key. Needed when
    /// a model id and a texture id share a name (`block/quartz_pillar`) but only the texture moved.
    pub fn rewrite_model_textures(&self, model: &mut Value) -> bool {
        model.get_mut("textures").is_some_and(|textures| self.rewrite_all(textures))
    }
}

/// Paths of files with one of `exts` below `assets/<namespace>/<dir>/` in every namespace.
pub fn asset_files(pack: &Pack, dir: &str, exts: &[&str]) -> Vec<String> {
    let prefix = format!("{dir}/");
    pack.paths()
        .filter(|path| {
            path.strip_prefix("assets/")
                .and_then(|rest| rest.split_once('/'))
                .is_some_and(|(_, rest)| rest.starts_with(&prefix))
                && exts.iter().any(|ext| path.ends_with(ext))
        })
        .map(str::to_string)
        .collect()
}

/// Applies `edit` to each JSON file and saves those it reports as changed. Malformed files
/// are reported and left as they are.
pub fn edit_json_files(pack: &mut Pack, ctx: &mut Context, files: Vec<String>, mut edit: impl FnMut(&mut Value) -> bool) {
    for path in files {
        match read_json(pack, &path) {
            Some(Ok(mut json)) => {
                if edit(&mut json) {
                    write_json(pack, &path, &json);
                }
            }
            Some(Err(e)) => ctx.warn(format!("{e}; file left unchanged")),
            None => {}
        }
    }
}

/// Whether any item definition contains a model object whose `type` is one of `types`.
pub fn uses_item_model_type(pack: &Pack, types: &[&str]) -> bool {
    fn contains(value: &Value, types: &[&str]) -> bool {
        match value {
            Value::Object(map) => {
                map.get("type").and_then(Value::as_str).is_some_and(|t| types.contains(&t)) || map.values().any(|v| contains(v, types))
            }
            Value::Array(items) => items.iter().any(|v| contains(v, types)),
            _ => false,
        }
    }
    asset_files(pack, "items", &[".json"])
        .iter()
        .filter_map(|path| read_json(pack, path)?.ok())
        .any(|json| contains(&json, types))
}

/// Custom shaders are GLSL tied to the engine's internal uniforms and cannot be converted.
pub fn warn_if_shaders(pack: &Pack, ctx: &mut Context, reason: &str) {
    let has_shaders = pack
        .paths()
        .any(|p| p.strip_prefix("assets/").and_then(|r| r.split('/').nth(1)).is_some_and(|dir| dir == "shaders" || dir == "post_effect"));
    if has_shaders {
        ctx.warn(format!("custom shaders cannot be converted automatically ({reason}); review them by hand"));
    }
}

#[cfg(test)]
pub mod testing {
    use image::RgbaImage;

    use crate::step::Context;
    use crate::util::{read_png, write_png};
    use crate::version::Version;
    use crate::vfs::Pack;

    pub fn context() -> Context {
        Context::new(Version::new(1, 21, 8), Version::new(26, 2, 0))
    }

    pub fn insert_png(pack: &mut Pack, path: &str, img: &RgbaImage) {
        write_png(pack, path, img).unwrap();
    }

    pub fn png(pack: &Pack, path: &str) -> RgbaImage {
        read_png(pack, path).unwrap_or_else(|| panic!("missing {path}")).unwrap()
    }

    pub fn solid(w: u32, h: u32, color: [u8; 4]) -> RgbaImage {
        RgbaImage::from_pixel(w, h, image::Rgba(color))
    }

    /// Equal bytes, or equal JSON documents (steps re-serialize the JSON they edit).
    pub fn same_content(a: &[u8], b: &[u8]) -> bool {
        match (serde_json::from_slice::<serde_json::Value>(a), serde_json::from_slice::<serde_json::Value>(b)) {
            (Ok(a), Ok(b)) => a == b,
            _ => a == b,
        }
    }
}

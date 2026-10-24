//! Helpers shared by conversion steps for reading and writing pack files.

use image::RgbaImage;
use serde_json::Value;

use crate::error::{Error, Result};
use crate::vfs::Pack;

/// Parses a JSON file. Tolerates a UTF-8 BOM, which many hand-edited packs contain.
pub fn read_json(pack: &Pack, path: &str) -> Option<Result<Value>> {
    let bytes = pack.get(path)?;
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    Some(serde_json::from_slice(bytes).map_err(|e| Error::Json(format!("{path}: {e}"))))
}

pub fn write_json(pack: &mut Pack, path: &str, value: &Value) {
    let data = serde_json::to_vec_pretty(value).expect("JSON value serializes");
    pack.insert(path, data);
}

pub fn read_png(pack: &Pack, path: &str) -> Option<Result<RgbaImage>> {
    let bytes = pack.get(path)?;
    Some(
        image::load_from_memory_with_format(bytes, image::ImageFormat::Png)
            .map(|img| img.into_rgba8())
            .map_err(|e| Error::Image(format!("{path}: {e}"))),
    )
}

pub fn write_png(pack: &mut Pack, path: &str, img: &RgbaImage) -> Result<()> {
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png)
        .map_err(|e| Error::Image(format!("{path}: {e}")))?;
    pack.insert(path, out.into_inner());
    Ok(())
}

/// Namespaces present under `assets/`.
pub fn namespaces(pack: &Pack) -> Vec<String> {
    let mut out: Vec<String> = pack
        .paths_with_prefix("assets/")
        .iter()
        .filter_map(|p| p["assets/".len()..].split_once('/').map(|(ns, _)| ns.to_string()))
        .collect();
    out.dedup();
    out
}

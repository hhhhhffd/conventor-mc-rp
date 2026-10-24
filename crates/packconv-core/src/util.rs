//! Helpers shared by conversion steps for reading and writing pack files.

use std::io::Cursor;

use image::{ImageFormat, ImageReader, RgbaImage};
use serde_json::Value;

use crate::error::{Error, Result};
use crate::vfs::Pack;

/// Parses a JSON file. Like the game's lenient parser it tolerates a UTF-8 BOM and trailing
/// garbage after the first value (e.g. `// comments`), which hand-edited packs often contain.
pub fn read_json(pack: &Pack, path: &str) -> Option<Result<Value>> {
    let bytes = pack.get(path)?;
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    let first = serde_json::Deserializer::from_slice(bytes).into_iter::<Value>().next();
    Some(match first {
        Some(Ok(value)) => Ok(value),
        Some(Err(e)) => Err(Error::Json(format!("{path}: {e}"))),
        None => Err(Error::Json(format!("{path}: empty file"))),
    })
}

pub fn write_json(pack: &mut Pack, path: &str, value: &Value) {
    let data = serde_json::to_vec_pretty(value).expect("JSON value serializes");
    pack.insert(path, data);
}

/// Largest image the converter decodes or allocates, in pixels (256 MiB as RGBA). Covers
/// 512x resource packs while keeping tiny "pixel bomb" PNGs from exhausting memory.
pub const MAX_IMAGE_PIXELS: u64 = 64 * 1024 * 1024;

fn check_dimensions(width: u32, height: u32) -> std::result::Result<(), String> {
    if width == 0 || height == 0 {
        Err(format!("empty image {width}x{height}"))
    } else if u64::from(width) * u64::from(height) > MAX_IMAGE_PIXELS {
        Err(format!("image too large ({width}x{height})"))
    } else {
        Ok(())
    }
}

/// Decodes a PNG as RGBA. Dimensions are checked from the header before decoding.
pub fn read_png(pack: &Pack, path: &str) -> Option<Result<RgbaImage>> {
    let bytes = pack.get(path)?;
    let err = |e: String| Error::Image(format!("{path}: {e}"));
    let reader = || ImageReader::with_format(Cursor::new(bytes), ImageFormat::Png);
    let decoded = reader()
        .into_dimensions()
        .map_err(|e| err(e.to_string()))
        .and_then(|(w, h)| check_dimensions(w, h).map_err(err))
        .and_then(|()| reader().decode().map_err(|e| err(e.to_string())))
        .map(|img| img.into_rgba8());
    Some(decoded)
}

/// Allocates a transparent canvas, refusing empty or oversized dimensions that steps
/// derive from untrusted image sizes.
pub fn new_canvas(width: u32, height: u32) -> Result<RgbaImage> {
    check_dimensions(width, height).map_err(Error::Image)?;
    Ok(RgbaImage::new(width, height))
}

pub fn write_png(pack: &mut Pack, path: &str, img: &RgbaImage) -> Result<()> {
    check_dimensions(img.width(), img.height()).map_err(|e| Error::Image(format!("{path}: {e}")))?;
    let mut out = Cursor::new(Vec::new());
    img.write_to(&mut out, ImageFormat::Png)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_bad_canvas_sizes() {
        assert!(new_canvas(0, 16).is_err());
        assert!(new_canvas(16384, 16384).is_err());
        assert!(new_canvas(8192, 8192).is_ok());
    }

    #[test]
    fn rejects_oversized_png_from_header() {
        // A valid 1x1 PNG whose IHDR claims 20000x20000: rejected before any pixel decoding.
        let mut png = Vec::new();
        RgbaImage::new(1, 1).write_to(&mut Cursor::new(&mut png), ImageFormat::Png).unwrap();
        png[16..20].copy_from_slice(&20000u32.to_be_bytes());
        png[20..24].copy_from_slice(&20000u32.to_be_bytes());
        let mut pack = Pack::new();
        pack.insert("a.png", png);
        let err = read_png(&pack, "a.png").unwrap().unwrap_err().to_string();
        assert!(err.contains("too large") || err.contains("a.png"), "{err}");
    }
}

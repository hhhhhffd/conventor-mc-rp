//! Helpers shared by all eras for handling images and files that come from untrusted packs.
//! Failures are reported as warnings and the affected file is left as it is, so one hostile
//! or unusual file never aborts the whole conversion.

use image::RgbaImage;

use crate::step::Context;
use crate::util::{new_canvas, read_png, write_png, MAX_IMAGE_PIXELS};
use crate::vfs::Pack;

/// Reads a PNG. A missing file gives `None` silently; an undecodable or oversized one gives
/// `None` with a warning, so the caller leaves the file untouched.
pub(super) fn load_png(pack: &Pack, ctx: &mut Context, path: &str) -> Option<RgbaImage> {
    match read_png(pack, path)? {
        Ok(image) => Some(image),
        Err(e) => {
            ctx.warn(format!("skipped: {e}"));
            None
        }
    }
}

/// Allocates a transparent canvas for a size derived from input. `None` dimensions stand for
/// an overflowed computation. Warns and gives `None` when the size is empty or oversized.
pub(super) fn canvas(
    ctx: &mut Context,
    path: &str,
    width: Option<u32>,
    height: Option<u32>,
) -> Option<RgbaImage> {
    let (Some(width), Some(height)) = (width, height) else {
        ctx.warn(format!("{path}: derived image size overflows, skipped"));
        return None;
    };
    match new_canvas(width, height) {
        Ok(image) => Some(image),
        Err(e) => {
            ctx.warn(format!("{path}: {e}, skipped"));
            None
        }
    }
}

/// Whether an image of this size is within the converter's pixel limit.
pub(super) fn fits(width: u32, height: u32) -> bool {
    width > 0 && height > 0 && u64::from(width) * u64::from(height) <= MAX_IMAGE_PIXELS
}

/// The largest integer scale at which a `design_w x design_h` sheet still fits the pixel
/// limit; 0 when even the design size does not. Scales taken from sprites are clamped to it.
pub(super) fn max_scale(design_w: u32, design_h: u32) -> u32 {
    let area = u64::from(design_w) * u64::from(design_h);
    if area == 0 {
        return 0;
    }
    u32::try_from((MAX_IMAGE_PIXELS / area).isqrt()).unwrap_or(u32::MAX)
}

/// Limits the pixels of the images a step holds in memory at once, so many individually valid
/// images cannot add up to an allocation bomb.
pub(super) struct PixelBudget(u64);

impl PixelBudget {
    pub(super) fn new() -> Self {
        Self(MAX_IMAGE_PIXELS)
    }

    /// Takes the image's pixels from the budget; `false` when they do not fit any more.
    pub(super) fn take(&mut self, image: &RgbaImage) -> bool {
        let pixels = u64::from(image.width()) * u64::from(image.height());
        match self.0.checked_sub(pixels) {
            Some(rest) => {
                self.0 = rest;
                true
            }
            None => false,
        }
    }
}

/// Writes a PNG; a failure is a warning, not an error. Returns whether the file was written.
pub(super) fn save_png(pack: &mut Pack, ctx: &mut Context, path: &str, image: &RgbaImage) -> bool {
    match write_png(pack, path, image) {
        Ok(()) => true,
        Err(e) => {
            ctx.warn(format!("skipped: {e}"));
            false
        }
    }
}

/// Moves files without ever replacing one that already exists: the existing destination is
/// kept and the source is dropped. Collisions are reported once by [`Renamer::report`].
#[derive(Default)]
pub(super) struct Renamer {
    dropped: Vec<(String, String)>,
}

impl Renamer {
    /// Returns whether the source existed.
    pub(super) fn rename(&mut self, pack: &mut Pack, from: &str, to: &str) -> bool {
        if from == to {
            return pack.contains(from);
        }
        if !pack.contains(to) {
            return pack.rename(from, to);
        }
        if pack.remove(from).is_some() {
            self.dropped.push((from.to_string(), to.to_string()));
            return true;
        }
        false
    }

    /// Moves every file under directory `from` to directory `to` (both without trailing `/`).
    pub(super) fn rename_dir(&mut self, pack: &mut Pack, from: &str, to: &str) {
        let prefix = format!("{from}/");
        for path in pack.paths_with_prefix(&prefix) {
            self.rename(pack, &path, &format!("{to}/{}", &path[prefix.len()..]));
        }
    }

    pub(super) fn report(&self, ctx: &mut Context) {
        if let Some((from, to)) = self.dropped.first() {
            ctx.warn(format!(
                "{} renamed file(s) were dropped because the destination already exists \
                 (for example {from} -> {to}); the existing destination was kept",
                self.dropped.len()
            ));
        }
    }
}

/// A valid PNG whose header claims 20000x20000 pixels, which `read_png` rejects.
#[cfg(test)]
pub(super) fn oversized_png() -> Vec<u8> {
    use std::io::Cursor;

    let mut png = Vec::new();
    RgbaImage::new(1, 1)
        .write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    png[16..20].copy_from_slice(&20000u32.to_be_bytes());
    png[20..24].copy_from_slice(&20000u32.to_be_bytes());
    // IHDR is bytes 12..29; recompute its CRC-32 so the decoder reaches the size check.
    let mut crc = !0u32;
    for &byte in &png[12..29] {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    png[29..33].copy_from_slice(&(!crc).to_be_bytes());
    png
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::version::Version;

    #[test]
    fn max_scale_keeps_sheets_within_the_limit() {
        for (w, h) in [(128, 128), (256, 256), (512, 256), (64, 32)] {
            let k = max_scale(w, h);
            assert!(fits(w * k, h * k));
            assert!(!fits(w * (k + 1), h * (k + 1)));
        }
        assert_eq!(max_scale(0, 8), 0);
    }

    #[test]
    fn rename_keeps_existing_destination() {
        let mut pack = Pack::new();
        pack.insert("a", vec![1]);
        pack.insert("b", vec![2]);
        pack.insert("c", vec![3]);
        let mut renamer = Renamer::default();
        assert!(renamer.rename(&mut pack, "a", "b"));
        assert!(renamer.rename(&mut pack, "c", "d"));
        assert!(!renamer.rename(&mut pack, "missing", "d"));
        assert_eq!(pack.get("b"), Some(&[2][..]));
        assert_eq!(pack.get("d"), Some(&[3][..]));
        assert!(!pack.contains("a"));
        let mut ctx = Context::new(Version::new(1, 9, 0), Version::new(1, 10, 0));
        renamer.report(&mut ctx);
        assert_eq!(ctx.into_warnings().len(), 1);
    }
}

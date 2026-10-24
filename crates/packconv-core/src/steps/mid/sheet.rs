//! Pixel helpers and data-driven sprite sheet slicing.
//!
//! Sheet geometry is written in a "design" size (e.g. 256x256 for the particle sheet); a
//! pack with a sheet that is `k` times larger scales every box by `k`.

use image::imageops::{self, FilterType};
use image::{Rgba, RgbaImage};
use serde::Deserialize;

use crate::error::Result;
use crate::step::Context;
use crate::util::{read_png, write_png};
use crate::vfs::Pack;

const TRANSPARENT: Rgba<u8> = Rgba([0, 0, 0, 0]);

/// Mirroring applied while copying a rectangle. Every variant is its own inverse.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flip {
    None,
    Horizontal,
    Vertical,
    Both,
}

impl Flip {
    pub fn parse(code: &str) -> Flip {
        match code {
            "h" => Flip::Horizontal,
            "v" => Flip::Vertical,
            "r" => Flip::Both,
            _ => Flip::None,
        }
    }
}

/// Integer rectangle `(x, y, width, height)` in pixels.
pub type Area = (u32, u32, u32, u32);

/// Copies `area` of `src` to `(dx, dy)` of `dst`, overwriting; pixels outside either image are skipped.
pub fn copy_area(
    dst: &mut RgbaImage,
    src: &RgbaImage,
    area: Area,
    (dx, dy): (u32, u32),
    flip: Flip,
) {
    let (x, y, w, h) = area;
    for j in 0..h {
        for i in 0..w {
            let sx = x + if matches!(flip, Flip::Horizontal | Flip::Both) { w - 1 - i } else { i };
            let sy = y + if matches!(flip, Flip::Vertical | Flip::Both) { h - 1 - j } else { j };
            if sx < src.width()
                && sy < src.height()
                && dx + i < dst.width()
                && dy + j < dst.height()
            {
                dst.put_pixel(dx + i, dy + j, *src.get_pixel(sx, sy));
            }
        }
    }
}

pub fn clear_area(img: &mut RgbaImage, (x, y, w, h): Area) {
    for j in y..(y + h).min(img.height()) {
        for i in x..(x + w).min(img.width()) {
            img.put_pixel(i, j, TRANSPARENT);
        }
    }
}

pub fn crop(img: &RgbaImage, (x, y, w, h): Area) -> RgbaImage {
    imageops::crop_imm(img, x, y, w, h).to_image()
}

pub fn resize_nearest(img: &RgbaImage, width: u32, height: u32) -> RgbaImage {
    imageops::resize(img, width, height, FilterType::Nearest)
}

/// Integer factor between an image and its design size, if it is an exact multiple.
pub fn scale_factor(img: &RgbaImage, (design_w, design_h): (u32, u32)) -> Option<u32> {
    let k = img.width() / design_w;
    (k > 0 && img.width() == design_w * k && img.height() == design_h * k).then_some(k)
}

/// Reads a PNG, warning and returning `None` when it is not a valid image. A missing file is silent.
pub fn load_png(pack: &Pack, path: &str, ctx: &mut Context) -> Option<RgbaImage> {
    match read_png(pack, path)? {
        Ok(img) => Some(img),
        Err(err) => {
            ctx.warn(format!("{err}; file skipped"));
            None
        }
    }
}

#[derive(Deserialize, Clone, Copy)]
struct Cell {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

#[derive(Deserialize)]
struct RawSprite {
    path: String,
    #[serde(rename = "box")]
    cell: Cell,
    #[serde(default)]
    remove: bool,
    #[serde(default)]
    square: bool,
}

#[derive(Deserialize)]
struct RawSheet {
    path: String,
    width: u32,
    height: u32,
    #[serde(default = "default_delete")]
    delete: bool,
    textures: Vec<RawSprite>,
}

fn default_delete() -> bool {
    true
}

/// One sprite cut out of a sheet.
pub struct Sprite {
    pub path: String,
    cell: Cell,
    /// The cell is cleared in the kept sheet after cutting.
    clear: bool,
    /// The sprite file is the cell centred on a transparent square (Mojang's `SQUARE` transform).
    square: bool,
}

/// A sprite sheet that newer versions split into individual files.
pub struct Sheet {
    pub path: String,
    design: (u32, u32),
    /// The sheet stays in the pack (with cleared cells) after slicing.
    keep: bool,
    pub sprites: Vec<Sprite>,
}

impl Sheet {
    /// Parses one sheet definition, or a list of them. Paths are made relative to `root`.
    /// The definitions are embedded data, so malformed input is a programming error.
    pub fn parse_all(json: &str, root: &str) -> Vec<Sheet> {
        let raw: Vec<RawSheet> = match serde_json::from_str::<RawSheet>(json) {
            Ok(one) => vec![one],
            Err(_) => serde_json::from_str(json).expect("embedded sheet definition is valid"),
        };
        raw.into_iter()
            .map(|sheet| Sheet {
                path: format!("{root}/{}", sheet.path),
                design: (sheet.width, sheet.height),
                keep: !sheet.delete,
                sprites: sheet
                    .textures
                    .into_iter()
                    .map(|s| Sprite {
                        path: format!("{root}/{}", s.path),
                        cell: s.cell,
                        clear: s.remove,
                        square: s.square,
                    })
                    .collect(),
            })
            .collect()
    }

    /// Overrides the design size, for definitions whose boxes use another grid than they declare.
    pub fn with_design(mut self, width: u32, height: u32) -> Sheet {
        self.design = (width, height);
        self
    }

    /// Cuts the sprites out of the sheet. A missing sheet is skipped silently.
    pub fn slice(&self, pack: &mut Pack, ctx: &mut Context) -> Result<()> {
        let Some(mut sheet) = load_png(pack, &self.path, ctx) else { return Ok(()) };
        let Some(k) = scale_factor(&sheet, self.design) else {
            ctx.warn(format!(
                "{}: size {}x{} is not a multiple of {}x{}; sheet left unsliced",
                self.path,
                sheet.width(),
                sheet.height(),
                self.design.0,
                self.design.1
            ));
            return Ok(());
        };
        for sprite in &self.sprites {
            let area = sprite.area(k);
            let mut cut = crop(&sheet, area);
            if sprite.square {
                cut = pad_to_square(&cut);
            }
            write_png(pack, &sprite.path, &cut)?;
            if sprite.clear {
                clear_area(&mut sheet, area);
            }
        }
        if self.keep {
            write_png(pack, &self.path, &sheet)?;
        } else {
            pack.remove(&self.path);
        }
        Ok(())
    }

    /// Puts the sprites back into the sheet and removes their files. Returns `true` when only
    /// some of the sprites existed, so the sheet has blank cells. The caller decides whether to warn.
    pub fn unslice(&self, pack: &mut Pack, ctx: &mut Context) -> Result<bool> {
        let found: Vec<(&Sprite, RgbaImage)> = self
            .sprites
            .iter()
            .filter_map(|s| load_png(pack, &s.path, ctx).map(|img| (s, img)))
            .collect();
        if found.is_empty() {
            return Ok(false);
        }
        let base = if self.keep {
            match load_png(pack, &self.path, ctx) {
                Some(base) => Some(base),
                None => {
                    ctx.warn(format!(
                        "{}: missing, its split-off sprites were left in place",
                        self.path
                    ));
                    return Ok(false);
                }
            }
        } else {
            None
        };

        // The finest sprite decides the resolution of the rebuilt sheet.
        let base_k = base.as_ref().and_then(|b| scale_factor(b, self.design));
        let k = found
            .iter()
            .map(|(s, img)| img.width().div_ceil(s.source_size().0))
            .chain(base_k)
            .max()
            .unwrap_or(1);
        let (w, h) = (self.design.0 * k, self.design.1 * k);
        let mut sheet = match base {
            Some(base) if base.dimensions() == (w, h) => base,
            Some(base) => resize_nearest(&base, w, h),
            None => RgbaImage::new(w, h),
        };
        for (sprite, img) in &found {
            let target = sprite.area(k);
            let mut cell = if sprite.square { unpad_square(img, sprite) } else { img.clone() };
            if cell.dimensions() != (target.2, target.3) {
                cell = resize_nearest(&cell, target.2, target.3);
            }
            copy_area(
                &mut sheet,
                &cell,
                (0, 0, target.2, target.3),
                (target.0, target.1),
                Flip::None,
            );
        }
        write_png(pack, &self.path, &sheet)?;
        for (sprite, _) in &found {
            pack.remove(&sprite.path);
            pack.remove(&format!("{}.mcmeta", sprite.path));
        }
        Ok(found.len() < self.sprites.len())
    }
}

impl Sprite {
    fn area(&self, k: u32) -> Area {
        let b = self.cell;
        (b.x * k, b.y * k, b.width * k, b.height * k)
    }

    /// Design size of the sprite file: the cell, or its bounding square.
    fn source_size(&self) -> (u32, u32) {
        let side = self.cell.width.max(self.cell.height);
        if self.square {
            (side, side)
        } else {
            (self.cell.width, self.cell.height)
        }
    }
}

/// Centres the image on a transparent square canvas.
fn pad_to_square(img: &RgbaImage) -> RgbaImage {
    let side = img.width().max(img.height());
    let mut out = RgbaImage::new(side, side);
    let at = ((side - img.width()) / 2, (side - img.height()) / 2);
    copy_area(&mut out, img, (0, 0, img.width(), img.height()), at, Flip::None);
    out
}

/// Inverse of [`pad_to_square`] for a sprite whose cell has the proportions of `sprite.cell`.
fn unpad_square(img: &RgbaImage, sprite: &Sprite) -> RgbaImage {
    let side = img.width();
    let b = sprite.cell;
    let (w, h) = (b.width * side / b.width.max(b.height), b.height * side / b.width.max(b.height));
    crop(img, ((side - w) / 2, (side - h) / 2, w, h))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::version::Version;

    fn ctx() -> Context {
        Context::new(Version::new(1, 13, 0), Version::new(1, 14, 0))
    }

    /// 8x8 sheet with four 4x4 cells in distinct colours.
    fn sheet_image(scale: u32) -> RgbaImage {
        RgbaImage::from_fn(8 * scale, 8 * scale, |x, y| {
            let cell = (x / (4 * scale)) + 2 * (y / (4 * scale));
            Rgba([10 + cell as u8 * 50, (x % 7) as u8, (y % 5) as u8, 255])
        })
    }

    const DEF: &str = r#"{"path":"s.png","width":8,"height":8,"delete":true,"textures":[
        {"path":"a.png","box":{"x":0,"y":0,"width":4,"height":4}},
        {"path":"b.png","box":{"x":4,"y":4,"width":4,"height":4}}]}"#;

    #[test]
    fn slices_and_restores_scaled_sheet() {
        for scale in [1, 2, 4] {
            let sheet = Sheet::parse_all(DEF, "root").remove(0);
            let original = sheet_image(scale);
            let mut pack = Pack::new();
            write_png(&mut pack, "root/s.png", &original).unwrap();
            sheet.slice(&mut pack, &mut ctx()).unwrap();
            assert!(!pack.contains("root/s.png"));
            let a = read_png(&pack, "root/a.png").unwrap().unwrap();
            assert_eq!(a.dimensions(), (4 * scale, 4 * scale));
            assert_eq!(a, crop(&original, (0, 0, 4 * scale, 4 * scale)));

            let partial = sheet.unslice(&mut pack, &mut ctx()).unwrap();
            assert!(!partial);
            assert!(!pack.contains("root/a.png"));
            let rebuilt = read_png(&pack, "root/s.png").unwrap().unwrap();
            assert_eq!(crop(&rebuilt, (0, 0, 4 * scale, 4 * scale)), a);
            assert_eq!(
                crop(&rebuilt, (4 * scale, 4 * scale, 4 * scale, 4 * scale)),
                crop(&original, (4 * scale, 4 * scale, 4 * scale, 4 * scale))
            );
        }
    }

    #[test]
    fn unslice_upscales_to_the_finest_sprite_and_reports_partial_sets() {
        let sheet = Sheet::parse_all(DEF, "root").remove(0);
        let mut pack = Pack::new();
        write_png(&mut pack, "root/a.png", &RgbaImage::from_pixel(8, 8, Rgba([1, 2, 3, 255])))
            .unwrap();
        assert!(sheet.unslice(&mut pack, &mut ctx()).unwrap());
        let rebuilt = read_png(&pack, "root/s.png").unwrap().unwrap();
        assert_eq!(rebuilt.dimensions(), (16, 16));
        assert_eq!(rebuilt.get_pixel(7, 7), &Rgba([1, 2, 3, 255]));
        assert_eq!(rebuilt.get_pixel(8, 8)[3], 0);
    }

    #[test]
    fn kept_sheet_has_its_cells_cleared_and_gets_them_back() {
        let json = r#"{"path":"s.png","width":8,"height":8,"delete":false,"textures":[
            {"path":"a.png","remove":true,"box":{"x":0,"y":0,"width":4,"height":4}}]}"#;
        let sheet = Sheet::parse_all(json, "r").remove(0);
        let original = sheet_image(1);
        let mut pack = Pack::new();
        write_png(&mut pack, "r/s.png", &original).unwrap();
        sheet.slice(&mut pack, &mut ctx()).unwrap();
        let cleared = read_png(&pack, "r/s.png").unwrap().unwrap();
        assert_eq!(cleared.get_pixel(1, 1)[3], 0);
        assert_eq!(cleared.get_pixel(5, 1), original.get_pixel(5, 1));
        sheet.unslice(&mut pack, &mut ctx()).unwrap();
        assert_eq!(read_png(&pack, "r/s.png").unwrap().unwrap(), original);
    }

    #[test]
    fn square_sprites_are_padded_and_cropped_back() {
        let json = r#"{"path":"s.png","width":8,"height":4,"textures":[
            {"path":"a.png","square":true,"box":{"x":0,"y":0,"width":8,"height":4}}]}"#;
        let sheet = Sheet::parse_all(json, "r").remove(0);
        let original = RgbaImage::from_pixel(8, 4, Rgba([9, 9, 9, 255]));
        let mut pack = Pack::new();
        write_png(&mut pack, "r/s.png", &original).unwrap();
        sheet.slice(&mut pack, &mut ctx()).unwrap();
        let padded = read_png(&pack, "r/a.png").unwrap().unwrap();
        assert_eq!(padded.dimensions(), (8, 8));
        assert_eq!((padded.get_pixel(0, 1)[3], padded.get_pixel(0, 2)[3]), (0, 255));
        sheet.unslice(&mut pack, &mut ctx()).unwrap();
        assert_eq!(read_png(&pack, "r/s.png").unwrap().unwrap(), original);
    }

    #[test]
    fn unexpected_sheet_size_is_left_alone_with_a_warning() {
        let sheet = Sheet::parse_all(DEF, "root").remove(0);
        let mut pack = Pack::new();
        write_png(&mut pack, "root/s.png", &RgbaImage::new(12, 12)).unwrap();
        let mut ctx = ctx();
        sheet.slice(&mut pack, &mut ctx).unwrap();
        assert!(pack.contains("root/s.png"));
        assert_eq!(ctx.into_warnings().len(), 1);
    }

    #[test]
    fn flips_are_involutions() {
        let src = RgbaImage::from_fn(3, 2, |x, y| Rgba([x as u8, y as u8, 0, 255]));
        for flip in [Flip::None, Flip::Horizontal, Flip::Vertical, Flip::Both] {
            let mut once = RgbaImage::new(3, 2);
            copy_area(&mut once, &src, (0, 0, 3, 2), (0, 0), flip);
            let mut twice = RgbaImage::new(3, 2);
            copy_area(&mut twice, &once, (0, 0, 3, 2), (0, 0), flip);
            assert_eq!(twice, src);
        }
    }
}

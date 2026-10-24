//! 1.15: the chests got a new texture layout (and double chests two halves), the enchantment
//! glint is no longer tinted at render time, and the iron golem texture moved into a folder.

use std::sync::OnceLock;

use image::RgbaImage;
use serde::Deserialize;

use crate::error::Result;
use crate::step::{Context, Step};
use crate::steps::support::{canvas, save_png, Renamer};
use crate::version::Version;
use crate::vfs::Pack;

use super::rename::move_texture;
use super::sheet::{copy_area, load_png, scale_factor, Flip};

const CHEST_DIR: &str = "assets/minecraft/textures/entity/chest";
const GLINT: &str = "assets/minecraft/textures/misc/enchanted_item_glint.png";
/// The colour 1.14 multiplied the grey glint with at render time (0x8040CC).
const GLINT_TINT: [u32; 3] = [128, 64, 204];

/// One rectangle copy of the pre-1.15 layout, `(src x, y, width, height, dst x, y, flip)` in
/// design units. The 1.15 layout is derived by running the copies forwards; the way back swaps
/// source and destination, which is exact because every flip is its own inverse.
#[derive(Deserialize)]
struct Op(u32, u32, u32, u32, u32, u32, String);

#[derive(Deserialize)]
struct Layout {
    /// Size of the pre-1.15 texture.
    design: (u32, u32),
    /// Size of the 1.15 texture.
    out: (u32, u32),
    ops: Vec<Op>,
}

#[derive(Deserialize)]
struct Chests {
    single: Layout,
    double_left: Layout,
    double_right: Layout,
}

fn chests() -> &'static Chests {
    static CHESTS: OnceLock<Chests> = OnceLock::new();
    CHESTS.get_or_init(|| {
        serde_json::from_str(include_str!("../../../data/mid/chest_1_15.json"))
            .expect("embedded chest layout is valid")
    })
}

impl Layout {
    /// Runs the copies from the old layout (`src`) into a fresh new-layout texture.
    fn forward(&self, ctx: &mut Context, path: &str, src: &RgbaImage, k: u32) -> Option<RgbaImage> {
        let mut out = canvas(ctx, path, self.out.0.checked_mul(k), self.out.1.checked_mul(k))?;
        self.run(src, &mut out, k, false);
        Some(out)
    }

    /// An empty old-layout texture for [`Layout::backward`].
    fn old_canvas(&self, ctx: &mut Context, path: &str, k: u32) -> Option<RgbaImage> {
        canvas(ctx, path, self.design.0.checked_mul(k), self.design.1.checked_mul(k))
    }

    /// Runs the copies backwards, from the new layout (`src`) into `dst` (the old layout).
    fn backward(&self, src: &RgbaImage, dst: &mut RgbaImage, k: u32) {
        self.run(src, dst, k, true);
    }

    fn run(&self, src: &RgbaImage, dst: &mut RgbaImage, k: u32, inverse: bool) {
        for Op(sx, sy, w, h, dx, dy, flip) in &self.ops {
            let (from, to) =
                if inverse { ((*dx, *dy), (*sx, *sy)) } else { ((*sx, *sy), (*dx, *dy)) };
            copy_area(
                dst,
                src,
                (from.0 * k, from.1 * k, w * k, h * k),
                (to.0 * k, to.1 * k),
                Flip::parse(flip),
            );
        }
    }
}

/// Texture names of single chests; the first three also exist as double chests.
const SINGLE: [&str; 4] = ["normal", "trapped", "christmas", "ender"];
const DOUBLE: [&str; 3] = ["normal", "trapped", "christmas"];

pub struct V1_15;

impl Step for V1_15 {
    fn name(&self) -> &'static str {
        "1.15 chests, enchantment glint, iron golem"
    }

    fn version(&self) -> Version {
        Version::new(1, 15, 0)
    }

    fn upgrade(&self, pack: &mut Pack, ctx: &mut Context) -> Result<()> {
        let layouts = chests();
        for name in DOUBLE {
            let path = format!("{CHEST_DIR}/{name}_double.png");
            let Some(double) = load_png(pack, &path, ctx) else { continue };
            let Some(k) = scale_factor(&double, layouts.double_left.design) else {
                ctx.warn(format!(
                    "{path}: unexpected size {}x{}; left unconverted",
                    double.width(),
                    double.height()
                ));
                continue;
            };
            let (left_path, right_path) =
                (format!("{CHEST_DIR}/{name}_left.png"), format!("{CHEST_DIR}/{name}_right.png"));
            let left = layouts.double_left.forward(ctx, &left_path, &double, k);
            let right = layouts.double_right.forward(ctx, &right_path, &double, k);
            let (Some(left), Some(right)) = (left, right) else { continue };
            if save_png(pack, ctx, &left_path, &left) && save_png(pack, ctx, &right_path, &right) {
                pack.remove(&path);
            }
        }
        for name in SINGLE {
            let path = format!("{CHEST_DIR}/{name}.png");
            let Some(chest) = load_png(pack, &path, ctx) else { continue };
            let Some(k) = scale_factor(&chest, layouts.single.design) else {
                ctx.warn(format!(
                    "{path}: unexpected size {}x{}; left unconverted",
                    chest.width(),
                    chest.height()
                ));
                continue;
            };
            if let Some(converted) = layouts.single.forward(ctx, &path, &chest, k) {
                save_png(pack, ctx, &path, &converted);
            }
        }

        if let Some(mut glint) = load_png(pack, GLINT, ctx) {
            for pixel in glint.pixels_mut().filter(|p| p[3] != 0) {
                // 1.14 tinted by the red channel of the grey texture.
                let grey = u32::from(pixel[0]);
                for (channel, tint) in pixel.0.iter_mut().zip(GLINT_TINT) {
                    *channel = (grey * tint / 255) as u8;
                }
            }
            save_png(pack, ctx, GLINT, &glint);
        }

        let mut renamer = Renamer::default();
        move_texture(pack, &mut renamer, "entity/iron_golem", "entity/iron_golem/iron_golem");
        renamer.report(ctx);
        Ok(())
    }

    fn downgrade(&self, pack: &mut Pack, ctx: &mut Context) -> Result<()> {
        let layouts = chests();
        for name in DOUBLE {
            let (left_path, right_path) =
                (format!("{CHEST_DIR}/{name}_left.png"), format!("{CHEST_DIR}/{name}_right.png"));
            let (left, right) = (load_png(pack, &left_path, ctx), load_png(pack, &right_path, ctx));
            let (Some(left), Some(right)) = (left, right) else {
                if pack.contains(&left_path) != pack.contains(&right_path) {
                    ctx.warn(format!("{CHEST_DIR}/{name}: only one half of the double chest exists, it cannot be converted"));
                }
                continue;
            };
            let Some(k) = scale_factor(&left, layouts.double_left.out)
                .filter(|&k| scale_factor(&right, layouts.double_right.out) == Some(k))
            else {
                ctx.warn(format!("{CHEST_DIR}/{name}: the halves have different or unexpected sizes; left unconverted"));
                continue;
            };
            let double_path = format!("{CHEST_DIR}/{name}_double.png");
            let Some(mut double) = layouts.double_left.old_canvas(ctx, &double_path, k) else {
                continue;
            };
            layouts.double_left.backward(&left, &mut double, k);
            layouts.double_right.backward(&right, &mut double, k);
            if save_png(pack, ctx, &double_path, &double) {
                pack.remove(&left_path);
                pack.remove(&right_path);
            }
        }
        for name in SINGLE {
            let path = format!("{CHEST_DIR}/{name}.png");
            let Some(chest) = load_png(pack, &path, ctx) else { continue };
            let Some(k) = scale_factor(&chest, layouts.single.out) else {
                ctx.warn(format!(
                    "{path}: unexpected size {}x{}; left unconverted",
                    chest.width(),
                    chest.height()
                ));
                continue;
            };
            let Some(mut old) = layouts.single.old_canvas(ctx, &path, k) else { continue };
            layouts.single.backward(&chest, &mut old, k);
            save_png(pack, ctx, &path, &old);
        }

        if let Some(mut glint) = load_png(pack, GLINT, ctx) {
            // 1.14 multiplies a grey texture with the tint, so pick the grey whose tinted colour
            // is closest (least squares) to each pixel; that inverts the upgrade exactly.
            let norm: u32 = GLINT_TINT.iter().map(|t| t * t).sum();
            for pixel in glint.pixels_mut().filter(|p| p[3] != 0) {
                let dot: u32 = pixel.0.iter().zip(GLINT_TINT).map(|(&c, t)| u32::from(c) * t).sum();
                let grey = ((255 * dot + norm / 2) / norm).min(255) as u8;
                pixel.0[..3].fill(grey);
            }
            if save_png(pack, ctx, GLINT, &glint) {
                ctx.warn("the enchantment glint was converted to grey, where 1.14 tints it purple; colours that are not purple are approximated");
            }
        }

        let mut renamer = Renamer::default();
        move_texture(pack, &mut renamer, "entity/iron_golem/iron_golem", "entity/iron_golem");
        renamer.report(ctx);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::{read_png, write_png};
    use image::Rgba;

    fn ctx() -> Context {
        Context::new(Version::new(1, 14, 0), Version::new(1, 15, 0))
    }

    /// Every pixel gets a distinct colour, so any misplaced copy shows up.
    fn pattern(w: u32, h: u32) -> RgbaImage {
        RgbaImage::from_fn(w, h, |x, y| {
            Rgba([(x % 256) as u8, (y % 256) as u8, ((x / 256) + 7 * (y / 256)) as u8, 255])
        })
    }

    /// Pixels the layout copies from the old texture.
    fn covered(layout: &Layout, k: u32, size: (u32, u32), inverse: bool) -> Vec<Vec<u32>> {
        let mut count = vec![vec![0u32; (size.0 * k) as usize]; (size.1 * k) as usize];
        for Op(sx, sy, w, h, dx, dy, _) in &layout.ops {
            let (x, y) = if inverse { (dx, dy) } else { (sx, sy) };
            for j in 0..h * k {
                for i in 0..w * k {
                    count[(y * k + j) as usize][(x * k + i) as usize] += 1;
                }
            }
        }
        count
    }

    #[test]
    fn layouts_copy_each_pixel_at_most_once_on_both_sides() {
        let layouts = chests();
        for (layout, name) in [
            (&layouts.single, "single"),
            (&layouts.double_left, "left"),
            (&layouts.double_right, "right"),
        ] {
            for inverse in [false, true] {
                let size = if inverse { layout.out } else { layout.design };
                assert!(
                    covered(layout, 1, size, inverse).iter().flatten().all(|&n| n <= 1),
                    "{name} inverse={inverse}"
                );
            }
        }
        // The two double chest halves read different parts of the double texture.
        let left = covered(&layouts.double_left, 1, layouts.double_left.design, false);
        let right = covered(&layouts.double_right, 1, layouts.double_right.design, false);
        assert!(left.iter().flatten().zip(right.iter().flatten()).all(|(l, r)| l + r <= 1));
    }

    #[test]
    fn single_chest_roundtrip_restores_every_copied_pixel() {
        for k in [1, 2] {
            let mut pack = Pack::new();
            let old = pattern(64 * k, 64 * k);
            write_png(&mut pack, &format!("{CHEST_DIR}/normal.png"), &old).unwrap();
            V1_15.upgrade(&mut pack, &mut ctx()).unwrap();
            let new = read_png(&pack, &format!("{CHEST_DIR}/normal.png")).unwrap().unwrap();
            assert_ne!(new, old);
            V1_15.downgrade(&mut pack, &mut ctx()).unwrap();
            let back = read_png(&pack, &format!("{CHEST_DIR}/normal.png")).unwrap().unwrap();
            let used = covered(&chests().single, k, (64, 64), false);
            for (y, row) in used.iter().enumerate() {
                for (x, &n) in row.iter().enumerate() {
                    if n > 0 {
                        assert_eq!(
                            back.get_pixel(x as u32, y as u32),
                            old.get_pixel(x as u32, y as u32)
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn double_chest_is_split_and_recombined() {
        let mut pack = Pack::new();
        let old = pattern(128, 64);
        write_png(&mut pack, &format!("{CHEST_DIR}/trapped_double.png"), &old).unwrap();
        V1_15.upgrade(&mut pack, &mut ctx()).unwrap();
        assert!(!pack.contains(&format!("{CHEST_DIR}/trapped_double.png")));
        let left = read_png(&pack, &format!("{CHEST_DIR}/trapped_left.png")).unwrap().unwrap();
        let right = read_png(&pack, &format!("{CHEST_DIR}/trapped_right.png")).unwrap().unwrap();
        assert_eq!((left.dimensions(), right.dimensions()), ((64, 64), (64, 64)));
        // The first copy of the left half: old [59,19 .. 74,33) flipped vertically to (14,19).
        assert_eq!(left.get_pixel(14, 19), old.get_pixel(59, 32));

        V1_15.downgrade(&mut pack, &mut ctx()).unwrap();
        assert!(!pack.contains(&format!("{CHEST_DIR}/trapped_left.png")));
        let back = read_png(&pack, &format!("{CHEST_DIR}/trapped_double.png")).unwrap().unwrap();
        assert_eq!(back.dimensions(), (128, 64));
        assert_eq!(back.get_pixel(59, 19), old.get_pixel(59, 19));
        assert_eq!(back.get_pixel(14, 0), old.get_pixel(14, 0));
    }

    #[test]
    fn lone_double_chest_half_is_reported() {
        let mut pack = Pack::new();
        write_png(&mut pack, &format!("{CHEST_DIR}/normal_left.png"), &pattern(64, 64)).unwrap();
        let mut ctx = ctx();
        V1_15.downgrade(&mut pack, &mut ctx).unwrap();
        assert!(pack.contains(&format!("{CHEST_DIR}/normal_left.png")));
        assert_eq!(ctx.into_warnings().len(), 1);
    }

    #[test]
    fn glint_is_tinted_and_greyed_back() {
        let mut pack = Pack::new();
        let grey = RgbaImage::from_fn(2, 1, |x, _| {
            if x == 0 {
                Rgba([200, 200, 200, 255])
            } else {
                Rgba([0, 0, 0, 0])
            }
        });
        write_png(&mut pack, GLINT, &grey).unwrap();
        V1_15.upgrade(&mut pack, &mut ctx()).unwrap();
        let tinted = read_png(&pack, GLINT).unwrap().unwrap();
        assert_eq!(tinted.get_pixel(0, 0), &Rgba([100, 50, 160, 255]));
        assert_eq!(tinted.get_pixel(1, 0)[3], 0);
        V1_15.downgrade(&mut pack, &mut ctx()).unwrap();
        let back = read_png(&pack, GLINT).unwrap().unwrap();
        assert_eq!(back.get_pixel(0, 0), &Rgba([200, 200, 200, 255]));
    }

    #[test]
    fn iron_golem_moves_into_its_folder_and_back() {
        let mut pack = Pack::new();
        pack.insert("assets/minecraft/textures/entity/iron_golem.png", vec![1]);
        pack.insert("assets/minecraft/textures/entity/iron_golem.png.mcmeta", vec![2]);
        V1_15.upgrade(&mut pack, &mut ctx()).unwrap();
        assert!(pack.contains("assets/minecraft/textures/entity/iron_golem/iron_golem.png.mcmeta"));
        V1_15.downgrade(&mut pack, &mut ctx()).unwrap();
        assert!(pack.contains("assets/minecraft/textures/entity/iron_golem.png"));
    }

    #[test]
    fn unexpected_sizes_are_left_alone() {
        let mut pack = Pack::new();
        write_png(&mut pack, &format!("{CHEST_DIR}/ender.png"), &pattern(60, 60)).unwrap();
        let before = pack.get(&format!("{CHEST_DIR}/ender.png")).unwrap().to_vec();
        let mut ctx = ctx();
        V1_15.upgrade(&mut pack, &mut ctx).unwrap();
        assert_eq!(pack.get(&format!("{CHEST_DIR}/ender.png")).unwrap(), before);
        assert_eq!(ctx.into_warnings().len(), 1);
    }
}

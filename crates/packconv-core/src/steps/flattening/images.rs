//! Texture sheets whose pixels changed in 1.13: water tint, the particle sheet and map icons.

use image::imageops;

use super::maps::Direction;
use crate::step::Context;
use crate::steps::support::{canvas, load_png, save_png};
use crate::vfs::Pack;

pub fn convert(pack: &mut Pack, ctx: &mut Context, direction: Direction) {
    convert_water(pack, ctx, direction);
    convert_particles(pack, ctx, direction);
    convert_map_icons(pack, ctx, direction);
}

/// 1.13 tints water by biome, so the textures turned grayscale; 1.12 had the blue baked in.
fn convert_water(pack: &mut Pack, ctx: &mut Context, direction: Direction) {
    /// Blue multiplier of the 1.12 water textures.
    const WATER_TINT: [u32; 3] = [45, 63, 244];
    let directory = match direction {
        Direction::Up => "block",
        Direction::Down => "blocks",
    };
    let mut converted = false;
    for name in ["water_still", "water_flow", "water_overlay"] {
        let path = format!("assets/minecraft/textures/{directory}/{name}.png");
        let Some(mut image) = load_png(pack, ctx, &path) else {
            continue;
        };
        for pixel in image.pixels_mut() {
            let [r, g, b, a] = pixel.0;
            pixel.0 = match direction {
                Direction::Up => {
                    let gray =
                        (0.299 * f32::from(r) + 0.587 * f32::from(g) + 0.114 * f32::from(b)) as u8;
                    [gray, gray, gray, a]
                }
                // The red channel carries the gray value of an already gray texture.
                Direction::Down if a != 0 => {
                    let tint = |channel: u32| (u32::from(r) * channel / 255) as u8;
                    [
                        tint(WATER_TINT[0]),
                        tint(WATER_TINT[1]),
                        tint(WATER_TINT[2]),
                        a,
                    ]
                }
                Direction::Down => pixel.0,
            };
        }
        converted |= save_png(pack, ctx, &path, &image);
    }
    if converted {
        ctx.warn(match direction {
            Direction::Up => {
                "water textures were converted to grayscale: 1.13 tints water by biome"
            }
            Direction::Down => {
                "water textures were re-tinted with the 1.12 blue; the biome tint is gone"
            }
        });
    }
}

/// The 1.13 particle sheet is 256x256 (twice the size of 1.12's 128x128) with the old sprites
/// in the top-left quadrant, so only the canvas changes.
fn convert_particles(pack: &mut Pack, ctx: &mut Context, direction: Direction) {
    let path = "assets/minecraft/textures/particle/particles.png";
    let Some(image) = load_png(pack, ctx, path) else {
        return;
    };
    let size = image.width();
    if image.height() != size || size % 2 != 0 {
        ctx.warn(format!(
            "{path}: particle sheet is not square, left unconverted"
        ));
        return;
    }
    let converted = match direction {
        Direction::Up => {
            let Some(mut sheet) = canvas(ctx, path, size.checked_mul(2), size.checked_mul(2))
            else {
                return;
            };
            imageops::replace(&mut sheet, &image, 0, 0);
            sheet
        }
        Direction::Down => {
            let half = size / 2;
            let kept = imageops::crop_imm(&image, 0, 0, half, half).to_image();
            let dropped_sprites = image
                .enumerate_pixels()
                .any(|(x, y, p)| (x >= half || y >= half) && p.0[3] != 0);
            if dropped_sprites {
                ctx.warn("particle sheet: sprites outside the top-left quadrant do not exist in 1.12 and were dropped");
            }
            kept
        }
    };
    save_png(pack, ctx, path, &converted);
}

/// Icons in the 1.12 sheet. Both sheets store icon `i` row-major, 8 px cells at vanilla scale:
/// the 1.12 sheet is a 4x4 grid (32 px wide), the 1.13 sheet a 16-wide grid (128 px wide).
const MAP_ICON_COUNT: u32 = 10;
const OLD_COLUMNS: u32 = 4;
const NEW_COLUMNS: u32 = 16;

fn convert_map_icons(pack: &mut Pack, ctx: &mut Context, direction: Direction) {
    let path = "assets/minecraft/textures/map/map_icons.png";
    let Some(image) = load_png(pack, ctx, path) else {
        return;
    };
    let size = image.width();
    // A 1.12 sheet scaled 4x is as wide as a vanilla-scale 1.13 sheet, so only sizes that
    // cannot be mistaken for the other layout are converted.
    let scale = match direction {
        Direction::Up if size < 128 && size % 32 == 0 => size / 32,
        Direction::Down if size % 128 == 0 => size / 128,
        _ => {
            ctx.warn(format!(
                "{path}: map icon sheet size does not match the expected layout, left unconverted"
            ));
            return;
        }
    };
    if image.height() != size {
        ctx.warn(format!(
            "{path}: map icon sheet is not square, left unconverted"
        ));
        return;
    }
    let cell = 8 * scale;
    let (from_columns, to_columns, to_size) = match direction {
        Direction::Up => (OLD_COLUMNS, NEW_COLUMNS, size * 4),
        Direction::Down => (NEW_COLUMNS, OLD_COLUMNS, size / 4),
    };
    let Some(mut sheet) = canvas(ctx, path, Some(to_size), Some(to_size)) else {
        return;
    };
    for index in 0..MAP_ICON_COUNT {
        let icon = imageops::crop_imm(
            &image,
            index % from_columns * cell,
            index / from_columns * cell,
            cell,
            cell,
        );
        imageops::replace(
            &mut sheet,
            &*icon,
            i64::from(index % to_columns * cell),
            i64::from(index / to_columns * cell),
        );
    }
    if direction == Direction::Up {
        ctx.warn("map icons were repacked into the 1.13 layout; icons added in 1.13 are blank");
    }
    save_png(pack, ctx, path, &sheet);
}

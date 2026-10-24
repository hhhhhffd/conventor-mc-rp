//! 1.20.2: GUI sheets were cut into individual sprites under `textures/gui/sprites/`.
//!
//! The boxes come from Mojang's own slicer (`data/mojang-slicer/gui-1.20.2-boxes.json`). Boxes
//! are in the design space of the sheet (usually 256x256), so HD sheets are scaled by
//! `image size / design size`. Downgrading pastes the sprites back onto the old sheets.

use std::sync::OnceLock;

use image::RgbaImage;
use serde::Deserialize;
use serde_json::{json, Value};

use super::common::{crop, move_with_mcmeta, paste, resize};
use crate::error::Result;
use crate::step::{Context, Step};
use crate::steps::support::{canvas, load_png, max_scale, save_png, PixelBudget, Renamer};
use crate::util::{read_png, write_json};
use crate::version::Version;
use crate::vfs::Pack;

/// `[input sheet, output sprite, [x, y, w, h, design_w, design_h], modifiers]`.
#[derive(Deserialize)]
struct RawRow(String, String, [u32; 6], Vec<String>);

/// Extra pixel work Mojang applies to a few sprites after cutting them.
#[derive(Clone, Copy)]
enum Transform {
    /// `n` frames laid side by side become `n` frames stacked vertically.
    FlipFrameAxis(u32),
    /// The sprite is drawn at `(x, y)` on a transparent `w x h` canvas; sizes are in the
    /// design space of a `src_w x src_h` sprite.
    Extend { src_w: u32, src_h: u32, x: u32, y: u32, w: u32, h: u32 },
}

struct Row {
    output: String,
    /// x, y, w, h, design_w, design_h
    boxed: [u32; 6],
    transform: Option<Transform>,
    has_metadata: bool,
}

struct Sheet {
    input: String,
    rows: Vec<Row>,
}

const SPRITES: &str = "assets/minecraft/textures/gui/sprites/";

/// Sheets that still exist in 1.20.2 (container backgrounds and books): sprites are copied out
/// of them but the sheet itself stays.
const KEPT_SHEETS: &[&str] = &[
    "anvil", "beacon", "blast_furnace", "brewing_stand", "cartography_table", "enchanting_table",
    "furnace", "gamemode_switcher", "grindstone", "horse", "inventory", "loom", "smithing",
    "smoker", "stonecutter",
];

/// 1.20.2 moved these Realms textures into the `minecraft` namespace.
const REALMS_MOVES: &[(&str, &str)] = &[
    ("assets/realms/textures/gui/realms/adventure.png", "assets/minecraft/textures/gui/realms/adventure.png"),
    ("assets/realms/textures/gui/realms/empty_frame.png", "assets/minecraft/textures/gui/realms/empty_frame.png"),
    ("assets/realms/textures/gui/realms/experience.png", "assets/minecraft/textures/gui/realms/experience.png"),
    ("assets/realms/textures/gui/realms/inspiration.png", "assets/minecraft/textures/gui/realms/inspiration.png"),
    ("assets/realms/textures/gui/realms/new_world.png", "assets/minecraft/textures/gui/realms/new_world.png"),
    ("assets/realms/textures/gui/realms/survival_spawn.png", "assets/minecraft/textures/gui/realms/survival_spawn.png"),
    ("assets/realms/textures/gui/realms/upload.png", "assets/minecraft/textures/gui/realms/upload.png"),
    ("assets/realms/textures/gui/title/realms.png", "assets/minecraft/textures/gui/title/realms.png"),
];

/// The villager sheet shrank to its panel: `villager2.png` (512x256) -> `villager.png` (276x166).
const VILLAGER_SHEET: &str = "assets/minecraft/textures/gui/container/villager2.png";
const VILLAGER_PANEL: &str = "assets/minecraft/textures/gui/container/villager.png";
const VILLAGER_BOX: [u32; 6] = [0, 0, 276, 166, 512, 256];

fn parse_transform(modifier: &str) -> Option<Transform> {
    match modifier.strip_prefix("apply:")? {
        "flipFrameAxis(2)" => Some(Transform::FlipFrameAxis(2)),
        "spriteExtender(17, 16, 3, 4, 26, 26)" => {
            Some(Transform::Extend { src_w: 17, src_h: 16, x: 3, y: 4, w: 26, h: 26 })
        }
        other => panic!("unsupported slicer modifier {other}"),
    }
}

fn sheets() -> &'static [Sheet] {
    static SHEETS: OnceLock<Vec<Sheet>> = OnceLock::new();
    SHEETS.get_or_init(|| {
        let raw: Vec<RawRow> = serde_json::from_str(include_str!("../../../data/mojang-slicer/gui-1.20.2-boxes.json"))
            .expect("vendored slicer boxes are valid JSON");
        let mut sheets: Vec<Sheet> = Vec::new();
        for RawRow(input, output, boxed, modifiers) in raw {
            // Inputs inside `sprites/` and the bundle background only existed in 1.20.2 snapshots.
            if input.starts_with(SPRITES) || input.ends_with("/bundle_background.png") {
                continue;
            }
            let row = Row {
                output,
                boxed,
                transform: modifiers.iter().find_map(|m| parse_transform(m)),
                has_metadata: modifiers.iter().any(|m| m == "metadata"),
            };
            match sheets.iter_mut().find(|s| s.input == input) {
                Some(sheet) => sheet.rows.push(row),
                None => sheets.push(Sheet { input, rows: vec![row] }),
            }
        }
        sheets
    })
}

/// `.mcmeta` content the slicer writes next to sprites that need a nine-slice or animation.
fn sprite_metadata(output: &str) -> Value {
    let nine_slice = |width: u32, height: u32, border: Value| {
        json!({"gui": {"scaling": {"type": "nine_slice", "width": width, "height": height, "border": border}}})
    };
    let button = || nine_slice(200, 20, json!({"left": 20, "top": 4, "right": 20, "bottom": 4}));
    let name = output.strip_prefix(SPRITES).unwrap_or(output);
    match name {
        "widget/button.png" | "widget/button_disabled.png" | "widget/button_highlighted.png"
        | "widget/slider.png" | "widget/slider_highlighted.png" | "widget/slider_handle.png"
        | "widget/slider_handle_highlighted.png" => button(),
        "widget/tab.png" | "widget/tab_selected.png" | "widget/tab_highlighted.png"
        | "widget/tab_selected_highlighted.png" => {
            nine_slice(130, 24, json!({"left": 2, "top": 2, "right": 2, "bottom": 0}))
        }
        "advancements/title_box.png" => nine_slice(200, 26, json!(10)),
        "container/bundle/background.png" | "recipe_book/overlay_recipe.png" => nine_slice(32, 32, json!(4)),
        "social_interactions/background.png" => nine_slice(236, 34, json!(8)),
        "realm_status/expires_soon.png" => json!({"animation": {"frametime": 10, "height": 28}}),
        "icon/trial_available.png" => json!({"animation": {"frametime": 20}}),
        _ => unreachable!("no metadata defined for {output}"),
    }
}

/// Box in pixels of an image of `width x height`, using the slicer's integer scaling. The
/// products need 64 bits: a wide image times a box coordinate overflows `u32`.
fn scaled_box(b: &[u32; 6], width: u32, height: u32) -> Option<(u32, u32, u32, u32)> {
    let scale = |value: u32, size: u32, design: u32| {
        u32::try_from(u64::from(value) * u64::from(size) / u64::from(design.max(1))).ok()
    };
    Some((
        scale(b[0], width, b[4])?,
        scale(b[1], height, b[5])?,
        scale(b[2], width, b[4])?,
        scale(b[3], height, b[5])?,
    ))
}

/// Applies the slicer's post-processing to a freshly cut sprite. `None` (with a warning) when
/// the sprite is too small or too large for it.
fn apply(ctx: &mut Context, path: &str, transform: Transform, sprite: &RgbaImage) -> Option<RgbaImage> {
    match transform {
        Transform::FlipFrameAxis(n) => {
            let (frame_w, h) = (sprite.width() / n, sprite.height());
            if frame_w == 0 {
                ctx.warn(format!("{path}: sprite is too narrow for {n} frames, skipped"));
                return None;
            }
            let mut out = canvas(ctx, path, Some(frame_w), h.checked_mul(n))?;
            for i in 0..n {
                if let Some(frame) = crop(sprite, i * frame_w, 0, frame_w, h) {
                    paste(&mut out, &frame, 0, i * h);
                }
            }
            Some(out)
        }
        Transform::Extend { src_w, x, y, w, h, .. } => {
            let s = sprite.width() / src_w.max(1);
            if s == 0 {
                ctx.warn(format!("{path}: sprite is below its design size, skipped"));
                return None;
            }
            let mut out = canvas(ctx, path, w.checked_mul(s), h.checked_mul(s))?;
            paste(&mut out, sprite, x * s, y * s);
            Some(out)
        }
    }
}

fn unapply(ctx: &mut Context, path: &str, transform: Transform, sprite: &RgbaImage) -> Option<RgbaImage> {
    match transform {
        Transform::FlipFrameAxis(n) => {
            let (w, frame_h) = (sprite.width(), sprite.height() / n);
            if frame_h == 0 {
                ctx.warn(format!("{path}: sprite is too short for {n} frames, not merged"));
                return None;
            }
            let mut out = canvas(ctx, path, w.checked_mul(n), Some(frame_h))?;
            for i in 0..n {
                paste(&mut out, &crop(sprite, 0, i * frame_h, w, frame_h)?, i * w, 0);
            }
            Some(out)
        }
        Transform::Extend { src_w, src_h, x, y, w, .. } => {
            let s = sprite.width() / w.max(1);
            crop(sprite, x.saturating_mul(s), y.saturating_mul(s), src_w.saturating_mul(s), src_h.saturating_mul(s))
        }
    }
}

/// Size of a sprite file in design units, which gives the HD scale of a pasted sprite.
fn sprite_design_size(row: &Row) -> (u32, u32) {
    match row.transform {
        Some(Transform::FlipFrameAxis(n)) => (row.boxed[2] / n, row.boxed[3] * n),
        Some(Transform::Extend { w, h, .. }) => (w, h),
        None => (row.boxed[2], row.boxed[3]),
    }
}

fn is_kept(input: &str) -> bool {
    input
        .strip_prefix("assets/minecraft/textures/gui/container/")
        .and_then(|n| n.strip_suffix(".png"))
        .is_some_and(|n| KEPT_SHEETS.contains(&n))
        || input.ends_with("/gui/book.png")
        || input.ends_with("/gui/recipe_book.png")
}

pub struct V1_20_2;

impl Step for V1_20_2 {
    fn name(&self) -> &'static str {
        "1.20.2 GUI sprites"
    }

    fn version(&self) -> Version {
        Version::new(1, 20, 2)
    }

    fn upgrade(&self, pack: &mut Pack, ctx: &mut Context) -> Result<()> {
        let mut renamer = Renamer::default();
        for (from, to) in REALMS_MOVES {
            move_with_mcmeta(pack, &mut renamer, from, to);
        }
        renamer.report(ctx);
        // The villager sheet shrinks to its panel; this must happen before its sprites are cut out
        // and the sheet is removed. A panel the pack already has is kept.
        if !pack.contains(VILLAGER_PANEL) {
            if let Some(img) = load_png(pack, ctx, VILLAGER_SHEET) {
                let panel = scaled_box(&VILLAGER_BOX, img.width(), img.height())
                    .and_then(|(x, y, w, h)| crop(&img, x, y, w, h));
                if let Some(panel) = panel {
                    save_png(pack, ctx, VILLAGER_PANEL, &panel);
                }
            }
        }
        for sheet in sheets() {
            slice_sheet(pack, ctx, sheet);
        }
        Ok(())
    }

    fn downgrade(&self, pack: &mut Pack, ctx: &mut Context) -> Result<()> {
        let mut renamer = Renamer::default();
        for (to, from) in REALMS_MOVES {
            move_with_mcmeta(pack, &mut renamer, from, to);
        }
        renamer.report(ctx);
        if !pack.contains(VILLAGER_SHEET) {
            if let Some(panel) = load_png(pack, ctx, VILLAGER_PANEL) {
                // Rebuild the 512x256 sheet around the panel so the sprites can be pasted onto it.
                let s = (panel.height() / VILLAGER_BOX[3]).max(1);
                let size = (VILLAGER_BOX[4].checked_mul(s), VILLAGER_BOX[5].checked_mul(s));
                if let Some(mut sheet) = canvas(ctx, VILLAGER_SHEET, size.0, size.1) {
                    paste(&mut sheet, &panel, 0, 0);
                    if save_png(pack, ctx, VILLAGER_SHEET, &sheet) {
                        pack.remove(VILLAGER_PANEL);
                    }
                }
            }
        }
        for sheet in sheets() {
            merge_sheet(pack, ctx, sheet);
        }
        Ok(())
    }
}

fn slice_sheet(pack: &mut Pack, ctx: &mut Context, sheet: &Sheet) {
    let Some(img) = load_png(pack, ctx, &sheet.input) else { return };
    let sheet_meta = pack.get(&format!("{}.mcmeta", sheet.input)).map(<[u8]>::to_vec);
    // The sheet is only deleted when every sprite was cut out or already existed.
    let mut complete = true;
    for row in &sheet.rows {
        // A pack that already ships the 1.20.2 sprite keeps it.
        if pack.contains(&row.output) {
            continue;
        }
        let cut = scaled_box(&row.boxed, img.width(), img.height()).and_then(|(x, y, w, h)| crop(&img, x, y, w, h));
        let Some(mut sprite) = cut else {
            complete = false;
            continue;
        };
        if let Some(transform) = row.transform {
            match apply(ctx, &row.output, transform, &sprite) {
                Some(transformed) => sprite = transformed,
                None => {
                    complete = false;
                    continue;
                }
            }
        }
        if !save_png(pack, ctx, &row.output, &sprite) {
            complete = false;
            continue;
        }
        if let Some(meta) = &sheet_meta {
            pack.insert(format!("{}.mcmeta", row.output), meta.clone());
        } else if row.has_metadata {
            write_json(pack, &format!("{}.mcmeta", row.output), &sprite_metadata(&row.output));
        }
    }
    if !complete {
        ctx.warn(format!(
            "{}: {}x{} does not match the expected layout, some GUI sprites were not cut and the sheet was kept",
            sheet.input,
            img.width(),
            img.height()
        ));
    } else if !is_kept(&sheet.input) {
        pack.remove(&sheet.input);
        pack.remove(&format!("{}.mcmeta", sheet.input));
    }
}

fn merge_sheet(pack: &mut Pack, ctx: &mut Context, sheet: &Sheet) {
    let design = sheet.rows[0].boxed;
    let max = max_scale(design[4], design[5]);
    let mut sprites = Vec::new();
    let mut budget = PixelBudget::new();
    for row in &sheet.rows {
        let Some(img) = load_png(pack, ctx, &row.output) else { continue };
        // A sprite so large that the sheet would exceed the size limit is an outlier: it stays.
        let scale = img.width() / sprite_design_size(row).0.max(1);
        if scale > max || !budget.take(&img) {
            ctx.warn(format!("{}: too large to merge into {}, left as it is", row.output, sheet.input));
            continue;
        }
        sprites.push((row, img));
    }
    if sprites.is_empty() {
        return;
    }
    // HD scale of the sheet: the largest sprite-to-design ratio, so no detail is lost.
    let scale = sprites
        .iter()
        .map(|(row, img)| img.width() / sprite_design_size(row).0.max(1))
        .max()
        .unwrap_or(1)
        .clamp(1, max);
    let (width, height) = (design[4] * scale, design[5] * scale);
    // Pixels the sprites do not cover (rest of the old sheet) are kept when the sheet exists.
    let mut target = match read_png(pack, &sheet.input) {
        Some(Ok(old)) => match resize(&old, width, height) {
            Some(old) => old,
            None => return,
        },
        Some(Err(e)) => {
            ctx.warn(format!("{e}; GUI sheet left as it is, its sprites were not merged"));
            return;
        }
        None => match canvas(ctx, &sheet.input, Some(width), Some(height)) {
            Some(target) => target,
            None => return,
        },
    };
    let mut merged = Vec::new();
    for (row, sprite) in &sprites {
        let sprite = match row.transform {
            Some(transform) => match unapply(ctx, &row.output, transform, sprite) {
                Some(restored) => restored,
                None => continue,
            },
            None => sprite.clone(),
        };
        let Some((x, y, w, h)) = scaled_box(&row.boxed, width, height) else { continue };
        let Some(sprite) = resize(&sprite, w, h) else { continue };
        paste(&mut target, &sprite, x, y);
        merged.push(*row);
    }
    if merged.is_empty() || !save_png(pack, ctx, &sheet.input, &target) {
        return;
    }
    for row in merged {
        pack.remove(&row.output);
        pack.remove(&format!("{}.mcmeta", row.output));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::steps::modern::common::testing::{png, put_png, solid};

    const WIDGETS: &str = "assets/minecraft/textures/gui/widgets.png";
    const BUTTON: &str = "assets/minecraft/textures/gui/sprites/widget/button.png";

    fn ctx() -> Context {
        Context::new(Version::new(1, 20, 1), Version::new(1, 20, 2))
    }

    #[test]
    fn all_data_is_understood() {
        assert!(sheets().len() > 50);
        for sheet in sheets() {
            for row in &sheet.rows {
                if row.has_metadata {
                    sprite_metadata(&row.output);
                }
            }
        }
    }

    fn widgets_pack(scale: u32) -> Pack {
        let mut pack = Pack::new();
        let mut sheet = solid(256 * scale, 256 * scale, [0, 0, 255, 255]);
        paste(&mut sheet, &solid(200 * scale, 20 * scale, [255, 0, 0, 255]), 0, 66 * scale);
        put_png(&mut pack, WIDGETS, &sheet);
        pack
    }

    #[test]
    fn slices_widgets_with_nine_slice_metadata() {
        let mut pack = widgets_pack(1);
        V1_20_2.upgrade(&mut pack, &mut ctx()).unwrap();
        assert!(!pack.contains(WIDGETS));
        let button = png(&pack, BUTTON);
        assert_eq!(button.dimensions(), (200, 20));
        assert_eq!(button.get_pixel(5, 5).0, [255, 0, 0, 255]);
        let meta = String::from_utf8(pack.get(&format!("{BUTTON}.mcmeta")).unwrap().to_vec()).unwrap();
        assert!(meta.contains("nine_slice") && meta.contains("\"left\": 20"));
    }

    #[test]
    fn scales_boxes_for_hd_sheets() {
        let mut pack = widgets_pack(4);
        V1_20_2.upgrade(&mut pack, &mut ctx()).unwrap();
        assert_eq!(png(&pack, BUTTON).dimensions(), (800, 80));
    }

    #[test]
    fn downgrade_merges_sprites_and_keeps_existing_sheet_pixels() {
        let mut pack = Pack::new();
        let sheet_path = "assets/minecraft/textures/gui/container/furnace.png";
        put_png(&mut pack, sheet_path, &solid(256, 256, [7, 7, 7, 255]));
        put_png(
            &mut pack,
            "assets/minecraft/textures/gui/sprites/container/furnace/lit_progress.png",
            &solid(14, 14, [255, 128, 0, 255]),
        );
        V1_20_2.downgrade(&mut pack, &mut ctx()).unwrap();
        let sheet = png(&pack, sheet_path);
        assert_eq!(sheet.dimensions(), (256, 256));
        assert_eq!(sheet.get_pixel(200, 200).0, [7, 7, 7, 255]);
        assert!(!pack.contains("assets/minecraft/textures/gui/sprites/container/furnace/lit_progress.png"));
    }

    #[test]
    fn roundtrip_restores_sprite_area() {
        let mut pack = widgets_pack(1);
        V1_20_2.upgrade(&mut pack, &mut ctx()).unwrap();
        V1_20_2.downgrade(&mut pack, &mut ctx()).unwrap();
        let sheet = png(&pack, WIDGETS);
        assert_eq!(sheet.dimensions(), (256, 256));
        assert_eq!(sheet.get_pixel(100, 70).0, [255, 0, 0, 255]);
        assert!(!pack.contains(BUTTON));
    }

    #[test]
    fn flips_animation_frames_and_back() {
        let mut pack = Pack::new();
        let input = "assets/realms/textures/gui/realms/expires_soon_icon.png";
        let output = "assets/minecraft/textures/gui/sprites/realm_status/expires_soon.png";
        let mut icon = solid(20, 28, [0, 255, 0, 255]);
        paste(&mut icon, &solid(10, 28, [255, 0, 0, 255]), 10, 0);
        put_png(&mut pack, input, &icon);
        V1_20_2.upgrade(&mut pack, &mut ctx()).unwrap();
        let strip = png(&pack, output);
        assert_eq!(strip.dimensions(), (10, 56));
        assert_eq!(strip.get_pixel(0, 0).0, [0, 255, 0, 255]);
        assert_eq!(strip.get_pixel(0, 30).0, [255, 0, 0, 255]);
        assert!(pack.contains(&format!("{output}.mcmeta")));
        V1_20_2.downgrade(&mut pack, &mut ctx()).unwrap();
        assert_eq!(png(&pack, input), icon);
    }

    #[test]
    fn villager_sheet_is_clipped_and_restored() {
        let mut pack = Pack::new();
        put_png(&mut pack, VILLAGER_SHEET, &solid(512, 256, [1, 2, 3, 255]));
        V1_20_2.upgrade(&mut pack, &mut ctx()).unwrap();
        assert_eq!(png(&pack, VILLAGER_PANEL).dimensions(), (276, 166));
        assert!(!pack.contains(VILLAGER_SHEET));
        V1_20_2.downgrade(&mut pack, &mut ctx()).unwrap();
        assert_eq!(png(&pack, VILLAGER_SHEET).dimensions(), (512, 256));
    }

    #[test]
    fn degenerate_sheets_do_not_abort_the_conversion() {
        let mut pack = Pack::new();
        // Half-resolution loom: the error sprite is smaller than the sprite extender's design size.
        let loom = "assets/minecraft/textures/gui/container/loom.png";
        let error = "assets/minecraft/textures/gui/sprites/container/loom/error.png";
        put_png(&mut pack, loom, &solid(128, 128, [1, 2, 3, 255]));
        // A one pixel wide animation cannot be split into frames.
        let expires = "assets/realms/textures/gui/realms/expires_soon_icon.png";
        put_png(&mut pack, expires, &solid(1, 1, [1, 2, 3, 255]));
        let mut ctx = ctx();
        V1_20_2.upgrade(&mut pack, &mut ctx).unwrap();
        assert!(pack.contains(loom));
        assert!(!pack.contains(error));
        assert!(!ctx.into_warnings().is_empty());
    }

    #[test]
    fn oversized_images_are_left_alone_both_ways() {
        let mut pack = Pack::new();
        let paths = [WIDGETS, VILLAGER_SHEET, VILLAGER_PANEL, "assets/minecraft/textures/gui/sprites/widget/button.png"];
        for path in paths {
            pack.insert(path, crate::steps::support::oversized_png());
        }
        let before = pack.clone();
        let mut ctx = ctx();
        V1_20_2.upgrade(&mut pack, &mut ctx).unwrap();
        V1_20_2.downgrade(&mut pack, &mut ctx).unwrap();
        for path in paths {
            assert_eq!(pack.get(path), before.get(path), "{path}");
        }
    }

    #[test]
    fn scaled_box_does_not_overflow_on_wide_images() {
        assert!(scaled_box(&VILLAGER_BOX, u32::MAX, 1).is_some());
        assert!(scaled_box(&[0, 0, 512, 256, 1, 1], u32::MAX, 1).is_none());
    }

    #[test]
    fn oversized_sprite_is_not_merged() {
        let mut pack = Pack::new();
        let sprite = "assets/minecraft/textures/gui/sprites/container/furnace/lit_progress.png";
        put_png(&mut pack, sprite, &solid(14 * 40, 1, [255, 128, 0, 255]));
        let mut ctx = ctx();
        V1_20_2.downgrade(&mut pack, &mut ctx).unwrap();
        assert!(pack.contains(sprite));
        assert!(!pack.contains("assets/minecraft/textures/gui/container/furnace.png"));
        assert_eq!(ctx.into_warnings().len(), 1);
    }
}

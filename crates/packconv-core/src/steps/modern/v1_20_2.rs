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
use crate::util::{read_png, write_json, write_png};
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

/// Box in pixels of an image of `width x height`, using the slicer's integer scaling.
fn scaled_box(b: &[u32; 6], width: u32, height: u32) -> (u32, u32, u32, u32) {
    (b[0] * width / b[4], b[1] * height / b[5], b[2] * width / b[4], b[3] * height / b[5])
}

fn apply(transform: Transform, sprite: RgbaImage) -> RgbaImage {
    match transform {
        Transform::FlipFrameAxis(n) => {
            let (frame_w, h) = (sprite.width() / n, sprite.height());
            let mut out = RgbaImage::new(frame_w, h * n);
            for i in 0..n {
                if let Some(frame) = crop(&sprite, i * frame_w, 0, frame_w, h) {
                    paste(&mut out, &frame, 0, i * h);
                }
            }
            out
        }
        Transform::Extend { src_w, x, y, w, h, .. } => {
            let s = sprite.width() / src_w.max(1);
            let mut out = RgbaImage::new(w * s, h * s);
            paste(&mut out, &sprite, x * s, y * s);
            out
        }
    }
}

fn unapply(transform: Transform, sprite: &RgbaImage) -> Option<RgbaImage> {
    match transform {
        Transform::FlipFrameAxis(n) => {
            let (w, frame_h) = (sprite.width(), sprite.height() / n);
            let mut out = RgbaImage::new(w * n, frame_h);
            for i in 0..n {
                paste(&mut out, &crop(sprite, 0, i * frame_h, w, frame_h)?, i * w, 0);
            }
            Some(out)
        }
        Transform::Extend { src_w, src_h, x, y, w, .. } => {
            let s = sprite.width() / w.max(1);
            crop(sprite, x * s, y * s, src_w * s, src_h * s)
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
        for (from, to) in REALMS_MOVES {
            move_with_mcmeta(pack, from, to);
        }
        // The villager sheet shrinks to its panel; this must happen before its sprites are cut out
        // and the sheet is removed.
        if let Some(sheet) = read_png(pack, VILLAGER_SHEET) {
            match sheet {
                Ok(img) => {
                    let (x, y, w, h) = scaled_box(&VILLAGER_BOX, img.width(), img.height());
                    if let Some(panel) = crop(&img, x, y, w, h) {
                        write_png(pack, VILLAGER_PANEL, &panel)?;
                    }
                }
                Err(e) => ctx.warn(format!("{e}; villager panel not converted")),
            }
        }
        for sheet in sheets() {
            slice_sheet(pack, ctx, sheet)?;
        }
        Ok(())
    }

    fn downgrade(&self, pack: &mut Pack, ctx: &mut Context) -> Result<()> {
        for (to, from) in REALMS_MOVES {
            move_with_mcmeta(pack, from, to);
        }
        if let Some(panel) = read_png(pack, VILLAGER_PANEL) {
            match panel {
                Ok(panel) => {
                    // Rebuild the 512x256 sheet around the panel so the sprites can be pasted onto it.
                    let s = (panel.height() / VILLAGER_BOX[3]).max(1);
                    let mut sheet = RgbaImage::new(VILLAGER_BOX[4] * s, VILLAGER_BOX[5] * s);
                    paste(&mut sheet, &panel, 0, 0);
                    write_png(pack, VILLAGER_SHEET, &sheet)?;
                    pack.remove(VILLAGER_PANEL);
                }
                Err(e) => ctx.warn(format!("{e}; villager panel not converted")),
            }
        }
        for sheet in sheets() {
            merge_sheet(pack, ctx, sheet)?;
        }
        Ok(())
    }
}

fn slice_sheet(pack: &mut Pack, ctx: &mut Context, sheet: &Sheet) -> Result<()> {
    let img = match read_png(pack, &sheet.input) {
        None => return Ok(()),
        Some(Ok(img)) => img,
        Some(Err(e)) => {
            ctx.warn(format!("{e}; GUI sheet not sliced"));
            return Ok(());
        }
    };
    let sheet_meta = pack.get(&format!("{}.mcmeta", sheet.input)).map(<[u8]>::to_vec);
    for row in &sheet.rows {
        // A pack that already ships the 1.20.2 sprite keeps it.
        if pack.contains(&row.output) {
            continue;
        }
        let (x, y, w, h) = scaled_box(&row.boxed, img.width(), img.height());
        let Some(mut sprite) = crop(&img, x, y, w, h) else { continue };
        if let Some(transform) = row.transform {
            sprite = apply(transform, sprite);
        }
        write_png(pack, &row.output, &sprite)?;
        if let Some(meta) = &sheet_meta {
            pack.insert(format!("{}.mcmeta", row.output), meta.clone());
        } else if row.has_metadata {
            write_json(pack, &format!("{}.mcmeta", row.output), &sprite_metadata(&row.output));
        }
    }
    if !is_kept(&sheet.input) {
        pack.remove(&sheet.input);
        pack.remove(&format!("{}.mcmeta", sheet.input));
    }
    Ok(())
}

fn merge_sheet(pack: &mut Pack, ctx: &mut Context, sheet: &Sheet) -> Result<()> {
    let mut sprites = Vec::new();
    for row in &sheet.rows {
        match read_png(pack, &row.output) {
            None => {}
            Some(Ok(img)) => sprites.push((row, img)),
            Some(Err(e)) => ctx.warn(format!("{e}; GUI sprite not merged")),
        }
    }
    if sprites.is_empty() {
        return Ok(());
    }
    // HD scale of the sheet: the largest sprite-to-design ratio, so no detail is lost.
    let scale = sprites
        .iter()
        .map(|(row, img)| img.width() / sprite_design_size(row).0.max(1))
        .max()
        .unwrap_or(1)
        .max(1);
    let design = sheet.rows[0].boxed;
    let (width, height) = (design[4] * scale, design[5] * scale);
    // Pixels the sprites do not cover (rest of the old sheet) are kept when the sheet exists.
    let mut canvas = match read_png(pack, &sheet.input) {
        Some(Ok(old)) => resize(&old, width, height),
        Some(Err(e)) => {
            ctx.warn(format!("{e}; GUI sheet rebuilt from sprites only"));
            RgbaImage::new(width, height)
        }
        None => RgbaImage::new(width, height),
    };
    for (row, sprite) in &sprites {
        let sprite = match row.transform {
            Some(transform) => match unapply(transform, sprite) {
                Some(restored) => restored,
                None => continue,
            },
            None => sprite.clone(),
        };
        let (x, y, w, h) = scaled_box(&row.boxed, width, height);
        paste(&mut canvas, &resize(&sprite, w, h), x, y);
    }
    write_png(pack, &sheet.input, &canvas)?;
    for (row, _) in &sprites {
        pack.remove(&row.output);
        pack.remove(&format!("{}.mcmeta", row.output));
    }
    Ok(())
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
}

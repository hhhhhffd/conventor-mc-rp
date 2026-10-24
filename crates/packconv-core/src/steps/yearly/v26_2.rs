//! 26.2: beds, signs and hanging signs became ordinary block models. Their entity texture
//! sheets were cut into per-face block textures exactly as Mojang's `slicer` 26.2 does (see
//! `data/yearly/slice_26_2.json`), the `beds`/`signs` atlases disappeared, and the quartz and
//! purpur pillar textures got a `_side` suffix.

use std::collections::BTreeSet;
use std::sync::OnceLock;

use image::imageops::{self, FilterType};
use image::RgbaImage;
use serde::Deserialize;

use super::common::{asset_files, directed, edit_json_files, move_files, uses_item_model_type, Pairs, RefMap, MINECRAFT};
use crate::error::Result;
use crate::step::{Context, Step};
use crate::util::{namespaces, read_png, write_png};
use crate::version::Version;
use crate::vfs::Pack;

const PILLAR_FILES: &Pairs = &[
    ("textures/block/quartz_pillar.png", "textures/block/quartz_pillar_side.png"),
    ("textures/block/purpur_pillar.png", "textures/block/purpur_pillar_side.png"),
];
/// Only texture references move: the models and blockstates keep their ids.
const PILLAR_TEXTURES: &Pairs = &[("block/quartz_pillar", "block/quartz_pillar_side"), ("block/purpur_pillar", "block/purpur_pillar_side")];

const REMOVED_ATLASES: [&str; 2] = ["beds", "signs"];
const REMOVED_SPECIAL_MODELS: [&str; 6] = [
    "minecraft:bed",
    "minecraft:standing_sign",
    "minecraft:hanging_sign",
    "bed",
    "standing_sign",
    "hanging_sign",
];

#[derive(Deserialize)]
struct Slicing {
    groups: Vec<Group>,
}

/// Entity sheets that share a layout (one per bed color or wood type).
#[derive(Deserialize)]
struct Group {
    /// Sheet path with `{name}`, relative to `assets/minecraft/`.
    input: String,
    /// Grid the part coordinates refer to; real sheets are scaled by `size / reference`.
    #[serde(rename = "ref")]
    reference: [u32; 2],
    names: Vec<String>,
    outputs: Vec<Output>,
    /// Name of the one sheet that the `shared_outputs` (color independent faces) are cut from.
    shared_source: Option<String>,
    #[serde(default)]
    shared_outputs: Vec<Output>,
}

#[derive(Deserialize)]
struct Output {
    path: String,
    /// Size of the output at `reference` scale.
    canvas: [u32; 2],
    parts: Vec<Part>,
}

#[derive(Deserialize)]
struct Part {
    /// `[x, y, width, height]` in the sheet.
    src: [u32; 4],
    /// `[x, y]` in the output, applied after `transforms`.
    dst: [u32; 2],
    transforms: Vec<Transform>,
}

#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "camelCase")]
enum Transform {
    /// Clockwise quarter turns.
    Rotate90,
    Rotate180,
    Rotate270,
    MirrorVertically,
}

impl Transform {
    fn apply(self, image: &RgbaImage) -> RgbaImage {
        match self {
            Transform::Rotate90 => imageops::rotate90(image),
            Transform::Rotate180 => imageops::rotate180(image),
            Transform::Rotate270 => imageops::rotate270(image),
            Transform::MirrorVertically => imageops::flip_vertical(image),
        }
    }

    fn inverse(self) -> Self {
        match self {
            Transform::Rotate90 => Transform::Rotate270,
            Transform::Rotate270 => Transform::Rotate90,
            other => other,
        }
    }

    fn swaps_axes(self) -> bool {
        matches!(self, Transform::Rotate90 | Transform::Rotate270)
    }
}

fn slicing() -> &'static [Group] {
    static DATA: OnceLock<Slicing> = OnceLock::new();
    &DATA
        .get_or_init(|| serde_json::from_str(include_str!("../../../data/yearly/slice_26_2.json")).expect("embedded slice_26_2.json is valid"))
        .groups
}

pub struct SlicedBlockEntities;

fn full(path: &str) -> String {
    format!("{MINECRAFT}{path}")
}

/// Cuts one output out of a sheet using Mojang's integer arithmetic (`value * size / reference`).
/// Returns `None` when the sheet is too small for the layout.
fn slice(sheet: &RgbaImage, reference: [u32; 2], output: &Output) -> Option<RgbaImage> {
    let (width, height) = sheet.dimensions();
    let scale_x = |v: u32| v * width / reference[0];
    let scale_y = |v: u32| v * height / reference[1];
    let mut canvas = RgbaImage::new(scale_x(output.canvas[0]), scale_y(output.canvas[1]));
    if canvas.width() == 0 || canvas.height() == 0 {
        return None;
    }
    for part in &output.parts {
        let [x, y, w, h] = part.src;
        let (x, y, w, h) = (scale_x(x), scale_y(y), scale_x(w), scale_y(h));
        if w == 0 || h == 0 || x + w > width || y + h > height {
            return None;
        }
        let mut piece = imageops::crop_imm(sheet, x, y, w, h).to_image();
        for transform in &part.transforms {
            piece = transform.apply(&piece);
        }
        imageops::replace(&mut canvas, &piece, i64::from(scale_x(part.dst[0])), i64::from(scale_y(part.dst[1])));
    }
    Some(canvas)
}

fn slice_sheets(pack: &mut Pack, ctx: &mut Context, group: &Group) -> Result<()> {
    for name in &group.names {
        let input = full(&group.input.replace("{name}", name));
        let sheet = match read_png(pack, &input) {
            None => continue,
            Some(Err(e)) => {
                ctx.warn(format!("{e}; sheet not converted"));
                continue;
            }
            Some(Ok(sheet)) => sheet,
        };
        let shared: &[Output] = if group.shared_source.as_deref() == Some(name) { &group.shared_outputs } else { &[] };
        let outputs: Vec<&Output> = group.outputs.iter().chain(shared).collect();
        let Some(images) = outputs.iter().map(|o| slice(&sheet, group.reference, o)).collect::<Option<Vec<_>>>() else {
            ctx.warn(format!("{input} does not match the vanilla sheet layout; not converted"));
            continue;
        };
        // Mojang's slicer copies the sheet's .mcmeta next to every slice.
        let meta = pack.get(&format!("{input}.mcmeta")).map(<[u8]>::to_vec);
        for (output, image) in outputs.iter().zip(&images) {
            let path = full(&output.path.replace("{name}", name));
            write_png(pack, &path, image)?;
            if let Some(meta) = &meta {
                pack.insert(format!("{path}.mcmeta"), meta.clone());
            }
        }
        pack.remove(&input);
        pack.remove(&format!("{input}.mcmeta"));
    }
    Ok(())
}

/// Scale factor of an output image relative to its `canvas`, if it is an exact multiple.
fn exact_scale(image: &RgbaImage, canvas: [u32; 2]) -> Option<u32> {
    let scale = image.width() / canvas[0];
    (scale > 0 && image.width() == canvas[0] * scale && image.height() == canvas[1] * scale).then_some(scale)
}

struct Slice<'a> {
    output: &'a Output,
    image: RgbaImage,
    path: String,
    scale: u32,
    shared: bool,
}

/// Pastes slices back into a sheet: the inverse of [`slice`] for the regions the slices cover.
fn paste_slices(reference: [u32; 2], slices: &[Slice]) -> RgbaImage {
    let scale = slices.iter().map(|s| s.scale).max().unwrap_or(1);
    let mut sheet = RgbaImage::new(reference[0] * scale, reference[1] * scale);
    for slice in slices {
        let image = if slice.scale == scale {
            slice.image.clone()
        } else {
            let [w, h] = slice.output.canvas;
            imageops::resize(&slice.image, w * scale, h * scale, FilterType::Nearest)
        };
        for part in &slice.output.parts {
            let [x, y, w, h] = part.src;
            let (w, h) = if part.transforms.iter().filter(|t| t.swaps_axes()).count() % 2 == 1 { (h, w) } else { (w, h) };
            let mut piece = imageops::crop_imm(&image, part.dst[0] * scale, part.dst[1] * scale, w * scale, h * scale).to_image();
            for transform in part.transforms.iter().rev() {
                piece = transform.inverse().apply(&piece);
            }
            imageops::replace(&mut sheet, &piece, i64::from(x * scale), i64::from(y * scale));
        }
    }
    sheet
}

/// Rebuilds entity sheets from sliced textures. Returns the number of sheets built.
fn join_sheets(pack: &mut Pack, ctx: &mut Context, group: &Group) -> Result<usize> {
    let mut consumed = BTreeSet::new();
    let mut built = 0;
    for name in &group.names {
        let mut slices = Vec::new();
        // Shared slices go first so the sheet's own slices win where they overlap.
        let shared: &[Output] = if group.shared_source.is_some() { &group.shared_outputs } else { &[] };
        for (output, is_shared) in shared.iter().map(|o| (o, true)).chain(group.outputs.iter().map(|o| (o, false))) {
            let path = full(&output.path.replace("{name}", name));
            match read_png(pack, &path) {
                None => {}
                Some(Err(e)) => ctx.warn(format!("{e}; slice ignored")),
                Some(Ok(image)) => match exact_scale(&image, output.canvas) {
                    Some(scale) => slices.push(Slice { output, image, path, scale, shared: is_shared }),
                    None => ctx.warn(format!("{path} has an unexpected size for its 26.2 layout; ignored")),
                },
            }
        }
        if slices.iter().all(|s| s.shared) {
            continue;
        }
        let input = full(&group.input.replace("{name}", name));
        write_png(pack, &input, &paste_slices(group.reference, &slices))?;
        if let Some(meta) = slices.iter().find_map(|s| pack.get(&format!("{}.mcmeta", s.path))).map(<[u8]>::to_vec) {
            pack.insert(format!("{input}.mcmeta"), meta);
        }
        consumed.extend(slices.into_iter().map(|s| s.path));
        built += 1;
    }
    for path in consumed {
        pack.remove(&path);
        pack.remove(&format!("{path}.mcmeta"));
    }
    Ok(built)
}

fn has_core_shader(pack: &Pack, prefixes: &[&str]) -> bool {
    pack.paths().any(|p| {
        p.strip_prefix("assets/")
            .and_then(|rest| rest.split_once("/shaders/core/"))
            .is_some_and(|(_, file)| prefixes.iter().any(|prefix| file.starts_with(prefix)))
    })
}

fn rename_pillars(pack: &mut Pack, ctx: &mut Context, up: bool) {
    move_files(pack, &directed(PILLAR_FILES, up));
    let refs = RefMap::new(&directed(PILLAR_TEXTURES, up));
    let models = asset_files(pack, "models", &[".json"]);
    edit_json_files(pack, ctx, models, |json| refs.rewrite_model_textures(json));
}

impl Step for SlicedBlockEntities {
    fn name(&self) -> &'static str {
        "bed/sign block textures and pillar suffix"
    }

    fn version(&self) -> Version {
        Version::new(26, 2, 0)
    }

    fn upgrade(&self, pack: &mut Pack, ctx: &mut Context) -> Result<()> {
        for group in slicing() {
            slice_sheets(pack, ctx, group)?;
        }
        rename_pillars(pack, ctx, true);

        let mut removed_atlas = false;
        for namespace in namespaces(pack) {
            for atlas in REMOVED_ATLASES {
                removed_atlas |= pack.remove(&format!("assets/{namespace}/atlases/{atlas}.json")).is_some();
            }
        }
        if removed_atlas {
            ctx.warn("the beds and signs texture atlases no longer exist in 26.2 (their textures are in the blocks atlas); custom atlas sources were dropped");
        }
        if uses_item_model_type(pack, &REMOVED_SPECIAL_MODELS) {
            ctx.warn("item definitions use the bed/standing_sign/hanging_sign special models, which 26.2 removed; use block models instead");
        }
        if has_core_shader(pack, &["rendertype_text"]) {
            ctx.warn("26.2 replaced the rendertype_text* core shaders with core/text and core/text_background; custom text shaders must be ported by hand");
        }
        Ok(())
    }

    fn downgrade(&self, pack: &mut Pack, ctx: &mut Context) -> Result<()> {
        let mut built = 0;
        for group in slicing() {
            built += join_sheets(pack, ctx, group)?;
        }
        if built > 0 {
            ctx.warn(format!(
                "rebuilt {built} bed/sign entity sheet(s) from the 26.2 block textures; areas of the sheets that 26.2 no longer uses are transparent and color independent bed faces come from the shared textures"
            ));
        }
        rename_pillars(pack, ctx, false);
        if has_core_shader(pack, &["text.", "text_background."]) {
            ctx.warn("26.2 merged the text core shaders into core/text; custom text shaders must be ported back to rendertype_text* by hand");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::steps::yearly::common::testing::*;
    use image::Rgba;

    /// Every pixel encodes its own position, so any misplaced pixel is detected.
    fn sheet(width: u32, height: u32, seed: u8) -> RgbaImage {
        RgbaImage::from_fn(width, height, |x, y| Rgba([x as u8, y as u8, seed, 255]))
    }

    fn group(input_prefix: &str) -> &'static Group {
        slicing().iter().find(|g| g.input.starts_with(input_prefix)).unwrap()
    }

    /// Asserts that every sheet pixel covered by `outputs` is equal in both images.
    fn assert_covered_equal(a: &RgbaImage, b: &RgbaImage, reference: [u32; 2], outputs: &[&Output]) {
        let scale = a.width() / reference[0];
        assert_eq!(a.dimensions(), b.dimensions());
        for output in outputs {
            for part in &output.parts {
                for y in part.src[1] * scale..(part.src[1] + part.src[3]) * scale {
                    for x in part.src[0] * scale..(part.src[0] + part.src[2]) * scale {
                        assert_eq!(a.get_pixel(x, y), b.get_pixel(x, y), "pixel {x},{y}");
                    }
                }
            }
        }
    }

    #[test]
    fn embedded_data_is_complete() {
        let groups = slicing();
        assert_eq!(groups.len(), 3);
        assert_eq!(groups[0].names.len(), 16);
        assert_eq!(groups[1].names.len(), 12);
        assert_eq!(groups[2].names.len(), 12);
    }

    #[test]
    fn bed_is_sliced_like_the_mojang_slicer() {
        let mut pack = Pack::new();
        let red = sheet(64, 64, 1);
        insert_png(&mut pack, "assets/minecraft/textures/entity/bed/red.png", &red);
        pack.insert("assets/minecraft/textures/entity/bed/red.png.mcmeta", b"{}".to_vec());
        let mut ctx = context();
        SlicedBlockEntities.upgrade(&mut pack, &mut ctx).unwrap();

        assert!(!pack.contains("assets/minecraft/textures/entity/bed/red.png"));
        assert!(pack.contains("assets/minecraft/textures/block/red_bed_foot_east.png.mcmeta"));
        let head_up = png(&pack, "assets/minecraft/textures/block/red_bed_head_up.png");
        assert_eq!(head_up.dimensions(), (16, 16));
        assert_eq!(head_up.get_pixel(0, 0), red.get_pixel(6, 6));
        // foot_south: src (22,22,16,6) flipped top to bottom lands at (0,7).
        let foot_south = png(&pack, "assets/minecraft/textures/block/red_bed_foot_south.png");
        assert_eq!(foot_south.get_pixel(0, 7), red.get_pixel(22, 27));
        assert_eq!(foot_south.get_pixel(15, 12), red.get_pixel(37, 22));
        // Clockwise quarter turn: foot_east src (22,28,6,16) -> output (x, 7 + y) takes src (22 + y, 43 - x).
        let foot_east = png(&pack, "assets/minecraft/textures/block/red_bed_foot_east.png");
        assert_eq!(foot_east.get_pixel(0, 7), red.get_pixel(22, 43));
        assert_eq!(foot_east.get_pixel(15, 7), red.get_pixel(22, 28));
        // The color independent faces come from the black sheet only.
        assert!(!pack.contains("assets/minecraft/textures/block/bed_down.png"));
        assert!(ctx.into_warnings().is_empty());
    }

    #[test]
    fn bed_roundtrip_restores_covered_pixels_and_uses_shared_faces() {
        let mut pack = Pack::new();
        let black = sheet(64, 64, 0);
        let red = sheet(64, 64, 1);
        insert_png(&mut pack, "assets/minecraft/textures/entity/bed/black.png", &black);
        insert_png(&mut pack, "assets/minecraft/textures/entity/bed/red.png", &red);
        let mut ctx = context();
        SlicedBlockEntities.upgrade(&mut pack, &mut ctx).unwrap();
        assert!(pack.contains("assets/minecraft/textures/block/bed_down.png"));
        assert!(pack.contains("assets/minecraft/textures/block/bed_head_north.png"));

        SlicedBlockEntities.downgrade(&mut pack, &mut ctx).unwrap();
        let bed = group("textures/entity/bed");
        let own: Vec<&Output> = bed.outputs.iter().collect();
        let all: Vec<&Output> = bed.outputs.iter().chain(&bed.shared_outputs).collect();
        let black_out = png(&pack, "assets/minecraft/textures/entity/bed/black.png");
        assert_covered_equal(&black, &black_out, bed.reference, &all);
        let red_out = png(&pack, "assets/minecraft/textures/entity/bed/red.png");
        assert_covered_equal(&red, &red_out, bed.reference, &own);
        // The shared underside is black's, also in the red sheet.
        assert_eq!(red_out.get_pixel(30, 10), black.get_pixel(30, 10));
        assert!(pack.paths().all(|p| !p.contains("textures/block/")));
        assert_eq!(ctx.into_warnings().len(), 1);
    }

    #[test]
    fn hd_signs_roundtrip() {
        let mut pack = Pack::new();
        let standing = sheet(128, 64, 2);
        let hanging = sheet(128, 64, 3);
        insert_png(&mut pack, "assets/minecraft/textures/entity/signs/oak.png", &standing);
        insert_png(&mut pack, "assets/minecraft/textures/entity/signs/hanging/oak.png", &hanging);
        let mut ctx = context();
        SlicedBlockEntities.upgrade(&mut pack, &mut ctx).unwrap();
        assert_eq!(png(&pack, "assets/minecraft/textures/block/oak_sign.png").dimensions(), (64, 64));
        assert_eq!(png(&pack, "assets/minecraft/textures/gui/signs/oak.png").dimensions(), (48, 52));
        assert_eq!(png(&pack, "assets/minecraft/textures/block/oak_hanging_sign.png").dimensions(), (64, 64));

        SlicedBlockEntities.downgrade(&mut pack, &mut ctx).unwrap();
        let signs = group("textures/entity/signs/{name}");
        let hanging_group = group("textures/entity/signs/hanging");
        assert_covered_equal(
            &standing,
            &png(&pack, "assets/minecraft/textures/entity/signs/oak.png"),
            signs.reference,
            &signs.outputs.iter().collect::<Vec<_>>(),
        );
        assert_covered_equal(
            &hanging,
            &png(&pack, "assets/minecraft/textures/entity/signs/hanging/oak.png"),
            hanging_group.reference,
            &hanging_group.outputs.iter().collect::<Vec<_>>(),
        );
        assert!(!pack.contains("assets/minecraft/textures/gui/signs/oak.png"));
        assert!(!pack.contains("assets/minecraft/textures/block/oak_sign.png"));
    }

    #[test]
    fn wrong_sized_or_broken_sheets_are_kept_with_a_warning() {
        let mut pack = Pack::new();
        // Too small for the 3x3 pixel boxes of the 64x64 layout to be non-empty.
        insert_png(&mut pack, "assets/minecraft/textures/entity/bed/blue.png", &sheet(8, 8, 1));
        pack.insert("assets/minecraft/textures/entity/bed/lime.png", b"not a png".to_vec());
        let mut ctx = context();
        SlicedBlockEntities.upgrade(&mut pack, &mut ctx).unwrap();
        assert!(pack.contains("assets/minecraft/textures/entity/bed/blue.png"));
        assert!(pack.contains("assets/minecraft/textures/entity/bed/lime.png"));
        assert_eq!(ctx.into_warnings().len(), 2);
    }

    #[test]
    fn pillar_textures_move_but_model_ids_stay() {
        let mut pack = Pack::new();
        pack.insert("assets/minecraft/textures/block/quartz_pillar.png", vec![1]);
        pack.insert("assets/minecraft/textures/block/purpur_pillar.png.mcmeta", vec![2]);
        pack.insert("assets/minecraft/textures/block/purpur_pillar.png", vec![3]);
        pack.insert(
            "assets/minecraft/models/block/quartz_pillar.json",
            br#"{"parent":"minecraft:block/column","textures":{"side":"minecraft:block/quartz_pillar","end":"block/quartz_pillar_top"}}"#.to_vec(),
        );
        pack.insert("assets/minecraft/blockstates/quartz_pillar.json", br#"{"variants":{"":{"model":"minecraft:block/quartz_pillar"}}}"#.to_vec());
        let mut ctx = context();
        SlicedBlockEntities.upgrade(&mut pack, &mut ctx).unwrap();
        assert!(pack.contains("assets/minecraft/textures/block/quartz_pillar_side.png"));
        assert!(pack.contains("assets/minecraft/textures/block/purpur_pillar_side.png.mcmeta"));
        let model = String::from_utf8_lossy(pack.get("assets/minecraft/models/block/quartz_pillar.json").unwrap()).into_owned();
        assert!(model.contains("minecraft:block/quartz_pillar_side") && model.contains("block/quartz_pillar_top"));
        let state = String::from_utf8_lossy(pack.get("assets/minecraft/blockstates/quartz_pillar.json").unwrap()).into_owned();
        assert!(state.contains("\"minecraft:block/quartz_pillar\""));

        SlicedBlockEntities.downgrade(&mut pack, &mut ctx).unwrap();
        assert!(pack.contains("assets/minecraft/textures/block/quartz_pillar.png"));
        let model = String::from_utf8_lossy(pack.get("assets/minecraft/models/block/quartz_pillar.json").unwrap()).into_owned();
        assert!(model.contains("\"minecraft:block/quartz_pillar\"") && model.contains("block/quartz_pillar_top"));
    }

    #[test]
    fn removed_atlases_and_shader_and_special_models_warn() {
        let mut pack = Pack::new();
        pack.insert("assets/minecraft/atlases/beds.json", b"{}".to_vec());
        pack.insert("assets/minecraft/atlases/signs.json", b"{}".to_vec());
        pack.insert("assets/minecraft/atlases/blocks.json", b"{}".to_vec());
        pack.insert("assets/minecraft/shaders/core/rendertype_text.vsh", vec![]);
        pack.insert("assets/minecraft/items/red_bed.json", br#"{"model":{"type":"minecraft:special","model":{"type":"minecraft:bed"}}}"#.to_vec());
        let mut ctx = context();
        SlicedBlockEntities.upgrade(&mut pack, &mut ctx).unwrap();
        assert!(!pack.contains("assets/minecraft/atlases/beds.json") && !pack.contains("assets/minecraft/atlases/signs.json"));
        assert!(pack.contains("assets/minecraft/atlases/blocks.json"));
        assert_eq!(ctx.into_warnings().len(), 3);
    }
}

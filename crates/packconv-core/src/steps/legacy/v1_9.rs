//! 1.9 changes:
//! - the compass and clock animations became one texture per frame;
//! - item models got per-hand display transforms (`display`) and shared parents
//!   (`item/generated`, `item/handheld`, `item/handheld_rod`);
//! - blockstates gained `multipart`;
//! - the off-hand slot was added to `widgets.png`;
//! - the single boat texture became one per wood type.

use std::sync::OnceLock;

use image::imageops;
use serde_json::{json, Map, Value};

use crate::error::Result;
use crate::step::{Context, Step};
use crate::steps::support::{canvas, load_png, save_png};
use crate::util::read_json;
use crate::version::Version;
use crate::vfs::Pack;

use super::common::{
    drop_added_files, json_paths, move_textures, retarget_texture_refs, reversed, update_json, MC,
};

/// Animated item textures that were a vertical strip of square frames: `(name, frame count)`.
const ANIMATION_STRIPS: [(&str, u32); 2] = [("compass", 32), ("clock", 64)];

/// Only the oak boat has a pre-1.9 counterpart; the other wood types are new.
const BOAT_RENAMES: [(&str, &str); 2] = [
    ("items/boat", "items/oak_boat"),
    ("entity/boat", "entity/boat/boat_oak"),
];

/// Item parents that did not exist in 1.8, where models used `builtin/generated` directly.
const ITEM_PARENTS: [&str; 3] = ["item/generated", "item/handheld", "item/handheld_rod"];

/// A rotation, translation or scale triple.
type Vec3 = [f64; 3];

/// Rewrites one pose (`firstperson` or `thirdperson`) of a model's `display`.
type PoseConversion = fn(&mut Map<String, Value>);

pub struct V1_9;

impl Step for V1_9 {
    fn name(&self) -> &'static str {
        "legacy-1.9"
    }

    fn version(&self) -> Version {
        Version::new(1, 9, 0)
    }

    fn upgrade(&self, pack: &mut Pack, ctx: &mut Context) -> Result<()> {
        for (name, frames) in ANIMATION_STRIPS {
            split_animation_strip(pack, ctx, name, frames);
        }
        drop_dial_models(pack);
        add_off_hand_slot(pack, ctx);
        move_textures(pack, ctx, &BOAT_RENAMES);
        for path in json_paths(pack, "models") {
            update_json(pack, ctx, &path, |model| {
                let retargeted = retarget_texture_refs(model, &BOAT_RENAMES);
                upgrade_model(model) | retargeted
            });
        }
        Ok(())
    }

    fn downgrade(&self, pack: &mut Pack, ctx: &mut Context) -> Result<()> {
        for (name, frames) in ANIMATION_STRIPS {
            join_animation_frames(pack, ctx, name, frames);
        }
        drop_overridden_dial_models(pack);
        drop_multipart_blockstates(pack, ctx);
        let boats_back = reversed(&BOAT_RENAMES);
        move_textures(pack, ctx, &boats_back);
        for path in json_paths(pack, "models") {
            update_json(pack, ctx, &path, |model| {
                let retargeted = retarget_texture_refs(model, &boats_back);
                downgrade_model(model) | retargeted
            });
        }
        drop_added_files(pack, ctx, "1.9");
        Ok(())
    }
}

fn animation_strip_path(name: &str) -> String {
    format!("{MC}/textures/items/{name}.png")
}

fn animation_frame_path(name: &str, index: u32) -> String {
    format!("{MC}/textures/items/{name}_{index:02}.png")
}

/// `compass.png` (16x512, 32 frames) -> `compass_00.png` .. `compass_31.png`. The frames are
/// square, so the frame size is the width, which also covers HD packs. An unreadable or
/// oversized strip (e.g. a huge custom animation) is left as it is.
fn split_animation_strip(pack: &mut Pack, ctx: &mut Context, name: &str, frames: u32) {
    let path = animation_strip_path(name);
    let Some(strip) = load_png(pack, ctx, &path) else {
        return;
    };
    let size = strip.width();
    if size.checked_mul(frames) != Some(strip.height()) {
        ctx.warn(format!(
            "{path} is {}x{}, expected a strip of {frames} square frames: not split",
            strip.width(),
            strip.height()
        ));
        return;
    }
    for index in 0..frames {
        let frame = imageops::crop_imm(&strip, 0, index * size, size, size).to_image();
        if !save_png(pack, ctx, &animation_frame_path(name, index), &frame) {
            return;
        }
    }
    pack.remove(&path);
    pack.remove(&format!("{path}.mcmeta"));
}

/// Inverse of [`split_animation_strip`]. The 1.8 `.mcmeta` only marks the texture as animated.
/// Frames are pasted one by one so that at most one frame and the strip are in memory.
fn join_animation_frames(pack: &mut Pack, ctx: &mut Context, name: &str, frames: u32) {
    let path = animation_strip_path(name);
    let Some(first) = load_png(pack, ctx, &animation_frame_path(name, 0)) else {
        return;
    };
    let size = first.width();
    let Some(mut strip) = canvas(ctx, &path, Some(size), size.checked_mul(frames)) else {
        return;
    };
    imageops::replace(&mut strip, &first, 0, 0);
    for index in 1..frames {
        let Some(frame) = load_png(pack, ctx, &animation_frame_path(name, index)) else {
            ctx.warn(format!("{name} needs {frames} frames to build the 1.8 strip: not joined"));
            return;
        };
        if frame.width() != size || frame.height() != size {
            ctx.warn(format!(
                "{name} needs {frames} square frames of equal size to build the 1.8 strip: not joined"
            ));
            return;
        }
        imageops::replace(&mut strip, &frame, 0, i64::from(index * size));
    }
    if !save_png(pack, ctx, &path, &strip) {
        return;
    }
    pack.insert(format!("{path}.mcmeta"), br#"{"animation":{}}"#.to_vec());
    for index in 0..frames {
        pack.remove(&animation_frame_path(name, index));
    }
}

fn parent_of(model: &Value) -> Option<&str> {
    model.get("parent")?.as_str()
}

/// Reads a model only to look at it; malformed files are reported by the conversion pass.
fn read_model(pack: &Pack, path: &str) -> Option<Value> {
    read_json(pack, path)?.ok()
}

/// 1.8 drew the compass and clock with the special parents `builtin/compass` and
/// `builtin/clock`. 1.9 replaced them with `overrides` over the per-frame models, so such a
/// model would not load; vanilla's 1.9 model takes over and uses the sliced frames.
fn drop_dial_models(pack: &mut Pack) {
    for path in json_paths(pack, "models") {
        let is_dial = read_model(pack, &path)
            .is_some_and(|m| matches!(parent_of(&m), Some("builtin/compass" | "builtin/clock")));
        if is_dial {
            pack.remove(&path);
        }
    }
}

/// Inverse of [`drop_dial_models`]: the 1.9 compass and clock models are `overrides` over
/// frame models that 1.8 does not have, so vanilla's 1.8 model takes over.
fn drop_overridden_dial_models(pack: &mut Pack) {
    for (name, _) in ANIMATION_STRIPS {
        let path = format!("{MC}/models/item/{name}.json");
        if read_model(pack, &path).is_some_and(|m| m.get("overrides").is_some()) {
            pack.remove(&path);
        }
    }
}

/// 1.8 has no `multipart`; the model of such a block cannot be expressed with `variants`
/// alone without enumerating every state, so the vanilla 1.8 blockstate takes over.
fn drop_multipart_blockstates(pack: &mut Pack, ctx: &mut Context) {
    let mut removed = 0;
    for path in json_paths(pack, "blockstates") {
        match read_json(pack, &path) {
            Some(Ok(state)) if state.get("multipart").is_some() => {
                pack.remove(&path);
                removed += 1;
            }
            Some(Err(err)) => ctx.warn(format!("skipped: {err}")),
            _ => {}
        }
    }
    if removed > 0 {
        ctx.warn(format!(
            "Minecraft 1.8 has no multipart blockstates: removed {removed} files"
        ));
    }
}

/// Builds the off-hand slot art in `widgets.png` (256x256 at vanilla size) from the hotbar
/// edges: the left slot is a 29x24 box at (24, 22) and the right one a 29x24 box at (53, 22).
/// Each slot's art is 22x22, aligned to the outer side of its box: 11 px of the hotbar's
/// left edge followed by 11 px of its right edge.
fn add_off_hand_slot(pack: &mut Pack, ctx: &mut Context) {
    let path = format!("{MC}/textures/gui/widgets.png");
    let Some(mut widgets) = load_png(pack, ctx, &path) else {
        return;
    };
    if widgets.width() % 256 != 0 || widgets.width() != widgets.height() {
        ctx.warn(format!(
            "{path} is {}x{}, expected a square multiple of 256: off-hand slot not added",
            widgets.width(),
            widgets.height()
        ));
        return;
    }
    let scale = widgets.width() / 256;
    let area = |x: u32, y: u32, w: u32, h: u32| (x * scale, y * scale, w * scale, h * scale);

    let (x, y, w, h) = area(0, 0, 11, 22);
    let hotbar_left = imageops::crop_imm(&widgets, x, y, w, h).to_image();
    let (x, y, w, h) = area(171, 0, 11, 22);
    let hotbar_right = imageops::crop_imm(&widgets, x, y, w, h).to_image();

    for (box_x, art_x) in [(24, 24), (53, 60)] {
        let (x, y, w, h) = area(box_x, 22, 29, 24);
        let Some(blank) = canvas(ctx, &path, Some(w), Some(h)) else { return };
        imageops::replace(&mut widgets, &blank, i64::from(x), i64::from(y));
        imageops::replace(
            &mut widgets,
            &hotbar_left,
            i64::from(art_x * scale),
            i64::from(23 * scale),
        );
        imageops::replace(
            &mut widgets,
            &hotbar_right,
            i64::from((art_x + 11) * scale),
            i64::from(23 * scale),
        );
    }
    save_png(pack, ctx, &path, &widgets);
}

/// The 1.9 display defaults that `upgrade_display` adds (`ground`, `head` and the right hands).
fn display_defaults_1_9() -> &'static Map<String, Value> {
    static DATA: OnceLock<Map<String, Value>> = OnceLock::new();
    DATA.get_or_init(|| {
        serde_json::from_str(include_str!("../../../data/rpc/forwards/display.json"))
            .expect("embedded display.json is valid")
    })
}

/// The `thirdperson`/`firstperson` values vanilla 1.8 gives each item parent introduced in 1.9.
fn display_defaults_1_8() -> &'static Map<String, Value> {
    static DATA: OnceLock<Map<String, Value>> = OnceLock::new();
    DATA.get_or_init(|| {
        serde_json::from_str(include_str!("../../../data/legacy/display_1_8.json"))
            .expect("embedded display_1_8.json is valid")
    })
}

fn read_vec3(value: &Value) -> Option<Vec3> {
    match value.as_array()?.as_slice() {
        [x, y, z] => Some([x.as_f64()?, y.as_f64()?, z.as_f64()?]),
        _ => None,
    }
}

/// Rounds to 4 decimals so that `0.4 * 4.1` does not leak float noise into the JSON.
fn vec3_value(v: Vec3) -> Value {
    Value::Array(v.iter().map(|x| json!((x * 1e4).round() / 1e4)).collect())
}

fn add(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn sub(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn mul(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] * b[0], a[1] * b[1], a[2] * b[2]]
}

fn div(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] / b[0], a[1] / b[1], a[2] / b[2]]
}

/// Applies `f` to the triple stored under `key`; anything that is not three numbers stays.
fn map_vec3(transform: &mut Map<String, Value>, key: &str, f: impl Fn(Vec3) -> Vec3) {
    if let Some(value) = transform.get_mut(key) {
        if let Some(vec) = read_vec3(value) {
            *value = vec3_value(f(vec));
        }
    }
}

// The 1.8 -> 1.9 conversion of the hand transforms. In 1.8 the item was positioned relative to
// a camera pose; 1.9 positions it relative to the hand. These are the constants of the
// ResourcePackConverter reference implementation, applied per component.

fn first_person_up(t: &mut Map<String, Value>) {
    map_vec3(t, "rotation", |r| add(r, [0.0, 45.0, 0.0]));
    map_vec3(t, "translation", |p| {
        add(mul(sub(p, [0.0, 4.0, 2.0]), [0.4; 3]), [1.13, 3.2, 1.13])
    });
    map_vec3(t, "scale", |s| mul(s, [0.4; 3]));
}

fn first_person_down(t: &mut Map<String, Value>) {
    map_vec3(t, "rotation", |r| sub(r, [0.0, 45.0, 0.0]));
    map_vec3(t, "translation", |p| {
        add(div(sub(p, [1.13, 3.2, 1.13]), [0.4; 3]), [0.0, 4.0, 2.0])
    });
    map_vec3(t, "scale", |s| div(s, [0.4; 3]));
}

fn third_person_up(t: &mut Map<String, Value>) {
    map_vec3(t, "rotation", |r| {
        add(mul(r, [1.0, -1.0, -1.0]), [0.0, 0.0, 20.0])
    });
    map_vec3(t, "translation", |p| {
        add(mul(p, [1.0, 1.0, -1.0]), [0.0, 2.75, -3.0])
    });
}

fn third_person_down(t: &mut Map<String, Value>) {
    map_vec3(t, "rotation", |r| {
        div(sub(r, [0.0, 0.0, 20.0]), [1.0, -1.0, -1.0])
    });
    map_vec3(t, "translation", |p| {
        div(sub(p, [0.0, 2.75, -3.0]), [1.0, 1.0, -1.0])
    });
}

/// The left hand is the right hand mirrored: yaw and roll flip sign.
fn mirror_hand(right: &Value) -> Value {
    let mut left = right.clone();
    if let Some(transform) = left.as_object_mut() {
        map_vec3(transform, "rotation", |r| [r[0], -r[1], -r[2]]);
    }
    left
}

fn upgrade_display(display: &mut Map<String, Value>) {
    let defaults = display_defaults_1_9();
    let poses: [(&str, PoseConversion); 2] = [
        ("firstperson", first_person_up),
        ("thirdperson", third_person_up),
    ];
    for (pose, convert) in poses {
        let right = format!("{pose}_righthand");
        let left = format!("{pose}_lefthand");
        if let Some(Value::Object(mut transform)) = display.remove(pose) {
            convert(&mut transform);
            display.insert(right.clone(), Value::Object(transform));
        } else if !display.contains_key(&right) {
            display.insert(right.clone(), defaults[&right].clone());
        }
        if !display.contains_key(&left) {
            let mirrored = mirror_hand(&display[&right]);
            display.insert(left, mirrored);
        }
    }
    for key in ["ground", "head"] {
        if !display.contains_key(key) {
            display.insert(key.to_string(), defaults[key].clone());
        }
    }
}

/// `fallback` supplies the 1.8 poses for a model whose 1.9 parent carried them.
fn downgrade_display(display: &mut Map<String, Value>, fallback: Option<&Value>) {
    let poses: [(&str, PoseConversion); 2] = [
        ("firstperson", first_person_down),
        ("thirdperson", third_person_down),
    ];
    for (pose, convert) in poses {
        let right = display.remove(&format!("{pose}_righthand"));
        let left = display.remove(&format!("{pose}_lefthand"));
        // Only the right hand has a 1.8 equivalent; the left one is used when it is alone.
        match right.or_else(|| left.map(|left| mirror_hand(&left))) {
            Some(Value::Object(mut transform)) => {
                convert(&mut transform);
                display.insert(pose.to_string(), Value::Object(transform));
            }
            _ => {
                if let Some(default) = fallback.and_then(|f| f.get(pose)) {
                    display.insert(pose.to_string(), default.clone());
                }
            }
        }
    }
    // Remove the entries `upgrade_display` adds, but keep ones the pack customised.
    for key in ["ground", "head"] {
        if display.get(key) == display_defaults_1_9().get(key) {
            display.remove(key);
        }
    }
}

fn upgrade_model(model: &mut Value) -> bool {
    match model.get_mut("display") {
        Some(Value::Object(display)) => {
            upgrade_display(display);
            true
        }
        _ => false,
    }
}

/// Converts display transforms back and replaces the 1.9-only item parents by
/// `builtin/generated`, adding the poses those parents provided.
fn downgrade_model(model: &mut Value) -> bool {
    let item_parent = parent_of(model)
        .map(|p| p.strip_prefix("minecraft:").unwrap_or(p))
        .filter(|p| ITEM_PARENTS.contains(p))
        .map(str::to_string);
    let Some(model_map) = model.as_object_mut() else {
        return false;
    };
    let fallback = item_parent
        .as_deref()
        .and_then(|p| display_defaults_1_8().get(p));

    let mut changed = false;
    match model_map.get_mut("display") {
        Some(Value::Object(display)) => {
            downgrade_display(display, fallback);
            changed = true;
        }
        _ => {
            if let Some(fallback) = fallback {
                model_map.insert("display".to_string(), fallback.clone());
                changed = true;
            }
        }
    }
    if item_parent.is_some() {
        model_map.insert("parent".to_string(), json!("builtin/generated"));
        changed = true;
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::steps::legacy::common::testing::*;
    use image::{Rgba, RgbaImage};

    fn up() -> Context {
        Context::new(Version::new(1, 8, 0), Version::new(1, 9, 0))
    }

    fn down() -> Context {
        Context::new(Version::new(1, 9, 0), Version::new(1, 8, 0))
    }

    /// A strip whose frame `i` is filled with the color `[i, 0, 0, 255]`.
    fn strip(size: u32, frames: u32) -> RgbaImage {
        let mut strip = RgbaImage::new(size, size * frames);
        for (_, y, pixel) in strip.enumerate_pixels_mut() {
            *pixel = Rgba([(y / size) as u8, 0, 0, 255]);
        }
        strip
    }

    #[test]
    fn splits_and_rejoins_compass_and_clock_strips() {
        for (name, frames) in ANIMATION_STRIPS {
            for size in [16, 32] {
                let mut pack = Pack::new();
                let original = strip(size, frames);
                put_png(&mut pack, &animation_strip_path(name), &original);
                put_json(
                    &mut pack,
                    &format!("{}.mcmeta", animation_strip_path(name)),
                    "{}",
                );

                V1_9.upgrade(&mut pack, &mut up()).unwrap();
                assert!(!pack.contains(&animation_strip_path(name)));
                assert!(!pack.contains(&format!("{}.mcmeta", animation_strip_path(name))));
                let frame = get_png(&pack, &animation_frame_path(name, frames - 1));
                assert_eq!(frame.dimensions(), (size, size));
                assert_eq!(frame.get_pixel(0, 0).0, [(frames - 1) as u8, 0, 0, 255]);
                assert!(pack.contains(&animation_frame_path(name, 0)));

                V1_9.downgrade(&mut pack, &mut down()).unwrap();
                assert_eq!(get_png(&pack, &animation_strip_path(name)), original);
                assert!(!pack.contains(&animation_frame_path(name, 0)));
                assert_eq!(
                    pack.get(&format!("{}.mcmeta", animation_strip_path(name)))
                        .unwrap(),
                    br#"{"animation":{}}"#
                );
            }
        }
    }

    #[test]
    fn wrong_strip_size_is_reported_and_kept() {
        let mut pack = Pack::new();
        put_png(
            &mut pack,
            &animation_strip_path("compass"),
            &solid(16, 100, [0, 0, 0, 255]),
        );
        let mut ctx = up();
        V1_9.upgrade(&mut pack, &mut ctx).unwrap();
        assert!(pack.contains(&animation_strip_path("compass")));
        assert_eq!(ctx.into_warnings().len(), 1);
    }

    #[test]
    fn incomplete_frames_are_not_joined() {
        let mut pack = Pack::new();
        put_png(
            &mut pack,
            &animation_frame_path("clock", 0),
            &solid(16, 16, [1, 0, 0, 255]),
        );
        let mut ctx = down();
        V1_9.downgrade(&mut pack, &mut ctx).unwrap();
        assert!(!pack.contains(&animation_strip_path("clock")));
        assert_eq!(ctx.into_warnings().len(), 2); // not joined + 1.9-only file removed
    }

    #[test]
    fn off_hand_slot_is_built_from_the_hotbar_at_any_scale() {
        for scale in [1u32, 2] {
            let mut widgets = solid(256 * scale, 256 * scale, [0, 0, 0, 0]);
            // Left hotbar edge red, right hotbar edge blue.
            for y in 0..22 * scale {
                for x in 0..11 * scale {
                    widgets.put_pixel(x, y, Rgba([255, 0, 0, 255]));
                    widgets.put_pixel(171 * scale + x, y, Rgba([0, 0, 255, 255]));
                }
            }
            // Junk inside the box that must be cleared.
            widgets.put_pixel(50 * scale, 30 * scale, Rgba([9, 9, 9, 255]));
            let mut pack = Pack::new();
            put_png(
                &mut pack,
                "assets/minecraft/textures/gui/widgets.png",
                &widgets,
            );

            V1_9.upgrade(&mut pack, &mut up()).unwrap();

            let out = get_png(&pack, "assets/minecraft/textures/gui/widgets.png");
            let at = |x: u32, y: u32| out.get_pixel(x * scale, y * scale).0;
            assert_eq!(at(24, 23), [255, 0, 0, 255]);
            assert_eq!(at(35, 23), [0, 0, 255, 255]);
            assert_eq!(at(60, 23), [255, 0, 0, 255]);
            assert_eq!(at(71, 23), [0, 0, 255, 255]);
            assert_eq!(at(50, 30), [0, 0, 0, 0]);
            assert_eq!(at(53, 22), [0, 0, 0, 0]);
            assert_eq!(at(24, 22), [0, 0, 0, 0]);
            // The hotbar itself is untouched.
            assert_eq!(at(0, 0), [255, 0, 0, 255]);
        }
    }

    #[test]
    fn odd_sized_widgets_are_reported_and_kept() {
        let mut pack = Pack::new();
        let widgets = solid(100, 100, [1, 2, 3, 4]);
        put_png(
            &mut pack,
            "assets/minecraft/textures/gui/widgets.png",
            &widgets,
        );
        let mut ctx = up();
        V1_9.upgrade(&mut pack, &mut ctx).unwrap();
        assert_eq!(
            get_png(&pack, "assets/minecraft/textures/gui/widgets.png"),
            widgets
        );
        assert_eq!(ctx.into_warnings().len(), 1);
    }

    #[test]
    fn display_upgrade_matches_the_reference_formulas() {
        let mut pack = Pack::new();
        put_json(
            &mut pack,
            "assets/minecraft/models/item/x.json",
            r#"{"parent":"builtin/generated","display":{
                "firstperson":{"rotation":[0,-135,25],"translation":[0,4,2],"scale":[1.7,1.7,1.7]},
                "thirdperson":{"rotation":[-90,0,0],"translation":[0,1,-3],"scale":[0.55,0.55,0.55]}}}"#,
        );
        V1_9.upgrade(&mut pack, &mut up()).unwrap();

        let display = &get_json(&pack, "assets/minecraft/models/item/x.json")["display"];
        assert!(display.get("firstperson").is_none());
        assert_eq!(
            display["firstperson_righthand"]["rotation"],
            json!([0.0, -90.0, 25.0])
        );
        assert_eq!(
            display["firstperson_righthand"]["translation"],
            json!([1.13, 3.2, 1.13])
        );
        assert_eq!(
            display["firstperson_righthand"]["scale"],
            json!([0.68, 0.68, 0.68])
        );
        assert_eq!(
            display["firstperson_lefthand"]["rotation"],
            json!([0.0, 90.0, -25.0])
        );
        assert_eq!(
            display["thirdperson_righthand"]["rotation"],
            json!([-90.0, 0.0, 20.0])
        );
        assert_eq!(
            display["thirdperson_righthand"]["translation"],
            json!([0.0, 3.75, 0.0])
        );
        assert_eq!(
            display["thirdperson_righthand"]["scale"],
            json!([0.55, 0.55, 0.55])
        );
        assert_eq!(display["ground"], display_defaults_1_9()["ground"]);
        assert_eq!(display["head"], display_defaults_1_9()["head"]);
    }

    #[test]
    fn display_roundtrip_restores_the_1_8_poses() {
        let model = r#"{"parent":"builtin/generated","display":{
            "firstperson":{"rotation":[10,-135,25],"translation":[1,4.5,2],"scale":[1.7,1.7,1.7]},
            "thirdperson":{"rotation":[-90,5,0],"translation":[0,1,-3],"scale":[0.55,0.55,0.55]}}}"#;
        let mut pack = Pack::new();
        put_json(&mut pack, "assets/minecraft/models/item/x.json", model);

        V1_9.upgrade(&mut pack, &mut up()).unwrap();
        V1_9.downgrade(&mut pack, &mut down()).unwrap();

        let restored = get_json(&pack, "assets/minecraft/models/item/x.json");
        let original: Value = serde_json::from_str(model).unwrap();
        let (r, o) = (&restored["display"], &original["display"]);
        for pose in ["firstperson", "thirdperson"] {
            for field in ["rotation", "translation", "scale"] {
                let got = read_vec3(&r[pose][field]).unwrap();
                let want = read_vec3(&o[pose][field]).unwrap();
                for i in 0..3 {
                    assert!(
                        (got[i] - want[i]).abs() < 1e-3,
                        "{pose}.{field}: {got:?} vs {want:?}"
                    );
                }
            }
        }
        assert!(r.get("ground").is_none() && r.get("head").is_none());
        assert!(r.get("firstperson_lefthand").is_none());
        assert_eq!(restored["parent"], "builtin/generated");
    }

    #[test]
    fn downgrade_replaces_item_parents_and_adds_their_1_8_poses() {
        let mut pack = Pack::new();
        put_json(
            &mut pack,
            "assets/minecraft/models/item/sword.json",
            r#"{"parent":"item/handheld","textures":{"layer0":"items/sword"}}"#,
        );
        put_json(
            &mut pack,
            "assets/minecraft/models/item/apple.json",
            r#"{"parent":"minecraft:item/generated","textures":{"layer0":"items/apple"}}"#,
        );
        put_json(
            &mut pack,
            "assets/minecraft/models/block/other.json",
            r#"{"parent":"block/cube_all"}"#,
        );
        V1_9.downgrade(&mut pack, &mut down()).unwrap();

        let sword = get_json(&pack, "assets/minecraft/models/item/sword.json");
        assert_eq!(sword["parent"], "builtin/generated");
        assert_eq!(
            sword["display"]["thirdperson"]["rotation"],
            json!([0, 90, -35])
        );
        let apple = get_json(&pack, "assets/minecraft/models/item/apple.json");
        assert_eq!(
            apple["display"]["thirdperson"]["scale"],
            json!([0.55, 0.55, 0.55])
        );
        let other = get_json(&pack, "assets/minecraft/models/block/other.json");
        assert_eq!(other, json!({"parent": "block/cube_all"}));
    }

    #[test]
    fn dial_models_follow_the_target_format() {
        let mut pack = Pack::new();
        put_json(
            &mut pack,
            "assets/minecraft/models/item/compass.json",
            r#"{"parent":"builtin/compass","textures":{"layer0":"items/compass"}}"#,
        );
        V1_9.upgrade(&mut pack, &mut up()).unwrap();
        assert!(!pack.contains("assets/minecraft/models/item/compass.json"));

        put_json(
            &mut pack,
            "assets/minecraft/models/item/clock.json",
            r#"{"parent":"item/generated","overrides":[{"predicate":{"time":0},"model":"item/clock_01"}]}"#,
        );
        V1_9.downgrade(&mut pack, &mut down()).unwrap();
        assert!(!pack.contains("assets/minecraft/models/item/clock.json"));
    }

    #[test]
    fn downgrade_removes_multipart_blockstates_only() {
        let mut pack = Pack::new();
        put_json(
            &mut pack,
            "assets/minecraft/blockstates/fence.json",
            r#"{"multipart":[{"apply":{"model":"oak_fence_post"}}]}"#,
        );
        put_json(
            &mut pack,
            "assets/minecraft/blockstates/stone.json",
            r#"{"variants":{"normal":{"model":"stone"}}}"#,
        );
        let mut ctx = down();
        V1_9.downgrade(&mut pack, &mut ctx).unwrap();
        assert!(!pack.contains("assets/minecraft/blockstates/fence.json"));
        assert!(pack.contains("assets/minecraft/blockstates/stone.json"));
        assert_eq!(ctx.into_warnings().len(), 1);
    }

    #[test]
    fn boat_textures_and_references_roundtrip() {
        let mut pack = Pack::new();
        put_png(
            &mut pack,
            "assets/minecraft/textures/items/boat.png",
            &solid(1, 1, [1; 4]),
        );
        put_png(
            &mut pack,
            "assets/minecraft/textures/entity/boat.png",
            &solid(1, 1, [2; 4]),
        );
        put_json(
            &mut pack,
            "assets/minecraft/textures/entity/boat.png.mcmeta",
            "{}",
        );
        put_json(
            &mut pack,
            "assets/minecraft/models/item/boat.json",
            r#"{"parent":"builtin/generated","textures":{"layer0":"items/boat"}}"#,
        );

        V1_9.upgrade(&mut pack, &mut up()).unwrap();
        assert!(pack.contains("assets/minecraft/textures/items/oak_boat.png"));
        assert!(pack.contains("assets/minecraft/textures/entity/boat/boat_oak.png"));
        assert!(pack.contains("assets/minecraft/textures/entity/boat/boat_oak.png.mcmeta"));
        let model = get_json(&pack, "assets/minecraft/models/item/boat.json");
        assert_eq!(model["textures"]["layer0"], "items/oak_boat");

        V1_9.downgrade(&mut pack, &mut down()).unwrap();
        assert!(pack.contains("assets/minecraft/textures/items/boat.png"));
        assert!(pack.contains("assets/minecraft/textures/entity/boat.png.mcmeta"));
        let model = get_json(&pack, "assets/minecraft/models/item/boat.json");
        assert_eq!(model["textures"]["layer0"], "items/boat");
    }

    #[test]
    fn downgrade_drops_1_9_textures() {
        let mut pack = Pack::new();
        put_png(
            &mut pack,
            "assets/minecraft/textures/entity/elytra.png",
            &solid(1, 1, [1; 4]),
        );
        put_png(
            &mut pack,
            "assets/minecraft/textures/entity/zombie/zombie.png",
            &solid(1, 1, [2; 4]),
        );
        V1_9.downgrade(&mut pack, &mut down()).unwrap();
        assert!(!pack.contains("assets/minecraft/textures/entity/elytra.png"));
        assert!(pack.contains("assets/minecraft/textures/entity/zombie/zombie.png"));
    }

    #[test]
    fn malformed_model_is_reported_and_skipped() {
        let mut pack = Pack::new();
        put_json(
            &mut pack,
            "assets/minecraft/models/item/bad.json",
            "{not json",
        );
        let mut ctx = up();
        V1_9.upgrade(&mut pack, &mut ctx).unwrap();
        assert_eq!(
            pack.get("assets/minecraft/models/item/bad.json").unwrap(),
            b"{not json"
        );
        assert_eq!(ctx.into_warnings().len(), 1);
    }

    #[test]
    fn oversized_images_are_left_alone() {
        let mut pack = Pack::new();
        let strip = animation_strip_path("compass");
        let frame = animation_frame_path("clock", 0);
        let widgets = format!("{MC}/textures/gui/widgets.png");
        for path in [&strip, &frame, &widgets] {
            pack.insert(path, crate::steps::support::oversized_png());
        }
        let mut ctx = up();
        V1_9.upgrade(&mut pack, &mut ctx).unwrap();
        join_animation_frames(&mut pack, &mut ctx, "clock", 64);
        for path in [&strip, &frame, &widgets] {
            assert_eq!(pack.get(path), Some(&crate::steps::support::oversized_png()[..]), "{path}");
        }
        assert_eq!(ctx.into_warnings().len(), 3);
    }

    #[test]
    fn boat_rename_keeps_an_existing_destination() {
        let mut pack = Pack::new();
        put_png(&mut pack, &format!("{MC}/textures/items/boat.png"), &solid(1, 1, [1, 0, 0, 255]));
        put_png(&mut pack, &format!("{MC}/textures/items/oak_boat.png"), &solid(1, 1, [2, 0, 0, 255]));
        let mut ctx = up();
        V1_9.upgrade(&mut pack, &mut ctx).unwrap();
        assert_eq!(get_png(&pack, &format!("{MC}/textures/items/oak_boat.png")).get_pixel(0, 0).0[0], 2);
        assert!(!pack.contains(&format!("{MC}/textures/items/boat.png")));
        assert_eq!(ctx.into_warnings().len(), 1);
    }
}

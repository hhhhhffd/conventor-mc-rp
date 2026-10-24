//! 1.20: pottery shards became sherds, and the title logo texture was re-laid out.

use image::RgbaImage;

use super::common::{crop, paste, rename_ids, reversed};
use crate::error::Result;
use crate::step::{Context, Step};
use crate::util::{read_png, write_png};
use crate::version::Version;
use crate::vfs::Pack;

const TITLE: &str = "assets/minecraft/textures/gui/title/minecraft.png";

const POTTERY: &[(&str, &str)] = &[
    ("item/pottery_shard_archer", "item/archer_pottery_sherd"),
    ("item/pottery_shard_arms_up", "item/arms_up_pottery_sherd"),
    ("item/pottery_shard_prize", "item/prize_pottery_sherd"),
    ("item/pottery_shard_skull", "item/skull_pottery_sherd"),
    ("entity/decorated_pot/pottery_pattern_archer", "entity/decorated_pot/archer_pottery_pattern"),
    ("entity/decorated_pot/pottery_pattern_arms_up", "entity/decorated_pot/arms_up_pottery_pattern"),
    ("entity/decorated_pot/pottery_pattern_prize", "entity/decorated_pot/prize_pottery_pattern"),
    ("entity/decorated_pot/pottery_pattern_skull", "entity/decorated_pot/skull_pottery_pattern"),
];

pub struct V1_20;

impl Step for V1_20 {
    fn name(&self) -> &'static str {
        "1.20 pottery sherds and title logo"
    }

    fn version(&self) -> Version {
        Version::new(1, 20, 0)
    }

    fn upgrade(&self, pack: &mut Pack, ctx: &mut Context) -> Result<()> {
        rename_ids(pack, ctx, POTTERY);
        upgrade_title(pack, ctx)
    }

    fn downgrade(&self, pack: &mut Pack, ctx: &mut Context) -> Result<()> {
        rename_ids(pack, ctx, &reversed(POTTERY));
        downgrade_title(pack, ctx)
    }
}

/// The old logo is a square 256x256 sheet with "MINE" at (0,0,155x44) and "CRAFT" below it at
/// (0,45,119x45). 1.20 draws both halves side by side from a 274x64 design-space sheet.
/// MINE is cut 156 wide and CRAFT is pasted at x=155, so they overlap by one column (as the upstream converter does).
fn upgrade_title(pack: &mut Pack, ctx: &mut Context) -> Result<()> {
    let Some(img) = read_png(pack, TITLE) else { return Ok(()) };
    let img = match img {
        Ok(img) => img,
        Err(e) => {
            ctx.warn(format!("{e}; title logo not converted"));
            return Ok(());
        }
    };
    let (w, h) = img.dimensions();
    if w != h {
        // Already in the new, non-square layout.
        return Ok(());
    }
    if w % 256 != 0 {
        ctx.warn(format!("{TITLE} is {w}x{h}, not a multiple of 256; title logo not converted"));
        return Ok(());
    }
    let s = w / 256;
    let mut out = RgbaImage::new(274 * s, 64 * s);
    if let (Some(mine), Some(craft)) = (crop(&img, 0, 0, 156 * s, 44 * s), crop(&img, 0, 45 * s, 119 * s, 45 * s)) {
        paste(&mut out, &mine, 0, 0);
        paste(&mut out, &craft, 155 * s, 0);
    }
    write_png(pack, TITLE, &out)
}

fn downgrade_title(pack: &mut Pack, ctx: &mut Context) -> Result<()> {
    let Some(img) = read_png(pack, TITLE) else { return Ok(()) };
    let img = match img {
        Ok(img) => img,
        Err(e) => {
            ctx.warn(format!("{e}; title logo not converted"));
            return Ok(());
        }
    };
    let (w, h) = img.dimensions();
    let s = h / 64;
    if s == 0 || w != 274 * s || h != 64 * s {
        ctx.warn(format!(
            "{TITLE} is {w}x{h}, not the 274x64 layout produced by the upgrade; the 1.20 logo cannot be split back and was left unchanged"
        ));
        return Ok(());
    }
    let mut out = RgbaImage::new(256 * s, 256 * s);
    if let (Some(mine), Some(craft)) = (crop(&img, 0, 0, 156 * s, 44 * s), crop(&img, 155 * s, 0, 119 * s, 45 * s)) {
        paste(&mut out, &mine, 0, 0);
        paste(&mut out, &craft, 0, 45 * s);
    }
    write_png(pack, TITLE, &out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::steps::modern::common::testing::{png, put_png, solid};

    fn ctx() -> Context {
        Context::new(Version::new(1, 19, 4), Version::new(1, 20, 0))
    }

    #[test]
    fn renames_sherds_and_references() {
        let mut pack = Pack::new();
        let mut ctx = ctx();
        put_png(&mut pack, "assets/minecraft/textures/item/pottery_shard_archer.png", &solid(1, 1, [1, 2, 3, 255]));
        pack.insert(
            "assets/minecraft/models/item/pottery_shard_archer.json",
            br#"{"parent":"item/generated","textures":{"layer0":"minecraft:item/pottery_shard_archer"}}"#.to_vec(),
        );
        V1_20.upgrade(&mut pack, &mut ctx).unwrap();
        assert!(pack.contains("assets/minecraft/textures/item/archer_pottery_sherd.png"));
        let model = String::from_utf8(pack.get("assets/minecraft/models/item/archer_pottery_sherd.json").unwrap().to_vec()).unwrap();
        assert!(model.contains("minecraft:item/archer_pottery_sherd"));
        V1_20.downgrade(&mut pack, &mut ctx).unwrap();
        assert!(pack.contains("assets/minecraft/textures/item/pottery_shard_archer.png"));
        assert!(pack.contains("assets/minecraft/models/item/pottery_shard_archer.json"));
    }

    #[test]
    fn title_roundtrip_restores_halves() {
        let mut pack = Pack::new();
        let mut ctx = ctx();
        let mut old = solid(256, 256, [0, 0, 0, 0]);
        paste(&mut old, &solid(155, 44, [255, 0, 0, 255]), 0, 0);
        paste(&mut old, &solid(119, 45, [0, 255, 0, 255]), 0, 45);
        put_png(&mut pack, TITLE, &old);
        V1_20.upgrade(&mut pack, &mut ctx).unwrap();
        let new = png(&pack, TITLE);
        assert_eq!(new.dimensions(), (274, 64));
        assert_eq!(new.get_pixel(10, 10).0, [255, 0, 0, 255]);
        assert_eq!(new.get_pixel(200, 10).0, [0, 255, 0, 255]);
        V1_20.downgrade(&mut pack, &mut ctx).unwrap();
        let back = png(&pack, TITLE);
        assert_eq!(back.dimensions(), (256, 256));
        assert_eq!(back.get_pixel(10, 10).0, [255, 0, 0, 255]);
        assert_eq!(back.get_pixel(10, 60).0, [0, 255, 0, 255]);
        assert_eq!(back.get_pixel(10, 150).0, [0, 0, 0, 0]);
    }

    #[test]
    fn vanilla_new_logo_is_not_touched_downwards() {
        let mut pack = Pack::new();
        let mut ctx = ctx();
        put_png(&mut pack, TITLE, &solid(1024, 256, [9, 9, 9, 255]));
        V1_20.downgrade(&mut pack, &mut ctx).unwrap();
        assert_eq!(png(&pack, TITLE).dimensions(), (1024, 256));
        assert_eq!(ctx.into_warnings().len(), 1);
    }
}

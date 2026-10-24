//! 1.14: paintings, mob effect icons, particles, explosion and sweep leave their sprite
//! sheets, plus the dye, sign and slab renames.

use std::sync::OnceLock;

use crate::error::Result;
use crate::step::{Context, Step};
use crate::version::Version;
use crate::vfs::Pack;

use super::rename::Renames;
use super::sheet::Sheet;

const TEXTURES: &str = "assets/minecraft/textures";

/// Sheets that are cut together; `name` is used in the downgrade warning.
struct Group {
    name: &'static str,
    sheets: Vec<Sheet>,
}

fn groups() -> &'static [Group] {
    static GROUPS: OnceLock<Vec<Group>> = OnceLock::new();
    GROUPS.get_or_init(|| {
        let mut particles = Sheet::parse_all(
            include_str!("../../../data/rpc/forwards/particles.json"),
            &format!("{TEXTURES}/particle"),
        );
        // The vendored definition declares 128x128 but its boxes are in the 256x256 space of the
        // 1.13 sheet (Mojang/slicer 1.14 uses the same boxes).
        particles = particles.into_iter().map(|s| s.with_design(256, 256)).collect();
        // The fishing hook is an entity texture since 1.14, not a particle.
        for sprite in particles.iter_mut().flat_map(|s| s.sprites.iter_mut()) {
            if sprite.path.ends_with("/particle/fishing_hook.png") {
                sprite.path = format!("{TEXTURES}/entity/fishing_hook.png");
            }
        }
        vec![
            Group {
                name: "painting",
                sheets: Sheet::parse_all(
                    include_str!(
                        "../../../data/rpc/forwards/paintings_kristoffer_zetterstrand.json"
                    ),
                    &format!("{TEXTURES}/painting"),
                ),
            },
            Group {
                name: "mob effect",
                sheets: Sheet::parse_all(
                    include_str!("../../../data/rpc/forwards/mob_effect.json"),
                    TEXTURES,
                ),
            },
            Group { name: "particle", sheets: particles },
            Group {
                name: "explosion and sweep",
                sheets: Sheet::parse_all(
                    include_str!("../../../data/mid/slices_1_14.json"),
                    TEXTURES,
                ),
            },
        ]
    })
}

const RENAMES: Renames = Renames {
    textures: &[
        ("item/rose_red", "item/red_dye"),
        ("item/cactus_green", "item/green_dye"),
        ("item/dandelion_yellow", "item/yellow_dye"),
        ("item/sign", "item/oak_sign"),
        ("block/stone_slab_top", "block/smooth_stone"),
        ("block/stone_slab_side", "block/smooth_stone_slab_side"),
        ("entity/sign", "entity/signs/oak"),
        ("entity/cow/mooshroom", "entity/cow/red_mooshroom"),
    ],
    models: &[
        ("item/rose_red", "item/red_dye"),
        ("item/cactus_green", "item/green_dye"),
        ("item/dandelion_yellow", "item/yellow_dye"),
        ("item/sign", "item/oak_sign"),
    ],
    // Ink sacs, lapis, cocoa beans and bone meal remain items, but each now also has a dye.
    texture_copies: &[
        ("item/ink_sac", "item/black_dye"),
        ("item/lapis_lazuli", "item/blue_dye"),
        ("item/cocoa_beans", "item/brown_dye"),
        ("item/bone_meal", "item/white_dye"),
    ],
    model_copies: &[
        ("item/ink_sac", "item/black_dye"),
        ("item/lapis_lazuli", "item/blue_dye"),
        ("item/cocoa_beans", "item/brown_dye"),
        ("item/bone_meal", "item/white_dye"),
    ],
    // Same as the "1_14" section of data/rpc/forwards/lang.json.
    lang: &[
        ("title.oldgl1", "title.oldgl.eol.line1"),
        ("title.oldgl2", "title.oldgl.eol.line2"),
        ("block.minecraft.sign", "block.minecraft.oak_sign"),
        ("block.minecraft.wall_sign", "block.minecraft.oak_wall_sign"),
        ("item.minecraft.rose_red", "item.minecraft.red_dye"),
        ("item.minecraft.cactus_green", "item.minecraft.green_dye"),
        ("item.minecraft.dandelion_yellow", "item.minecraft.yellow_dye"),
        ("entity.minecraft.villager.tool_smith", "entity.minecraft.villager.toolsmith"),
        ("entity.minecraft.villager.weapon_smith", "entity.minecraft.villager.weaponsmith"),
    ],
    ..Renames::NONE
};

/// Textures of the pre-1.14 villager layout, which 1.14 replaced by layered profession,
/// level and type textures that cannot be derived from them.
const OLD_VILLAGER_TEXTURES: [&str; 10] = [
    "villager/butcher",
    "villager/farmer",
    "villager/librarian",
    "villager/priest",
    "villager/smith",
    "zombie_villager/zombie_butcher",
    "zombie_villager/zombie_farmer",
    "zombie_villager/zombie_librarian",
    "zombie_villager/zombie_priest",
    "zombie_villager/zombie_smith",
];

pub struct V1_14;

impl Step for V1_14 {
    fn name(&self) -> &'static str {
        "1.14 sprite sheets and renames"
    }

    fn version(&self) -> Version {
        Version::new(1, 14, 0)
    }

    fn upgrade(&self, pack: &mut Pack, ctx: &mut Context) -> Result<()> {
        for sheet in groups().iter().flat_map(|g| &g.sheets) {
            sheet.slice(pack, ctx)?;
        }
        RENAMES.upgrade(pack, ctx);
        if OLD_VILLAGER_TEXTURES
            .iter()
            .any(|t| pack.contains(&format!("{TEXTURES}/entity/{t}.png")))
        {
            ctx.warn("pre-1.14 villager textures (entity/villager/*.png) cannot be converted to the 1.14 layered layout and are ignored by the game");
        }
        Ok(())
    }

    fn downgrade(&self, pack: &mut Pack, ctx: &mut Context) -> Result<()> {
        for group in groups() {
            let mut partial = false;
            for sheet in &group.sheets {
                partial |= sheet.unslice(pack, ctx)?;
            }
            if partial {
                ctx.warn(format!(
                    "{} sprites were merged into a sheet that replaces the vanilla one, but only some sprites were in the pack; the missing ones are blank",
                    group.name
                ));
            }
        }
        RENAMES.downgrade(pack, ctx);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::{read_png, write_png};
    use image::{Rgba, RgbaImage};
    use serde_json::json;

    fn ctx() -> Context {
        Context::new(Version::new(1, 13, 0), Version::new(1, 14, 0))
    }

    fn solid(w: u32, h: u32, v: u8) -> RgbaImage {
        RgbaImage::from_pixel(w, h, Rgba([v, v, v, 255]))
    }

    fn png(pack: &Pack, path: &str) -> RgbaImage {
        read_png(pack, path).expect("file exists").expect("valid png")
    }

    fn old_pack() -> Pack {
        let mut pack = Pack::new();
        // 256x256 paintings sheet: kebab at (0,0) is 16x16, back at (240,0).
        let mut paintings = RgbaImage::new(256, 256);
        for (x, y, v) in [(0, 0, 10u8), (240, 0, 20)] {
            image::imageops::replace(&mut paintings, &solid(16, 16, v), x, y);
        }
        write_png(
            &mut pack,
            "assets/minecraft/textures/painting/paintings_kristoffer_zetterstrand.png",
            &paintings,
        )
        .unwrap();
        // Inventory with the speed icon at (0,198) and a marker pixel that must survive.
        let mut inventory = RgbaImage::new(256, 256);
        image::imageops::replace(&mut inventory, &solid(18, 18, 30), 0, 198);
        inventory.put_pixel(100, 100, Rgba([1, 2, 3, 255]));
        write_png(&mut pack, "assets/minecraft/textures/gui/container/inventory.png", &inventory)
            .unwrap();
        // Particles: generic_0 at (0,0), fishing hook at (8,16).
        let mut particles = RgbaImage::new(256, 256);
        image::imageops::replace(&mut particles, &solid(8, 8, 40), 0, 0);
        image::imageops::replace(&mut particles, &solid(8, 8, 50), 8, 16);
        write_png(&mut pack, "assets/minecraft/textures/particle/particles.png", &particles)
            .unwrap();
        // Explosion 128x128 grid of 32px cells and sweep 128x32.
        let explosion =
            RgbaImage::from_fn(128, 128, |x, y| Rgba([(x / 32 + 4 * (y / 32)) as u8, 0, 0, 255]));
        write_png(&mut pack, "assets/minecraft/textures/entity/explosion.png", &explosion).unwrap();
        write_png(&mut pack, "assets/minecraft/textures/entity/sweep.png", &solid(128, 32, 60))
            .unwrap();
        pack.insert("assets/minecraft/textures/item/rose_red.png", vec![1]);
        pack.insert("assets/minecraft/textures/item/ink_sac.png", vec![2]);
        pack.insert(
            "assets/minecraft/models/item/rose_red.json",
            serde_json::to_vec(
                &json!({"parent": "item/generated", "textures": {"layer0": "item/rose_red"}}),
            )
            .unwrap(),
        );
        pack.insert(
            "assets/minecraft/lang/en_us.json",
            serde_json::to_vec(&json!({"item.minecraft.rose_red": "Rose"})).unwrap(),
        );
        pack
    }

    #[test]
    fn upgrade_slices_all_sheets() {
        let mut pack = old_pack();
        V1_14.upgrade(&mut pack, &mut ctx()).unwrap();
        let t = "assets/minecraft/textures";
        assert!(!pack.contains(&format!("{t}/painting/paintings_kristoffer_zetterstrand.png")));
        assert_eq!(png(&pack, &format!("{t}/painting/back.png")).get_pixel(0, 0)[0], 20);
        assert_eq!(png(&pack, &format!("{t}/painting/kebab.png")).get_pixel(15, 15)[0], 10);
        assert_eq!(png(&pack, &format!("{t}/mob_effect/speed.png")).dimensions(), (18, 18));
        let inventory = png(&pack, &format!("{t}/gui/container/inventory.png"));
        assert_eq!(inventory.get_pixel(0, 198)[3], 0);
        assert_eq!(inventory.get_pixel(100, 100), &Rgba([1, 2, 3, 255]));
        assert_eq!(png(&pack, &format!("{t}/particle/generic_0.png")).get_pixel(0, 0)[0], 40);
        assert_eq!(png(&pack, &format!("{t}/entity/fishing_hook.png")).get_pixel(0, 0)[0], 50);
        assert!(!pack.contains(&format!("{t}/particle/particles.png")));
        assert_eq!(png(&pack, &format!("{t}/particle/explosion_5.png")).get_pixel(3, 3)[0], 5);
        let sweep = png(&pack, &format!("{t}/particle/sweep_0.png"));
        assert_eq!(
            (sweep.dimensions(), sweep.get_pixel(0, 7)[3], sweep.get_pixel(0, 8)[3]),
            ((32, 32), 0, 255)
        );
        assert!(!pack.contains(&format!("{t}/entity/sweep.png")));
        // Renames: moved, copied, referenced and translated.
        assert!(
            pack.contains(&format!("{t}/item/red_dye.png"))
                && !pack.contains(&format!("{t}/item/rose_red.png"))
        );
        assert!(
            pack.contains(&format!("{t}/item/ink_sac.png"))
                && pack.contains(&format!("{t}/item/black_dye.png"))
        );
        let model = pack.get("assets/minecraft/models/item/red_dye.json").expect("model moved");
        assert!(String::from_utf8_lossy(model).contains("item/red_dye"));
        let lang = pack.get("assets/minecraft/lang/en_us.json").unwrap();
        assert!(String::from_utf8_lossy(lang).contains("item.minecraft.red_dye"));
    }

    #[test]
    fn roundtrip_restores_the_sheets() {
        let original = old_pack();
        let mut pack = original.clone();
        V1_14.upgrade(&mut pack, &mut ctx()).unwrap();
        let mut ctx = ctx();
        V1_14.downgrade(&mut pack, &mut ctx).unwrap();
        let t = "assets/minecraft/textures";
        for sheet in [
            "painting/paintings_kristoffer_zetterstrand.png",
            "gui/container/inventory.png",
            "particle/particles.png",
            "entity/explosion.png",
            "entity/sweep.png",
        ] {
            let path = format!("{t}/{sheet}");
            assert_eq!(png(&original, &path), png(&pack, &path), "{sheet}");
        }
        assert!(
            pack.contains(&format!("{t}/item/rose_red.png"))
                && !pack.contains(&format!("{t}/item/black_dye.png"))
        );
        assert!(!pack.contains(&format!("{t}/particle/generic_0.png")));
        assert!(ctx.into_warnings().is_empty());
    }

    #[test]
    fn downgrade_warns_when_a_sheet_is_rebuilt_from_a_partial_set() {
        let mut pack = Pack::new();
        write_png(&mut pack, "assets/minecraft/textures/particle/generic_0.png", &solid(8, 8, 1))
            .unwrap();
        let mut ctx = ctx();
        V1_14.downgrade(&mut pack, &mut ctx).unwrap();
        let sheet = png(&pack, "assets/minecraft/textures/particle/particles.png");
        assert_eq!(sheet.dimensions(), (256, 256));
        assert_eq!(sheet.get_pixel(7, 7)[0], 1);
        assert!(ctx.into_warnings().iter().any(|w| w.starts_with("particle sprites")));
    }

    #[test]
    fn vendored_definitions_are_consistent() {
        let counts: Vec<usize> =
            groups().iter().map(|g| g.sheets.iter().map(|s| s.sprites.len()).sum()).collect();
        assert_eq!(counts, [27, 27, 91, 24]);
    }

    #[test]
    fn warns_about_old_villager_textures() {
        let mut pack = Pack::new();
        pack.insert("assets/minecraft/textures/entity/villager/farmer.png", vec![1]);
        let mut ctx = ctx();
        V1_14.upgrade(&mut pack, &mut ctx).unwrap();
        assert_eq!(ctx.into_warnings().len(), 1);
    }
}

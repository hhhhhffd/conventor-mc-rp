//! 1.21.5: farm animals got temperate/cold/warm variants (the old textures became the
//! temperate ones), sheep wool was renamed, and the entity glint became the armor glint.
//!
//! Not converted: saddle textures (their layout changed and they moved into equipment assets),
//! spawn eggs (one texture per egg now) and the removed shader program JSON files.

use super::common::{move_textures, reversed};
use crate::error::Result;
use crate::step::{Context, Step};
use crate::version::Version;
use crate::vfs::Pack;

const TEXTURES: &[(&str, &str)] = &[
    ("entity/pig/pig.png", "entity/pig/temperate_pig.png"),
    ("entity/cow/cow.png", "entity/cow/temperate_cow.png"),
    ("entity/chicken.png", "entity/chicken/temperate_chicken.png"),
    ("entity/sheep/sheep_fur.png", "entity/sheep/sheep_wool.png"),
    ("misc/enchanted_glint_entity.png", "misc/enchanted_glint_armor.png"),
];

const SADDLES: &[&str] = &[
    "assets/minecraft/textures/entity/pig/pig_saddle.png",
    "assets/minecraft/textures/entity/strider/strider_saddle.png",
];

pub struct V1_21_5;

impl Step for V1_21_5 {
    fn name(&self) -> &'static str {
        "1.21.5 animal variants"
    }

    fn version(&self) -> Version {
        Version::new(1, 21, 5)
    }

    fn upgrade(&self, pack: &mut Pack, ctx: &mut Context) -> Result<()> {
        move_textures(pack, TEXTURES);
        if SADDLES.iter().any(|p| pack.contains(p)) {
            ctx.warn("pig and strider saddle textures changed layout in 1.21.5 and were not converted");
        }
        Ok(())
    }

    fn downgrade(&self, pack: &mut Pack, _ctx: &mut Context) -> Result<()> {
        move_textures(pack, &reversed(TEXTURES));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renames_variants_with_mcmeta_and_back() {
        let mut pack = Pack::new();
        let mut ctx = Context::new(Version::new(1, 21, 4), Version::new(1, 21, 5));
        pack.insert("assets/minecraft/textures/entity/chicken.png", vec![1]);
        pack.insert("assets/minecraft/textures/misc/enchanted_glint_entity.png", vec![2]);
        pack.insert("assets/minecraft/textures/misc/enchanted_glint_entity.png.mcmeta", vec![3]);
        V1_21_5.upgrade(&mut pack, &mut ctx).unwrap();
        assert!(pack.contains("assets/minecraft/textures/entity/chicken/temperate_chicken.png"));
        assert!(pack.contains("assets/minecraft/textures/misc/enchanted_glint_armor.png.mcmeta"));
        V1_21_5.downgrade(&mut pack, &mut ctx).unwrap();
        assert!(pack.contains("assets/minecraft/textures/entity/chicken.png"));
        assert!(pack.contains("assets/minecraft/textures/misc/enchanted_glint_entity.png"));
    }
}

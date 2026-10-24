//! 1.16: zombie pigmen became zombified piglins.

use crate::error::Result;
use crate::step::{Context, Step};
use crate::version::Version;
use crate::vfs::Pack;

use super::rename::Renames;

// The entity texture is not renamed: the zombified piglin uses the piglin model, whose texture
// layout differs from the zombie pigman's.
const RENAMES: Renames = Renames {
    models: &[("item/zombie_pigman_spawn_egg", "item/zombified_piglin_spawn_egg")],
    lang: &[
        ("entity.minecraft.zombie_pigman", "entity.minecraft.zombified_piglin"),
        ("item.minecraft.zombie_pigman_spawn_egg", "item.minecraft.zombified_piglin_spawn_egg"),
        ("subtitles.entity.zombie_pigman.ambient", "subtitles.entity.zombified_piglin.ambient"),
        ("subtitles.entity.zombie_pigman.angry", "subtitles.entity.zombified_piglin.angry"),
        ("subtitles.entity.zombie_pigman.death", "subtitles.entity.zombified_piglin.death"),
        ("subtitles.entity.zombie_pigman.hurt", "subtitles.entity.zombified_piglin.hurt"),
    ],
    ..Renames::NONE
};

pub struct V1_16;

impl Step for V1_16 {
    fn name(&self) -> &'static str {
        "1.16 zombified piglin renames"
    }

    fn version(&self) -> Version {
        Version::new(1, 16, 0)
    }

    fn upgrade(&self, pack: &mut Pack, ctx: &mut Context) -> Result<()> {
        RENAMES.upgrade(pack, ctx);
        Ok(())
    }

    fn downgrade(&self, pack: &mut Pack, ctx: &mut Context) -> Result<()> {
        RENAMES.downgrade(pack, ctx);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lang(pack: &Pack) -> String {
        String::from_utf8(pack.get("assets/minecraft/lang/en_us.json").unwrap().to_vec()).unwrap()
    }

    #[test]
    fn renames_spawn_egg_model_and_lang_keys_both_ways() {
        let mut pack = Pack::new();
        let ctx = &mut Context::new(Version::new(1, 15, 0), Version::new(1, 16, 0));
        pack.insert(
            "assets/minecraft/models/item/zombie_pigman_spawn_egg.json",
            br#"{"parent":"item/template_spawn_egg"}"#.to_vec(),
        );
        pack.insert(
            "assets/minecraft/lang/en_us.json",
            br#"{"entity.minecraft.zombie_pigman":"Pig"}"#.to_vec(),
        );
        V1_16.upgrade(&mut pack, ctx).unwrap();
        assert!(pack.contains("assets/minecraft/models/item/zombified_piglin_spawn_egg.json"));
        assert!(
            lang(&pack).contains("entity.minecraft.zombified_piglin")
                && !lang(&pack).contains("zombie_pigman")
        );
        V1_16.downgrade(&mut pack, ctx).unwrap();
        assert!(pack.contains("assets/minecraft/models/item/zombie_pigman_spawn_egg.json"));
        assert!(lang(&pack).contains("entity.minecraft.zombie_pigman"));
    }
}

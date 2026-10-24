//! 1.21.2: armor, elytra, horse/llama/wolf armor and armor trim textures moved under
//! `textures/entity/equipment/` and `textures/trims/entity/` (table in `data/modern/equipment-1.21.2.tsv`).

use super::common::{move_textures, parse_pairs, reversed};
use crate::error::Result;
use crate::step::{Context, Step};
use crate::version::Version;
use crate::vfs::Pack;

const TABLE: &str = include_str!("../../../data/modern/equipment-1.21.2.tsv");

pub struct V1_21_2;

impl Step for V1_21_2 {
    fn name(&self) -> &'static str {
        "1.21.2 equipment textures"
    }

    fn version(&self) -> Version {
        Version::new(1, 21, 2)
    }

    fn upgrade(&self, pack: &mut Pack, _ctx: &mut Context) -> Result<()> {
        move_textures(pack, &parse_pairs(TABLE));
        Ok(())
    }

    fn downgrade(&self, pack: &mut Pack, ctx: &mut Context) -> Result<()> {
        move_textures(pack, &reversed(&parse_pairs(TABLE)));
        let has_definitions = pack.paths().any(|p| p.starts_with("assets/") && p.contains("/models/equipment/"));
        if has_definitions {
            ctx.warn("equipment model definitions (models/equipment) do not exist before 1.21.2; custom armor layers were not converted");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::steps::modern::common::testing::{put_png, solid};

    #[test]
    fn moves_armor_layers_and_back() {
        let mut pack = Pack::new();
        let mut ctx = Context::new(Version::new(1, 21, 1), Version::new(1, 21, 2));
        let old = "assets/minecraft/textures/models/armor/leather_layer_1_overlay.png";
        put_png(&mut pack, old, &solid(64, 32, [1, 1, 1, 255]));
        pack.insert("assets/minecraft/textures/models/armor/iron_layer_2.png.mcmeta", b"{}".to_vec());
        pack.insert("assets/minecraft/textures/models/armor/iron_layer_2.png", vec![1]);
        V1_21_2.upgrade(&mut pack, &mut ctx).unwrap();
        assert!(pack.contains("assets/minecraft/textures/entity/equipment/humanoid/leather_overlay.png"));
        assert!(pack.contains("assets/minecraft/textures/entity/equipment/humanoid_leggings/iron.png.mcmeta"));
        assert!(!pack.contains(old));
        V1_21_2.downgrade(&mut pack, &mut ctx).unwrap();
        assert!(pack.contains(old));
        assert!(pack.contains("assets/minecraft/textures/models/armor/iron_layer_2.png"));
    }

    #[test]
    fn table_has_no_collisions() {
        let pairs = parse_pairs(TABLE);
        let mut targets: Vec<_> = pairs.iter().map(|p| p.1).collect();
        targets.sort_unstable();
        targets.dedup();
        assert_eq!(targets.len(), pairs.len());
    }
}

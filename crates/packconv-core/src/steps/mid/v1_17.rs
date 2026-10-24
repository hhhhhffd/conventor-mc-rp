//! 1.17: grass path became dirt path, cauldrons split by content, squid got a folder.

use crate::error::Result;
use crate::step::{Context, Step};
use crate::version::Version;
use crate::vfs::Pack;

use super::rename::Renames;

// Section "1_17" of data/rpc/forwards/{blocks,items}.json, split into what each name refers
// to. Water cauldron models are the only cauldron change that reaches pack files; the empty
// cauldron keeps its name.
const RENAMES: Renames = Renames {
    textures: &[
        ("block/grass_path_top", "block/dirt_path_top"),
        ("block/grass_path_side", "block/dirt_path_side"),
        ("entity/squid", "entity/squid/squid"),
    ],
    models: &[
        ("block/grass_path", "block/dirt_path"),
        ("item/grass_path", "item/dirt_path"),
        ("block/cauldron_level1", "block/water_cauldron_level1"),
        ("block/cauldron_level2", "block/water_cauldron_level2"),
        ("block/cauldron_level3", "block/water_cauldron_full"),
    ],
    blockstates: &[("grass_path", "dirt_path")],
    lang: &[("block.minecraft.grass_path", "block.minecraft.dirt_path")],
    ..Renames::NONE
};

pub struct V1_17;

impl Step for V1_17 {
    fn name(&self) -> &'static str {
        "1.17 dirt path, water cauldron and squid renames"
    }

    fn version(&self) -> Version {
        Version::new(1, 17, 0)
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

    fn pack() -> Pack {
        let mut pack = Pack::new();
        pack.insert("assets/minecraft/textures/block/grass_path_top.png", vec![1]);
        pack.insert("assets/minecraft/textures/block/grass_path_top.png.mcmeta", vec![2]);
        pack.insert("assets/minecraft/textures/entity/squid.png", vec![3]);
        pack.insert(
            "assets/minecraft/models/block/grass_path.json",
            br#"{"textures":{"top":"block/grass_path_top"}}"#.to_vec(),
        );
        pack.insert("assets/minecraft/models/block/cauldron_level3.json", b"{}".to_vec());
        pack.insert(
            "assets/minecraft/blockstates/grass_path.json",
            br#"{"variants":{"":{"model":"block/grass_path"}}}"#.to_vec(),
        );
        pack.insert(
            "assets/mymod/models/item/x.json",
            br#"{"parent":"minecraft:block/grass_path"}"#.to_vec(),
        );
        pack
    }

    fn text(pack: &Pack, path: &str) -> String {
        String::from_utf8(pack.get(path).unwrap().to_vec()).unwrap()
    }

    #[test]
    fn upgrade_and_downgrade_are_inverse() {
        let original = pack();
        let mut pack = pack();
        let ctx = &mut Context::new(Version::new(1, 16, 5), Version::new(1, 17, 0));
        V1_17.upgrade(&mut pack, ctx).unwrap();
        for path in [
            "assets/minecraft/textures/block/dirt_path_top.png",
            "assets/minecraft/textures/block/dirt_path_top.png.mcmeta",
            "assets/minecraft/textures/entity/squid/squid.png",
            "assets/minecraft/models/block/dirt_path.json",
            "assets/minecraft/models/block/water_cauldron_full.json",
            "assets/minecraft/blockstates/dirt_path.json",
        ] {
            assert!(pack.contains(path), "{path}");
        }
        assert!(text(&pack, "assets/minecraft/models/block/dirt_path.json")
            .contains("block/dirt_path_top"));
        assert!(text(&pack, "assets/minecraft/blockstates/dirt_path.json")
            .contains("block/dirt_path\""));
        assert!(
            text(&pack, "assets/mymod/models/item/x.json").contains("minecraft:block/dirt_path")
        );

        V1_17.downgrade(&mut pack, ctx).unwrap();
        assert_eq!(pack.paths().collect::<Vec<_>>(), original.paths().collect::<Vec<_>>());
        for path in original.paths().filter(|p| p.ends_with(".json")) {
            let parse = |p: &Pack| {
                serde_json::from_slice::<serde_json::Value>(p.get(path).unwrap()).unwrap()
            };
            assert_eq!(parse(&pack), parse(&original), "{path}");
        }
    }
}

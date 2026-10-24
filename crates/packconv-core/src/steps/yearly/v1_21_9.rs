//! 1.21.9: the chain block and item became `iron_chain` (textures, blockstate, item
//! definition and models) to make room for the copper chains.

use super::common::{asset_files, directed, edit_json_files, move_files, Pairs, RefMap};
use crate::error::Result;
use crate::step::{Context, Step};
use crate::version::Version;
use crate::vfs::Pack;

const FILES: &Pairs = &[
    ("textures/block/chain.png", "textures/block/iron_chain.png"),
    ("textures/item/chain.png", "textures/item/iron_chain.png"),
    ("blockstates/chain.json", "blockstates/iron_chain.json"),
    ("items/chain.json", "items/iron_chain.json"),
    ("models/block/chain.json", "models/block/iron_chain.json"),
    ("models/item/chain.json", "models/item/iron_chain.json"),
];

/// Model and texture ids share these names, so one table rewrites both kinds of reference.
const REFS: &Pairs = &[("block/chain", "block/iron_chain"), ("item/chain", "item/iron_chain")];

pub struct IronChain;

fn convert(pack: &mut Pack, ctx: &mut Context, up: bool) {
    move_files(pack, ctx, &directed(FILES, up));
    let refs = RefMap::new(&directed(REFS, up));
    for dir in ["blockstates", "models", "items"] {
        let files = asset_files(pack, dir, &[".json"]);
        edit_json_files(pack, ctx, files, |json| refs.rewrite_all(json));
    }
}

impl Step for IronChain {
    fn name(&self) -> &'static str {
        "chain <-> iron_chain"
    }

    fn version(&self) -> Version {
        Version::new(1, 21, 9)
    }

    fn upgrade(&self, pack: &mut Pack, ctx: &mut Context) -> Result<()> {
        convert(pack, ctx, true);
        Ok(())
    }

    fn downgrade(&self, pack: &mut Pack, ctx: &mut Context) -> Result<()> {
        convert(pack, ctx, false);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::steps::yearly::common::testing::*;

    fn sample() -> Pack {
        let mut pack = Pack::new();
        pack.insert("assets/minecraft/textures/block/chain.png", vec![1]);
        pack.insert("assets/minecraft/textures/block/chain.png.mcmeta", vec![2]);
        pack.insert("assets/minecraft/textures/item/chain.png", vec![3]);
        pack.insert("assets/minecraft/blockstates/chain.json", br#"{"variants":{"axis=y":{"model":"minecraft:block/chain"}}}"#.to_vec());
        pack.insert("assets/minecraft/items/chain.json", br#"{"model":{"type":"minecraft:model","model":"minecraft:item/chain"}}"#.to_vec());
        pack.insert("assets/minecraft/models/block/chain.json", br##"{"textures":{"all":"block/chain","particle":"#all"}}"##.to_vec());
        pack.insert("assets/minecraft/models/item/chain.json", br#"{"parent":"item/generated","textures":{"layer0":"minecraft:item/chain"}}"#.to_vec());
        pack.insert("assets/other/models/block/x.json", br#"{"textures":{"a":"other:block/chain"}}"#.to_vec());
        pack
    }

    #[test]
    fn upgrade_renames_files_and_references() {
        let mut pack = sample();
        let mut ctx = context();
        IronChain.upgrade(&mut pack, &mut ctx).unwrap();
        for path in [
            "textures/block/iron_chain.png",
            "textures/block/iron_chain.png.mcmeta",
            "textures/item/iron_chain.png",
            "blockstates/iron_chain.json",
            "items/iron_chain.json",
            "models/block/iron_chain.json",
            "models/item/iron_chain.json",
        ] {
            assert!(pack.contains(&format!("assets/minecraft/{path}")), "{path}");
        }
        assert!(!pack.contains("assets/minecraft/textures/block/chain.png"));
        let state = String::from_utf8_lossy(pack.get("assets/minecraft/blockstates/iron_chain.json").unwrap()).into_owned();
        assert!(state.contains("minecraft:block/iron_chain"));
        let model = String::from_utf8_lossy(pack.get("assets/minecraft/models/block/iron_chain.json").unwrap()).into_owned();
        assert!(model.contains("block/iron_chain") && model.contains("#all"));
        let other = String::from_utf8_lossy(pack.get("assets/other/models/block/x.json").unwrap()).into_owned();
        assert!(other.contains("other:block/chain"));
        assert!(ctx.into_warnings().is_empty());
    }

    #[test]
    fn roundtrip_restores_pack() {
        let original = sample();
        let mut pack = original.clone();
        let mut ctx = context();
        IronChain.upgrade(&mut pack, &mut ctx).unwrap();
        IronChain.downgrade(&mut pack, &mut ctx).unwrap();
        for path in original.paths() {
            let after = pack.get(path).unwrap_or_else(|| panic!("lost {path}"));
            assert!(same_content(original.get(path).unwrap(), after), "{path}");
        }
        assert_eq!(pack.len(), original.len());
    }

    #[test]
    fn warns_about_bad_json() {
        let mut pack = Pack::new();
        pack.insert("assets/minecraft/models/block/broken.json", b"{".to_vec());
        let mut ctx = context();
        IronChain.upgrade(&mut pack, &mut ctx).unwrap();
        assert_eq!(ctx.into_warnings().len(), 1);
    }
}

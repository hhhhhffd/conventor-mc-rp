//! 1.19: the generic door models got explicit left and right hinge names.

use crate::error::Result;
use crate::step::{Context, Step};
use crate::version::Version;
use crate::vfs::Pack;

use super::rename::Renames;

// Section "1_19" of data/rpc/forwards/blocks.json. These are the parent models that hold the
// door geometry. The per-wood wrappers and the new `*_open` models are left to the game.
const RENAMES: Renames = Renames {
    models: &[
        ("block/door_bottom", "block/door_bottom_left"),
        ("block/door_bottom_rh", "block/door_bottom_right"),
        ("block/door_top", "block/door_top_left"),
        ("block/door_top_rh", "block/door_top_right"),
    ],
    ..Renames::NONE
};

pub struct V1_19;

impl Step for V1_19 {
    fn name(&self) -> &'static str {
        "1.19 door model renames"
    }

    fn version(&self) -> Version {
        Version::new(1, 19, 0)
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

    fn wrapper(pack: &Pack) -> String {
        String::from_utf8(
            pack.get("assets/minecraft/models/block/oak_door_top_hinge.json").unwrap().to_vec(),
        )
        .unwrap()
    }

    #[test]
    fn door_parents_are_renamed_and_referenced_both_ways() {
        let mut pack = Pack::new();
        let ctx = &mut Context::new(Version::new(1, 18, 2), Version::new(1, 19, 0));
        pack.insert("assets/minecraft/models/block/door_top_rh.json", b"{}".to_vec());
        pack.insert(
            "assets/minecraft/models/block/oak_door_top_hinge.json",
            br#"{"parent":"minecraft:block/door_top_rh"}"#.to_vec(),
        );
        V1_19.upgrade(&mut pack, ctx).unwrap();
        assert!(pack.contains("assets/minecraft/models/block/door_top_right.json"));
        assert!(wrapper(&pack).contains("minecraft:block/door_top_right"));
        V1_19.downgrade(&mut pack, ctx).unwrap();
        assert!(pack.contains("assets/minecraft/models/block/door_top_rh.json"));
        assert!(wrapper(&pack).contains("minecraft:block/door_top_rh"));
    }
}

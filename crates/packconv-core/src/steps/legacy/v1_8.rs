//! 1.8 introduced block and item models and blockstates. Textures kept their names.

use crate::error::Result;
use crate::step::{Context, Step};
use crate::util::namespaces;
use crate::version::Version;
use crate::vfs::Pack;

use super::common::drop_added_files;

pub struct V1_8;

impl Step for V1_8 {
    fn name(&self) -> &'static str {
        "legacy-1.8"
    }

    fn version(&self) -> Version {
        Version::new(1, 8, 0)
    }

    /// A 1.7 pack has no models or blockstates, and its texture names are still valid.
    fn upgrade(&self, _pack: &mut Pack, _ctx: &mut Context) -> Result<()> {
        Ok(())
    }

    fn downgrade(&self, pack: &mut Pack, ctx: &mut Context) -> Result<()> {
        let mut removed = 0;
        for namespace in namespaces(pack) {
            for dir in ["models", "blockstates"] {
                for path in pack.paths_with_prefix(&format!("assets/{namespace}/{dir}/")) {
                    pack.remove(&path);
                    removed += 1;
                }
            }
        }
        if removed > 0 {
            ctx.warn(format!(
                "Minecraft 1.7 has no models or blockstates: removed {removed} files"
            ));
        }
        drop_added_files(pack, ctx, "1.8");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::steps::legacy::common::testing::*;

    #[test]
    fn downgrade_drops_models_blockstates_and_new_textures() {
        let mut pack = Pack::new();
        put_json(&mut pack, "assets/minecraft/models/block/stone.json", "{}");
        put_json(&mut pack, "assets/mod/models/item/x.json", "{}");
        put_json(&mut pack, "assets/minecraft/blockstates/stone.json", "{}");
        put_json(&mut pack, "assets/mod/blockstates/x.json", "{}");
        put_png(
            &mut pack,
            "assets/minecraft/textures/blocks/stone.png",
            &solid(1, 1, [1; 4]),
        );
        put_png(
            &mut pack,
            "assets/minecraft/textures/entity/rabbit/black.png",
            &solid(1, 1, [2; 4]),
        );

        let mut ctx = Context::new(Version::new(1, 8, 0), Version::new(1, 7, 10));
        V1_8.downgrade(&mut pack, &mut ctx).unwrap();

        let left: Vec<&str> = pack.paths().collect();
        assert_eq!(left, ["assets/minecraft/textures/blocks/stone.png"]);
        assert_eq!(ctx.into_warnings().len(), 2);
    }

    #[test]
    fn roundtrip_keeps_textures() {
        let mut pack = Pack::new();
        put_png(
            &mut pack,
            "assets/minecraft/textures/blocks/stone.png",
            &solid(1, 1, [1; 4]),
        );
        let before = pack
            .get("assets/minecraft/textures/blocks/stone.png")
            .unwrap()
            .to_vec();

        let mut ctx = Context::new(Version::new(1, 7, 10), Version::new(1, 8, 0));
        V1_8.upgrade(&mut pack, &mut ctx).unwrap();
        V1_8.downgrade(&mut pack, &mut ctx).unwrap();

        assert_eq!(
            pack.get("assets/minecraft/textures/blocks/stone.png")
                .unwrap(),
            before
        );
        assert!(ctx.into_warnings().is_empty());
    }
}

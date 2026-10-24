//! 1.10 only added content (magma, bone and nether wart blocks, stray, husk, polar bear...);
//! no existing resource changed its layout.

use crate::error::Result;
use crate::step::{Context, Step};
use crate::version::Version;
use crate::vfs::Pack;

use super::common::drop_added_files;

pub struct V1_10;

impl Step for V1_10 {
    fn name(&self) -> &'static str {
        "legacy-1.10"
    }

    fn version(&self) -> Version {
        Version::new(1, 10, 0)
    }

    fn upgrade(&self, _pack: &mut Pack, _ctx: &mut Context) -> Result<()> {
        Ok(())
    }

    fn downgrade(&self, pack: &mut Pack, ctx: &mut Context) -> Result<()> {
        drop_added_files(pack, ctx, "1.10");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::steps::legacy::common::testing::*;

    #[test]
    fn downgrade_drops_textures_added_in_1_10_only() {
        let mut pack = Pack::new();
        put_png(
            &mut pack,
            "assets/minecraft/textures/blocks/magma.png",
            &solid(1, 1, [1; 4]),
        );
        put_json(
            &mut pack,
            "assets/minecraft/textures/blocks/magma.png.mcmeta",
            "{}",
        );
        put_png(
            &mut pack,
            "assets/minecraft/textures/blocks/stone.png",
            &solid(1, 1, [2; 4]),
        );

        let mut ctx = Context::new(Version::new(1, 10, 0), Version::new(1, 9, 0));
        V1_10.downgrade(&mut pack, &mut ctx).unwrap();

        let left: Vec<&str> = pack.paths().collect();
        assert_eq!(left, ["assets/minecraft/textures/blocks/stone.png"]);
        assert_eq!(ctx.into_warnings().len(), 1);
    }

    #[test]
    fn upgrade_changes_nothing() {
        let mut pack = Pack::new();
        put_png(
            &mut pack,
            "assets/minecraft/textures/blocks/stone.png",
            &solid(1, 1, [2; 4]),
        );
        let mut ctx = Context::new(Version::new(1, 9, 0), Version::new(1, 10, 0));
        V1_10.upgrade(&mut pack, &mut ctx).unwrap();
        assert_eq!(pack.len(), 1);
    }
}

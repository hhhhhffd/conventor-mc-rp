//! 1.12 added concrete, glazed terracotta, parrots, the recipe book and advancement GUI
//! art. Beds became block entities drawn from `textures/entity/bed/<color>.png`.

use crate::error::Result;
use crate::step::{Context, Step};
use crate::version::Version;
use crate::vfs::Pack;

use super::common::{drop_added_files, MC};

const OLD_BED_TEXTURES: [&str; 6] = [
    "bed_feet_end",
    "bed_feet_side",
    "bed_feet_top",
    "bed_head_end",
    "bed_head_side",
    "bed_head_top",
];

pub struct V1_12;

impl Step for V1_12 {
    fn name(&self) -> &'static str {
        "legacy-1.12"
    }

    fn version(&self) -> Version {
        Version::new(1, 12, 0)
    }

    /// The 1.11 bed was a block model built from six textures, the 1.12 bed is an entity
    /// with one texture per color and a different UV layout. Re-mapping the pixels
    /// between the two is not possible without guessing, so the old art is kept unused.
    fn upgrade(&self, pack: &mut Pack, ctx: &mut Context) -> Result<()> {
        let has_old_bed = OLD_BED_TEXTURES
            .iter()
            .any(|name| pack.contains(&format!("{MC}/textures/blocks/{name}.png")));
        if has_old_bed {
            ctx.warn("bed textures use a different layout since 1.12 and were not converted");
        }
        Ok(())
    }

    fn downgrade(&self, pack: &mut Pack, ctx: &mut Context) -> Result<()> {
        if !pack
            .paths_with_prefix(&format!("{MC}/textures/entity/bed/"))
            .is_empty()
        {
            ctx.warn("bed textures use a different layout before 1.12 and were not converted");
        }
        drop_added_files(pack, ctx, "1.12");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::steps::legacy::common::testing::*;

    #[test]
    fn downgrade_drops_1_12_textures_and_warns_about_beds() {
        let mut pack = Pack::new();
        put_png(
            &mut pack,
            "assets/minecraft/textures/entity/bed/red.png",
            &solid(1, 1, [1; 4]),
        );
        put_png(
            &mut pack,
            "assets/minecraft/textures/blocks/concrete_red.png",
            &solid(1, 1, [2; 4]),
        );
        put_png(
            &mut pack,
            "assets/minecraft/textures/blocks/stone.png",
            &solid(1, 1, [3; 4]),
        );

        let mut ctx = Context::new(Version::new(1, 12, 0), Version::new(1, 11, 0));
        V1_12.downgrade(&mut pack, &mut ctx).unwrap();

        let left: Vec<&str> = pack.paths().collect();
        assert_eq!(left, ["assets/minecraft/textures/blocks/stone.png"]);
        assert_eq!(ctx.into_warnings().len(), 2);
    }

    #[test]
    fn upgrade_warns_about_old_beds_and_keeps_files() {
        let mut pack = Pack::new();
        put_png(
            &mut pack,
            "assets/minecraft/textures/blocks/bed_head_top.png",
            &solid(1, 1, [1; 4]),
        );

        let mut ctx = Context::new(Version::new(1, 11, 0), Version::new(1, 12, 0));
        V1_12.upgrade(&mut pack, &mut ctx).unwrap();

        assert!(pack.contains("assets/minecraft/textures/blocks/bed_head_top.png"));
        assert_eq!(ctx.into_warnings().len(), 1);
    }

    #[test]
    fn upgrade_without_beds_is_silent() {
        let mut pack = Pack::new();
        put_png(
            &mut pack,
            "assets/minecraft/textures/blocks/stone.png",
            &solid(1, 1, [1; 4]),
        );
        let mut ctx = Context::new(Version::new(1, 11, 0), Version::new(1, 12, 0));
        V1_12.upgrade(&mut pack, &mut ctx).unwrap();
        assert!(ctx.into_warnings().is_empty());
    }
}

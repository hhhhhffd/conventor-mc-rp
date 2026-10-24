//! 1.20.3: `grass` (the short plant) was renamed to `short_grass`, and non-PNG textures stopped loading.

use super::common::{rename_ids, reversed};
use crate::error::Result;
use crate::step::{Context, Step};
use crate::version::Version;
use crate::vfs::Pack;

const GRASS: &[(&str, &str)] = &[("block/grass", "block/short_grass"), ("item/grass", "item/short_grass")];

/// Extensions the pre-1.20.3 texture loader accepted besides PNG.
const LEGACY_IMAGE_EXTENSIONS: &[&str] = &[".jpg", ".jpeg", ".raw", ".ico", ".bmp"];

pub struct V1_20_3;

impl Step for V1_20_3 {
    fn name(&self) -> &'static str {
        "1.20.3 short_grass"
    }

    fn version(&self) -> Version {
        Version::new(1, 20, 3)
    }

    fn upgrade(&self, pack: &mut Pack, ctx: &mut Context) -> Result<()> {
        rename_ids(pack, ctx, GRASS);
        // Only PNG decoding is available in the core, so these cannot be converted.
        let has_legacy_images = pack.paths().any(|p| {
            let p = p.to_ascii_lowercase();
            p.starts_with("assets/") && p.contains("/textures/") && LEGACY_IMAGE_EXTENSIONS.iter().any(|ext| p.ends_with(ext))
        });
        if has_legacy_images {
            ctx.warn("textures in jpg/jpeg/raw/ico/bmp format are not loaded since 1.20.3 and were not converted to PNG");
        }
        Ok(())
    }

    fn downgrade(&self, pack: &mut Pack, ctx: &mut Context) -> Result<()> {
        rename_ids(pack, ctx, &reversed(GRASS));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::steps::modern::common::testing::{put_png, solid};

    fn ctx() -> Context {
        Context::new(Version::new(1, 20, 2), Version::new(1, 20, 3))
    }

    fn text(pack: &Pack, path: &str) -> String {
        String::from_utf8(pack.get(path).unwrap().to_vec()).unwrap()
    }

    #[test]
    fn renames_grass_everywhere_and_back() {
        let mut pack = Pack::new();
        let mut ctx = ctx();
        put_png(&mut pack, "assets/minecraft/textures/block/grass.png", &solid(1, 1, [0, 255, 0, 255]));
        pack.insert("assets/minecraft/blockstates/grass.json", br#"{"variants":{"":{"model":"block/grass"}}}"#.to_vec());
        pack.insert("assets/minecraft/models/block/grass.json", br#"{"textures":{"cross":"minecraft:block/grass"}}"#.to_vec());
        pack.insert("assets/custom/models/block/x.json", br#"{"parent":"minecraft:block/grass"}"#.to_vec());
        V1_20_3.upgrade(&mut pack, &mut ctx).unwrap();
        assert!(pack.contains("assets/minecraft/textures/block/short_grass.png"));
        assert!(pack.contains("assets/minecraft/models/block/short_grass.json"));
        assert!(text(&pack, "assets/minecraft/blockstates/short_grass.json").contains("block/short_grass"));
        assert!(text(&pack, "assets/custom/models/block/x.json").contains("minecraft:block/short_grass"));
        V1_20_3.downgrade(&mut pack, &mut ctx).unwrap();
        assert!(pack.contains("assets/minecraft/textures/block/grass.png"));
        assert!(pack.contains("assets/minecraft/models/block/grass.json"));
        assert!(!pack.contains("assets/minecraft/models/block/short_grass.json"));
    }

    #[test]
    fn warns_about_legacy_image_formats() {
        let mut pack = Pack::new();
        let mut ctx = ctx();
        pack.insert("assets/minecraft/textures/block/stone.JPG", vec![1]);
        V1_20_3.upgrade(&mut pack, &mut ctx).unwrap();
        assert_eq!(ctx.into_warnings().len(), 1);
    }
}

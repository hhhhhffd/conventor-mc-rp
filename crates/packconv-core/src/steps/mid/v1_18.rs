//! 1.18: the `barrier` and `light` particles merged into `block_marker`, and the inventory
//! sheet gained a small effect background.

use crate::error::Result;
use crate::step::{Context, Step};
use crate::steps::support::{save_png, Renamer};
use crate::version::Version;
use crate::vfs::Pack;

use super::sheet::{clear_area, copy_area, load_png, scale_factor, Flip};

const PARTICLES: &str = "assets/minecraft/particles";
const INVENTORY: &str = "assets/minecraft/textures/gui/container/inventory.png";
const INVENTORY_DESIGN: (u32, u32) = (256, 256);

/// The small effect background at (0,198) is the two 16x32 caps of the wide one at (0,166).
/// Each entry is `(source x, destination x)`; the 16x32 rows are at y 166 and 198.
const CAPS: [(u32, u32); 2] = [(0, 0), (104, 16)];

pub struct V1_18;

impl Step for V1_18 {
    fn name(&self) -> &'static str {
        "1.18 block marker particle and small effect background"
    }

    fn version(&self) -> Version {
        Version::new(1, 18, 0)
    }

    fn upgrade(&self, pack: &mut Pack, ctx: &mut Context) -> Result<()> {
        let marker = format!("{PARTICLES}/block_marker.json");
        let light = format!("{PARTICLES}/light.json");
        let mut renamer = Renamer::default();
        if !renamer.rename(pack, &format!("{PARTICLES}/barrier.json"), &marker) {
            renamer.rename(pack, &light, &marker);
        }
        renamer.report(ctx);
        pack.remove(&light);

        let Some(mut inventory) = load_png(pack, INVENTORY, ctx) else { return Ok(()) };
        let Some(k) = scale_factor(&inventory, INVENTORY_DESIGN) else {
            ctx.warn(format!(
                "{INVENTORY}: unexpected size {}x{}; left unconverted",
                inventory.width(),
                inventory.height()
            ));
            return Ok(());
        };
        let wide = inventory.clone();
        for (from_x, to_x) in CAPS {
            copy_area(
                &mut inventory,
                &wide,
                (from_x * k, 166 * k, 16 * k, 32 * k),
                (to_x * k, 198 * k),
                Flip::None,
            );
        }
        save_png(pack, ctx, INVENTORY, &inventory);
        Ok(())
    }

    fn downgrade(&self, pack: &mut Pack, ctx: &mut Context) -> Result<()> {
        // 1.17 has both particles, each reading its own definition.
        if let Some(marker) = pack.remove(&format!("{PARTICLES}/block_marker.json")) {
            let mut kept = false;
            for particle in ["barrier", "light"] {
                let path = format!("{PARTICLES}/{particle}.json");
                kept |= pack.contains(&path);
                if !pack.contains(&path) {
                    pack.insert(path, marker.clone());
                }
            }
            if kept {
                ctx.warn("an existing barrier or light particle definition was kept instead of block_marker");
            }
        }

        let Some(mut inventory) = load_png(pack, INVENTORY, ctx) else { return Ok(()) };
        let Some(k) = scale_factor(&inventory, INVENTORY_DESIGN) else {
            ctx.warn(format!(
                "{INVENTORY}: unexpected size {}x{}; left unconverted",
                inventory.width(),
                inventory.height()
            ));
            return Ok(());
        };
        // The small background was derived from the wide one, so dropping it loses nothing.
        clear_area(&mut inventory, (0, 198 * k, 32 * k, 32 * k));
        save_png(pack, ctx, INVENTORY, &inventory);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::{read_png, write_png};
    use image::{Rgba, RgbaImage};

    fn ctx() -> Context {
        Context::new(Version::new(1, 17, 1), Version::new(1, 18, 0))
    }

    fn inventory(k: u32) -> RgbaImage {
        RgbaImage::from_fn(256 * k, 256 * k, |x, y| {
            if y >= 166 * k && y < 198 * k && x < 120 * k {
                Rgba([(x / k) as u8, (y / k - 166) as u8, 9, 255])
            } else {
                Rgba([0, 0, 0, 0])
            }
        })
    }

    #[test]
    fn particles_are_merged_and_split_again() {
        let mut pack = Pack::new();
        pack.insert("assets/minecraft/particles/barrier.json", b"barrier".to_vec());
        pack.insert("assets/minecraft/particles/light.json", b"light".to_vec());
        V1_18.upgrade(&mut pack, &mut ctx()).unwrap();
        assert_eq!(pack.get("assets/minecraft/particles/block_marker.json"), Some(&b"barrier"[..]));
        assert!(!pack.contains("assets/minecraft/particles/light.json"));
        V1_18.downgrade(&mut pack, &mut ctx()).unwrap();
        assert_eq!(pack.get("assets/minecraft/particles/light.json"), Some(&b"barrier"[..]));
        assert_eq!(pack.get("assets/minecraft/particles/barrier.json"), Some(&b"barrier"[..]));
        assert!(!pack.contains("assets/minecraft/particles/block_marker.json"));
    }

    #[test]
    fn light_alone_becomes_the_block_marker() {
        let mut pack = Pack::new();
        pack.insert("assets/minecraft/particles/light.json", b"light".to_vec());
        V1_18.upgrade(&mut pack, &mut ctx()).unwrap();
        assert_eq!(pack.get("assets/minecraft/particles/block_marker.json"), Some(&b"light"[..]));
    }

    #[test]
    fn small_effect_background_is_built_from_the_wide_one_and_dropped() {
        for k in [1, 2] {
            let mut pack = Pack::new();
            let old = inventory(k);
            write_png(&mut pack, INVENTORY, &old).unwrap();
            V1_18.upgrade(&mut pack, &mut ctx()).unwrap();
            let new = read_png(&pack, INVENTORY).unwrap().unwrap();
            assert_eq!(
                new.get_pixel(3 * k, 198 * k + 5 * k),
                old.get_pixel(3 * k, 166 * k + 5 * k)
            );
            assert_eq!(
                new.get_pixel(16 * k + 2 * k, 198 * k),
                old.get_pixel(104 * k + 2 * k, 166 * k)
            );
            V1_18.downgrade(&mut pack, &mut ctx()).unwrap();
            assert_eq!(read_png(&pack, INVENTORY).unwrap().unwrap(), old);
        }
    }
}

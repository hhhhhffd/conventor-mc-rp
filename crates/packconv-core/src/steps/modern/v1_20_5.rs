//! 1.20.5: map icons were cut into `textures/map/decorations/*.png`, and `scute` became `turtle_scute`.

use image::RgbaImage;

use super::common::{crop, paste, rename_ids, resize, reversed};
use crate::error::Result;
use crate::step::{Context, Step};
use crate::util::{read_png, write_png};
use crate::version::Version;
use crate::vfs::Pack;

const MAP_ICONS: &str = "assets/minecraft/textures/map/map_icons.png";
const DECORATIONS: &str = "assets/minecraft/textures/map/decorations";

/// Decoration names in sheet order (Mojang slicer 1.20.5): the sheet is 128x128 with 8x8 cells,
/// 16 per row, and cell `i` is at `((i % 16) * 8, (i / 16) * 8)`.
const NAMES: [&str; 34] = [
    "player", "frame", "red_marker", "blue_marker", "target_x", "target_point", "player_off_map",
    "player_off_limits", "woodland_mansion", "ocean_monument", "white_banner", "orange_banner",
    "magenta_banner", "light_blue_banner", "yellow_banner", "lime_banner", "pink_banner",
    "gray_banner", "light_gray_banner", "cyan_banner", "purple_banner", "blue_banner",
    "brown_banner", "green_banner", "red_banner", "black_banner", "red_x", "desert_village",
    "plains_village", "savanna_village", "snowy_village", "taiga_village", "jungle_temple",
    "swamp_hut",
];

/// Cells from this index on (villages, jungle temple, swamp hut) are blank in sheets older than 1.20.2.
const FIRST_CELL_ADDED_IN_1_20_2: usize = 27;

const SCUTE: &[(&str, &str)] = &[("item/scute", "item/turtle_scute")];

pub struct V1_20_5;

impl Step for V1_20_5 {
    fn name(&self) -> &'static str {
        "1.20.5 map decorations"
    }

    fn version(&self) -> Version {
        Version::new(1, 20, 5)
    }

    fn upgrade(&self, pack: &mut Pack, ctx: &mut Context) -> Result<()> {
        rename_ids(pack, ctx, SCUTE);
        let sheet = match read_png(pack, MAP_ICONS) {
            None => return Ok(()),
            Some(Ok(sheet)) => sheet,
            Some(Err(e)) => {
                ctx.warn(format!("{e}; map icons not sliced"));
                return Ok(());
            }
        };
        let (cell_w, cell_h) = (sheet.width() / 16, sheet.height() / 16);
        let cells = if ctx.from < Version::new(1, 20, 2) { FIRST_CELL_ADDED_IN_1_20_2 } else { NAMES.len() };
        for (i, name) in NAMES.iter().enumerate().take(cells) {
            let (col, row) = (i as u32 % 16, i as u32 / 16);
            let path = format!("{DECORATIONS}/{name}.png");
            if pack.contains(&path) {
                continue;
            }
            if let Some(icon) = crop(&sheet, col * cell_w, row * cell_h, cell_w, cell_h) {
                write_png(pack, &path, &icon)?;
            }
        }
        pack.remove(MAP_ICONS);
        pack.remove(&format!("{MAP_ICONS}.mcmeta"));
        Ok(())
    }

    fn downgrade(&self, pack: &mut Pack, ctx: &mut Context) -> Result<()> {
        rename_ids(pack, ctx, &reversed(SCUTE));
        let mut icons = Vec::new();
        for (i, name) in NAMES.iter().enumerate() {
            match read_png(pack, &format!("{DECORATIONS}/{name}.png")) {
                None => {}
                Some(Ok(icon)) => icons.push((i as u32, icon)),
                Some(Err(e)) => ctx.warn(format!("{e}; map decoration not merged")),
            }
        }
        if icons.is_empty() {
            return Ok(());
        }
        let scale = icons.iter().map(|(_, icon)| icon.width() / 8).max().unwrap_or(1).max(1);
        let mut sheet = RgbaImage::new(128 * scale, 128 * scale);
        for (i, icon) in &icons {
            paste(&mut sheet, &resize(icon, 8 * scale, 8 * scale), (i % 16) * 8 * scale, (i / 16) * 8 * scale);
        }
        write_png(pack, MAP_ICONS, &sheet)?;
        for name in NAMES {
            pack.remove(&format!("{DECORATIONS}/{name}.png"));
            pack.remove(&format!("{DECORATIONS}/{name}.png.mcmeta"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::steps::modern::common::testing::{png, put_png, solid};

    #[test]
    fn slices_and_merges_map_icons() {
        let mut pack = Pack::new();
        let mut ctx = Context::new(Version::new(1, 20, 4), Version::new(1, 20, 5));
        let mut sheet = solid(128, 128, [0, 0, 0, 0]);
        paste(&mut sheet, &solid(8, 8, [255, 0, 0, 255]), 8, 0); // frame
        paste(&mut sheet, &solid(8, 8, [0, 0, 255, 255]), 8, 8); // cell 17: gray_banner
        put_png(&mut pack, MAP_ICONS, &sheet);
        V1_20_5.upgrade(&mut pack, &mut ctx).unwrap();
        assert!(!pack.contains(MAP_ICONS));
        assert_eq!(png(&pack, &format!("{DECORATIONS}/frame.png")).get_pixel(0, 0).0, [255, 0, 0, 255]);
        assert_eq!(png(&pack, &format!("{DECORATIONS}/gray_banner.png")).get_pixel(3, 3).0, [0, 0, 255, 255]);
        V1_20_5.downgrade(&mut pack, &mut ctx).unwrap();
        assert_eq!(png(&pack, MAP_ICONS), sheet);
        assert!(!pack.contains(&format!("{DECORATIONS}/frame.png")));
    }

    #[test]
    fn old_sheets_skip_cells_added_in_1_20_2() {
        let mut pack = Pack::new();
        let mut ctx = Context::new(Version::new(1, 19, 4), Version::new(1, 20, 5));
        put_png(&mut pack, MAP_ICONS, &solid(128, 128, [1, 1, 1, 255]));
        V1_20_5.upgrade(&mut pack, &mut ctx).unwrap();
        assert!(pack.contains(&format!("{DECORATIONS}/red_x.png")));
        assert!(!pack.contains(&format!("{DECORATIONS}/desert_village.png")));
    }

    #[test]
    fn renames_scute() {
        let mut pack = Pack::new();
        let mut ctx = Context::new(Version::new(1, 20, 4), Version::new(1, 20, 5));
        put_png(&mut pack, "assets/minecraft/textures/item/scute.png", &solid(1, 1, [1, 1, 1, 255]));
        V1_20_5.upgrade(&mut pack, &mut ctx).unwrap();
        assert!(pack.contains("assets/minecraft/textures/item/turtle_scute.png"));
    }
}

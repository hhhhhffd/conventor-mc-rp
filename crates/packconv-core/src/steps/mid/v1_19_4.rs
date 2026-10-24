//! 1.19.4: separate enchantment glint textures for items and entities, and a dedicated
//! slider texture.

use crate::error::Result;
use crate::step::{Context, Step};
use crate::steps::support::{canvas, save_png, Renamer};
use crate::version::Version;
use crate::vfs::Pack;

use super::sheet::{copy_area, load_png, scale_factor, Area, Flip};

const MISC: &str = "assets/minecraft/textures/misc";
const WIDGETS: &str = "assets/minecraft/textures/gui/widgets.png";
const SLIDER: &str = "assets/minecraft/textures/gui/slider.png";
const WIDGETS_DESIGN: (u32, u32) = (256, 256);

/// The slider texture is made of 200x20 button strips of the widgets sheet:
/// `(source y in widgets.png, destination y in slider.png)`. Row 46 is the disabled button
/// and serves as the track in both states, 66 and 86 are the normal and highlighted buttons
/// that used to double as the handle.
const SLIDER_STRIPS: [(u32, u32); 4] = [(46, 0), (46, 20), (66, 40), (86, 60)];

pub struct V1_19_4;

impl Step for V1_19_4 {
    fn name(&self) -> &'static str {
        "1.19.4 enchantment glint and slider textures"
    }

    fn version(&self) -> Version {
        Version::new(1, 19, 4)
    }

    fn upgrade(&self, pack: &mut Pack, ctx: &mut Context) -> Result<()> {
        let mut kept = false;
        for suffix in ["png", "png.mcmeta"] {
            if let Some(glint) = pack.remove(&format!("{MISC}/enchanted_item_glint.{suffix}")) {
                for kind in ["entity", "item"] {
                    let path = format!("{MISC}/enchanted_glint_{kind}.{suffix}");
                    kept |= pack.contains(&path);
                    if !pack.contains(&path) {
                        pack.insert(path, glint.clone());
                    }
                }
            }
        }
        if kept {
            ctx.warn("an existing enchanted_glint_item/entity texture was kept instead of enchanted_item_glint");
        }

        if pack.contains(SLIDER) {
            return Ok(());
        }
        let Some(widgets) = load_png(pack, WIDGETS, ctx) else { return Ok(()) };
        let Some(k) = scale_factor(&widgets, WIDGETS_DESIGN) else {
            ctx.warn(format!(
                "{WIDGETS}: unexpected size {}x{}; no slider texture created",
                widgets.width(),
                widgets.height()
            ));
            return Ok(());
        };
        let Some(mut slider) = canvas(ctx, SLIDER, Some(widgets.width()), Some(widgets.height()))
        else {
            return Ok(());
        };
        for (from_y, to_y) in SLIDER_STRIPS {
            let strip: Area = (0, from_y * k, 200 * k, 20 * k);
            copy_area(&mut slider, &widgets, strip, (0, to_y * k), Flip::None);
        }
        save_png(pack, ctx, SLIDER, &slider);
        Ok(())
    }

    fn downgrade(&self, pack: &mut Pack, ctx: &mut Context) -> Result<()> {
        // The item glint wins when both exist: it is the one players see most.
        let kept = if pack.contains(&format!("{MISC}/enchanted_glint_item.png")) {
            "item"
        } else {
            "entity"
        };
        let dropped = if kept == "item" { "entity" } else { "item" };
        let glint = |kind: &str| pack.get(&format!("{MISC}/enchanted_glint_{kind}.png"));
        if glint(dropped).is_some_and(|other| Some(other) != glint(kept)) {
            ctx.warn(format!("the {dropped} enchantment glint was dropped: before 1.19.4 items and entities share one texture, taken from the {kept} glint"));
        }
        let mut renamer = Renamer::default();
        for suffix in ["png", "png.mcmeta"] {
            renamer.rename(
                pack,
                &format!("{MISC}/enchanted_glint_{kept}.{suffix}"),
                &format!("{MISC}/enchanted_item_glint.{suffix}"),
            );
            pack.remove(&format!("{MISC}/enchanted_glint_{dropped}.{suffix}"));
        }
        renamer.report(ctx);

        // Before 1.19.4 sliders were drawn from widgets.png, which already holds the buttons.
        if pack.remove(SLIDER).is_some() {
            ctx.warn("gui/slider.png was dropped: sliders use gui/widgets.png before 1.19.4");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::{read_png, write_png};
    use image::RgbaImage;
    use image::Rgba;

    fn ctx() -> Context {
        Context::new(Version::new(1, 19, 3), Version::new(1, 19, 4))
    }

    #[test]
    fn glint_is_split_for_items_and_entities() {
        let mut pack = Pack::new();
        pack.insert("assets/minecraft/textures/misc/enchanted_item_glint.png", vec![1]);
        pack.insert("assets/minecraft/textures/misc/enchanted_item_glint.png.mcmeta", vec![2]);
        V1_19_4.upgrade(&mut pack, &mut ctx()).unwrap();
        assert!(!pack.contains("assets/minecraft/textures/misc/enchanted_item_glint.png"));
        for kind in ["entity", "item"] {
            assert_eq!(pack.get(&format!("{MISC}/enchanted_glint_{kind}.png")), Some(&[1u8][..]));
            assert_eq!(
                pack.get(&format!("{MISC}/enchanted_glint_{kind}.png.mcmeta")),
                Some(&[2u8][..])
            );
        }
        let mut ctx = ctx();
        V1_19_4.downgrade(&mut pack, &mut ctx).unwrap();
        assert_eq!(pack.get(&format!("{MISC}/enchanted_item_glint.png")), Some(&[1u8][..]));
        assert_eq!(pack.get(&format!("{MISC}/enchanted_item_glint.png.mcmeta")), Some(&[2u8][..]));
        assert_eq!(pack.paths().count(), 2);
        assert!(ctx.into_warnings().is_empty());
    }

    #[test]
    fn downgrade_prefers_the_item_glint_and_reports_the_other() {
        let mut pack = Pack::new();
        pack.insert(format!("{MISC}/enchanted_glint_item.png"), vec![1]);
        pack.insert(format!("{MISC}/enchanted_glint_entity.png"), vec![2]);
        let mut ctx = ctx();
        V1_19_4.downgrade(&mut pack, &mut ctx).unwrap();
        assert_eq!(pack.get(&format!("{MISC}/enchanted_item_glint.png")), Some(&[1u8][..]));
        assert_eq!(pack.paths().count(), 1);
        assert_eq!(ctx.into_warnings().len(), 1);
    }

    #[test]
    fn downgrade_uses_the_entity_glint_when_it_is_alone() {
        let mut pack = Pack::new();
        pack.insert(format!("{MISC}/enchanted_glint_entity.png"), vec![2]);
        V1_19_4.downgrade(&mut pack, &mut ctx()).unwrap();
        assert_eq!(pack.get(&format!("{MISC}/enchanted_item_glint.png")), Some(&[2u8][..]));
    }

    #[test]
    fn slider_is_built_from_the_widgets_and_dropped_on_downgrade() {
        for k in [1, 2] {
            let mut pack = Pack::new();
            let widgets = RgbaImage::from_fn(256 * k, 256 * k, |x, y| {
                Rgba([(x / k) as u8, (y / k) as u8, 5, 255])
            });
            write_png(&mut pack, WIDGETS, &widgets).unwrap();
            V1_19_4.upgrade(&mut pack, &mut ctx()).unwrap();
            let slider = read_png(&pack, SLIDER).unwrap().unwrap();
            assert_eq!(slider.dimensions(), widgets.dimensions());
            assert_eq!(slider.get_pixel(3 * k, 41 * k), widgets.get_pixel(3 * k, 67 * k));
            assert_eq!(slider.get_pixel(3 * k, 21 * k), widgets.get_pixel(3 * k, 47 * k));
            assert_eq!(slider.get_pixel(3 * k, 100 * k)[3], 0);

            let mut ctx = ctx();
            V1_19_4.downgrade(&mut pack, &mut ctx).unwrap();
            assert!(!pack.contains(SLIDER));
            assert_eq!(read_png(&pack, WIDGETS).unwrap().unwrap(), widgets);
            assert_eq!(ctx.into_warnings().len(), 1);
        }
    }

    #[test]
    fn existing_slider_is_kept_on_upgrade() {
        let mut pack = Pack::new();
        pack.insert(SLIDER, vec![7]);
        write_png(&mut pack, WIDGETS, &RgbaImage::new(256, 256)).unwrap();
        V1_19_4.upgrade(&mut pack, &mut ctx()).unwrap();
        assert_eq!(pack.get(SLIDER), Some(&[7u8][..]));
    }
}

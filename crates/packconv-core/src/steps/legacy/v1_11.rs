//! 1.11 lowercased language file names (`en_US.lang` -> `en_us.lang`), stopped accepting
//! spaces in resource names and replaced the single shulker texture with one per color.

use serde_json::Value;

use crate::error::Result;
use crate::step::{Context, Step};
use crate::version::Version;
use crate::vfs::Pack;

use super::common::{
    drop_added_files, json_paths, move_file, move_textures, retarget_texture_refs, reversed,
    update_json,
};

/// The 1.10 shulker texture became the purple variant.
const SHULKER_RENAMES: [(&str, &str); 1] =
    [("entity/shulker/endergolem", "entity/shulker/shulker_purple")];

pub struct V1_11;

impl Step for V1_11 {
    fn name(&self) -> &'static str {
        "legacy-1.11"
    }

    fn version(&self) -> Version {
        Version::new(1, 11, 0)
    }

    fn upgrade(&self, pack: &mut Pack, ctx: &mut Context) -> Result<()> {
        rename_language_files(pack, ctx, str::to_lowercase);
        let renamed_spaces = replace_spaces_in_paths(pack, ctx);
        move_textures(pack, ctx, &SHULKER_RENAMES);
        for path in json_paths(pack, "models") {
            update_json(pack, ctx, &path, |model| {
                let retargeted = retarget_texture_refs(model, &SHULKER_RENAMES);
                (renamed_spaces && replace_spaces_in_references(model)) | retargeted
            });
        }
        if renamed_spaces {
            for path in json_paths(pack, "blockstates") {
                update_json(pack, ctx, &path, replace_spaces_in_references);
            }
        }
        Ok(())
    }

    /// Spaces cannot be restored, so names that lost them stay with underscores.
    fn downgrade(&self, pack: &mut Pack, ctx: &mut Context) -> Result<()> {
        rename_language_files(pack, ctx, vanilla_1_10_case);
        let shulker_back = reversed(&SHULKER_RENAMES);
        move_textures(pack, ctx, &shulker_back);
        for path in json_paths(pack, "models") {
            update_json(pack, ctx, &path, |model| {
                retarget_texture_refs(model, &shulker_back)
            });
        }
        drop_added_files(pack, ctx, "1.11");
        Ok(())
    }
}

/// `en_us` -> `en_US`: the 1.10 language codes have an uppercase region.
fn vanilla_1_10_case(code: &str) -> String {
    match code.split_once('_') {
        Some((language, region)) => format!("{language}_{}", region.to_uppercase()),
        None => code.to_string(),
    }
}

/// Renames `assets/<namespace>/lang/<code>.lang` files, passing each `<code>` through `rename`.
fn rename_language_files(pack: &mut Pack, ctx: &mut Context, rename: fn(&str) -> String) {
    let langs: Vec<String> = pack
        .paths()
        .filter(|path| {
            let mut parts = path.split('/');
            parts.next() == Some("assets")
                && parts.nth(1) == Some("lang")
                && path.ends_with(".lang")
        })
        .map(str::to_string)
        .collect();
    for path in langs {
        let (dir, file) = path.rsplit_once('/').expect("path has a directory");
        let code = file.strip_suffix(".lang").expect("filtered on .lang");
        let renamed = format!("{dir}/{}.lang", rename(code));
        if renamed != path {
            move_file(pack, ctx, &path, &renamed);
        }
    }
}

/// Replaces spaces by underscores in every asset path. Returns whether any path had one.
fn replace_spaces_in_paths(pack: &mut Pack, ctx: &mut Context) -> bool {
    let with_spaces: Vec<String> = pack
        .paths_with_prefix("assets/")
        .into_iter()
        .filter(|path| path.contains(' '))
        .collect();
    for path in &with_spaces {
        move_file(pack, ctx, path, &path.replace(' ', "_"));
    }
    !with_spaces.is_empty()
}

/// Fixes the values that name other resources: `parent`, `model` and every `textures` entry.
fn replace_spaces_in_references(value: &mut Value) -> bool {
    let fix = |text: &mut Value| match text.as_str() {
        Some(s) if s.contains(' ') => {
            *text = Value::String(s.replace(' ', "_"));
            true
        }
        _ => false,
    };
    let mut changed = false;
    match value {
        Value::Object(map) => {
            for (key, child) in map.iter_mut() {
                if key == "parent" || key == "model" {
                    changed |= fix(child) | replace_spaces_in_references(child);
                } else if key == "textures" {
                    if let Value::Object(textures) = child {
                        for texture in textures.values_mut() {
                            changed |= fix(texture);
                        }
                    }
                } else {
                    changed |= replace_spaces_in_references(child);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                changed |= replace_spaces_in_references(item);
            }
        }
        _ => {}
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::steps::legacy::common::testing::*;

    #[test]
    fn upgrade_lowercases_language_files_in_every_namespace() {
        let mut pack = Pack::new();
        put_json(&mut pack, "assets/minecraft/lang/en_US.lang", "a=b");
        put_json(&mut pack, "assets/mod/lang/zh_TW.lang", "a=b");
        let mut ctx = Context::new(Version::new(1, 10, 0), Version::new(1, 11, 0));
        V1_11.upgrade(&mut pack, &mut ctx).unwrap();
        assert!(pack.contains("assets/minecraft/lang/en_us.lang"));
        assert!(pack.contains("assets/mod/lang/zh_tw.lang"));
        assert!(!pack.contains("assets/minecraft/lang/en_US.lang"));
    }

    #[test]
    fn downgrade_restores_language_case() {
        let mut pack = Pack::new();
        put_json(&mut pack, "assets/minecraft/lang/en_us.lang", "a=b");
        let mut ctx = Context::new(Version::new(1, 11, 0), Version::new(1, 10, 0));
        V1_11.downgrade(&mut pack, &mut ctx).unwrap();
        assert!(pack.contains("assets/minecraft/lang/en_US.lang"));
    }

    #[test]
    fn upgrade_replaces_spaces_in_names_and_references() {
        let mut pack = Pack::new();
        put_png(
            &mut pack,
            "assets/minecraft/textures/blocks/my stone.png",
            &solid(1, 1, [1; 4]),
        );
        put_json(
            &mut pack,
            "assets/minecraft/models/block/my block.json",
            r#"{"parent":"block/cube_all","textures":{"all":"blocks/my stone"}}"#,
        );
        put_json(
            &mut pack,
            "assets/minecraft/blockstates/my block.json",
            r#"{"variants":{"normal":{"model":"my block"}}}"#,
        );
        let mut ctx = Context::new(Version::new(1, 10, 0), Version::new(1, 11, 0));
        V1_11.upgrade(&mut pack, &mut ctx).unwrap();

        assert!(pack.contains("assets/minecraft/textures/blocks/my_stone.png"));
        let model = get_json(&pack, "assets/minecraft/models/block/my_block.json");
        assert_eq!(model["textures"]["all"], "blocks/my_stone");
        let state = get_json(&pack, "assets/minecraft/blockstates/my_block.json");
        assert_eq!(state["variants"]["normal"]["model"], "my_block");
        assert!(ctx.into_warnings().is_empty());
    }

    #[test]
    fn space_collision_is_reported() {
        let mut pack = Pack::new();
        put_json(&mut pack, "assets/minecraft/lang/a b.lang", "1");
        put_json(&mut pack, "assets/minecraft/lang/a_b.lang", "2");
        let mut ctx = Context::new(Version::new(1, 10, 0), Version::new(1, 11, 0));
        V1_11.upgrade(&mut pack, &mut ctx).unwrap();
        assert_eq!(ctx.into_warnings().len(), 1);
    }

    #[test]
    fn shulker_texture_roundtrip_and_new_files_are_dropped() {
        let mut pack = Pack::new();
        put_png(
            &mut pack,
            "assets/minecraft/textures/entity/shulker/endergolem.png",
            &solid(1, 1, [1; 4]),
        );
        put_json(
            &mut pack,
            "assets/minecraft/models/block/x.json",
            r#"{"textures":{"a":"minecraft:entity/shulker/endergolem"}}"#,
        );
        let mut ctx = Context::new(Version::new(1, 10, 0), Version::new(1, 11, 0));
        V1_11.upgrade(&mut pack, &mut ctx).unwrap();
        assert!(pack.contains("assets/minecraft/textures/entity/shulker/shulker_purple.png"));
        let model = get_json(&pack, "assets/minecraft/models/block/x.json");
        assert_eq!(model["textures"]["a"], "entity/shulker/shulker_purple");

        put_png(
            &mut pack,
            "assets/minecraft/textures/entity/llama/llama.png",
            &solid(1, 1, [2; 4]),
        );
        V1_11.downgrade(&mut pack, &mut ctx).unwrap();
        assert!(pack.contains("assets/minecraft/textures/entity/shulker/endergolem.png"));
        assert!(!pack.contains("assets/minecraft/textures/entity/llama/llama.png"));
        let model = get_json(&pack, "assets/minecraft/models/block/x.json");
        assert_eq!(model["textures"]["a"], "entity/shulker/endergolem");
    }
}

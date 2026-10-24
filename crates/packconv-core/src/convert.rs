use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde::Serialize;

use crate::archive::{read_archive, write_zip, Limits};
use crate::error::{Error, Result};
use crate::meta;
use crate::step::Context;
use crate::steps;
use crate::version::{self, PackFormat, Version};
use crate::vfs::Pack;

#[derive(Debug, Clone)]
pub struct ConvertOptions {
    /// Source version; detected from `pack.mcmeta` when `None`.
    pub from: Option<String>,
    pub to: String,
    pub limits: Limits,
    /// Set by the caller to abort a running conversion (checked between steps).
    pub cancel: Option<Arc<AtomicBool>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub from: String,
    pub to: String,
    pub target_format: String,
    pub steps: Vec<String>,
    pub warnings: Vec<String>,
    pub files_in: usize,
    pub files_out: usize,
}

impl Report {
    /// Compact form small enough for an HTTP header: warnings are only counted.
    pub fn header_summary(&self) -> serde_json::Value {
        serde_json::json!({
            "from": self.from,
            "to": self.to,
            "target_format": self.target_format,
            "steps": self.steps,
            "warnings": self.warnings.len(),
            "files_in": self.files_in,
            "files_out": self.files_out,
        })
    }
}

#[derive(Debug)]
pub struct ConvertOutput {
    pub zip: Vec<u8>,
    pub report: Report,
}

/// Names of all supported versions, oldest first.
pub fn supported_versions() -> Vec<String> {
    version::releases().map(|(v, _)| v.to_string()).collect()
}

/// Converts a ZIP/RAR resource pack archive and returns the converted pack as a ZIP.
pub fn convert(input: &[u8], options: &ConvertOptions) -> Result<ConvertOutput> {
    let to: Version = options.to.parse()?;
    let from = options.from.as_deref().map(str::parse::<Version>).transpose()?;
    let mut pack = read_archive(input, options.limits)?;
    let never = AtomicBool::new(false);
    let cancel = options.cancel.as_deref().unwrap_or(&never);
    let report = convert_pack(&mut pack, from, to, cancel)?;
    Ok(ConvertOutput { zip: write_zip(&pack)?, report })
}

fn format_name(format: PackFormat) -> String {
    if format.1 == 0 {
        format.0.to_string()
    } else {
        format!("{}.{}", format.0, format.1)
    }
}

/// Picks the source version from the declared format range. When the target is inside the
/// range the author already claims compatibility, so nothing needs converting; otherwise the
/// range end nearest to the target is used. Format 1 covers both 1.7 (no block models) and
/// 1.8, so the presence of models disambiguates.
fn detect_version(pack: &Pack, (min, max): (PackFormat, PackFormat), to: Version) -> Result<Version> {
    if max <= PackFormat(1, 0) {
        let has_models = !pack.paths_with_prefix("assets/minecraft/models/").is_empty();
        return Ok(if has_models { Version(1, 8, 9) } else { Version(1, 7, 10) });
    }
    let target = version::pack_format(to)?;
    let detected = if target < min {
        version::version_for_format(min)
    } else if target > max {
        version::version_for_format(max)
    } else {
        return Ok(to);
    };
    // Format 4 spans 1.13 and 1.14; 1.13 packs still have the sheets that 1.14 sliced.
    let format_4_sheets = [
        "assets/minecraft/textures/particle/particles.png",
        "assets/minecraft/textures/painting/paintings_kristoffer_zetterstrand.png",
    ];
    if version::pack_format(detected)? == PackFormat(4, 0) && format_4_sheets.iter().any(|p| pack.contains(p)) {
        return Ok(Version(1, 13, 2));
    }
    Ok(detected)
}

/// Files the converter touches: vanilla namespaces only. Other namespaces (mods), OptiFine and
/// MCPatcher folders and anything outside `assets/` pass through unchanged.
fn is_official(path: &str) -> bool {
    if path == "pack.mcmeta" || path == "pack.png" {
        return true;
    }
    let Some(rest) = path.strip_prefix("assets/minecraft/").or_else(|| path.strip_prefix("assets/realms/")) else {
        return false;
    };
    !(rest.starts_with("optifine/") || rest.starts_with("mcpatcher/"))
}

fn remove_junk(pack: &mut Pack) {
    let junk: Vec<String> = pack
        .paths()
        .filter(|p| {
            let name = p.rsplit('/').next().unwrap_or(p);
            p.starts_with("__MACOSX/") || name == ".DS_Store" || name == "Thumbs.db" || name == "desktop.ini"
        })
        .map(String::from)
        .collect();
    for path in junk {
        pack.remove(&path);
    }
}

/// Converts an unpacked pack in place. Fails with [`Error::Cancelled`] once `cancel` is set.
pub fn convert_pack(pack: &mut Pack, from: Option<Version>, to: Version, cancel: &AtomicBool) -> Result<Report> {
    let target_format = version::pack_format(to)?;
    let files_in = pack.len();
    remove_junk(pack);

    let from = match from {
        Some(v) => {
            version::pack_format(v)?;
            v
        }
        None => detect_version(pack, meta::declared_range(pack)?, to)?,
    };
    meta::flatten_overlays(pack, version::pack_format(from)?)?;
    let passthrough = pack.split_off(|p| !is_official(p));

    let mut ctx = Context::new(from, to);
    if from != to {
        strip_shaders(pack, &mut ctx);
    }
    let mut applied = Vec::new();
    let all = steps::all();
    if from < to {
        for step in all.iter().filter(|s| from < s.version() && s.version() <= to) {
            check_cancel(cancel)?;
            step.upgrade(pack, &mut ctx)?;
            applied.push(format!("upgrade {} ({})", step.name(), step.version()));
        }
    } else {
        for step in all.iter().rev().filter(|s| to < s.version() && s.version() <= from) {
            check_cancel(cancel)?;
            step.downgrade(pack, &mut ctx)?;
            applied.push(format!("downgrade {} ({})", step.name(), step.version()));
        }
    }
    pack.merge(passthrough);
    meta::adjust_language_codes(pack, to >= Version::new(1, 11, 0))?;
    meta::write_target_meta(pack, target_format)?;

    Ok(Report {
        from: from.to_string(),
        to: to.to_string(),
        target_format: format_name(target_format),
        steps: applied,
        warnings: ctx.into_warnings(),
        files_in,
        files_out: pack.len(),
    })
}

/// Shaders are GLSL bound to one engine version's uniforms, vertex formats and file layout
/// (they changed in 1.17, 1.21.2, 1.21.5, 1.21.6, 1.21.9, 26.1, 26.2...). Loaded on another
/// version they fail to compile, which can crash the game or leave a black screen, and they
/// cannot be converted mechanically. Removing them makes the game fall back to vanilla.
fn strip_shaders(pack: &mut Pack, ctx: &mut Context) {
    let removed = pack
        .split_off(|p| {
            p.starts_with("assets/minecraft/shaders/") || p.starts_with("assets/minecraft/post_effect/")
        })
        .len();
    if removed > 0 {
        ctx.warn(format!(
            "removed {removed} shader file(s): custom shaders only work on the version they were written for and can crash other versions"
        ));
    }
}

fn check_cancel(cancel: &AtomicBool) -> Result<()> {
    if cancel.load(Ordering::Relaxed) {
        Err(Error::Cancelled)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn official_paths() {
        assert!(is_official("pack.mcmeta"));
        assert!(is_official("assets/minecraft/textures/block/stone.png"));
        assert!(!is_official("assets/minecraft/optifine/cit/a.properties"));
        assert!(!is_official("assets/minecraft/mcpatcher/sky/world0/sky1.png"));
        assert!(!is_official("assets/mymod/textures/item/x.png"));
        assert!(!is_official("README.txt"));
    }

    #[test]
    fn passes_unofficial_files_through_and_honours_cancel() {
        let mut pack = Pack::new();
        pack.insert("pack.mcmeta", br#"{"pack":{"pack_format":3,"description":""}}"#.to_vec());
        pack.insert("assets/minecraft/mcpatcher/ctm/x.properties", b"matchBlocks=1".to_vec());
        pack.insert("assets/mymod/textures/blocks/a.png", vec![1]);
        convert_pack(&mut pack, None, Version::new(1, 16, 5), &AtomicBool::new(false)).unwrap();
        assert_eq!(pack.get("assets/minecraft/mcpatcher/ctm/x.properties"), Some(&b"matchBlocks=1"[..]));
        assert!(pack.contains("assets/mymod/textures/blocks/a.png"));

        let err = convert_pack(&mut pack, None, Version::new(1, 8, 9), &AtomicBool::new(true));
        assert!(matches!(err, Err(Error::Cancelled)));
    }

    #[test]
    fn strips_shaders_only_when_the_version_changes() {
        let mut pack = Pack::new();
        pack.insert("pack.mcmeta", br#"{"pack":{"pack_format":34,"description":""}}"#.to_vec());
        pack.insert("assets/minecraft/shaders/core/rendertype_solid.fsh", vec![1]);
        pack.insert("assets/minecraft/shaders/post/blur.json", vec![2]);
        pack.insert("assets/minecraft/post_effect/spider.json", vec![3]);
        pack.insert("assets/mymod/shaders/core/x.fsh", vec![4]);

        let mut same = pack.clone();
        let report = convert_pack(&mut same, None, Version::new(1, 21, 1), &AtomicBool::new(false)).unwrap();
        assert!(same.contains("assets/minecraft/shaders/core/rendertype_solid.fsh"));
        assert!(report.warnings.is_empty());

        let report = convert_pack(&mut pack, None, Version::new(1, 21, 4), &AtomicBool::new(false)).unwrap();
        assert!(pack.paths_with_prefix("assets/minecraft/shaders/").is_empty());
        assert!(!pack.contains("assets/minecraft/post_effect/spider.json"));
        assert!(pack.contains("assets/mymod/shaders/core/x.fsh"));
        assert!(report.warnings.iter().any(|w| w.contains("removed 3 shader")));
    }
}

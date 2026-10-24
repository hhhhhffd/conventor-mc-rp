use serde::Serialize;

use crate::archive::{read_archive, write_zip, Limits};
use crate::error::Result;
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
    let report = convert_pack(&mut pack, from, to)?;
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
    if target < min {
        Ok(version::version_for_format(min))
    } else if target > max {
        Ok(version::version_for_format(max))
    } else {
        Ok(to)
    }
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

/// Converts an unpacked pack in place.
pub fn convert_pack(pack: &mut Pack, from: Option<Version>, to: Version) -> Result<Report> {
    let target_format = version::pack_format(to)?;
    let files_in = pack.len();
    remove_junk(pack);

    let declared = meta::declared_range(pack)?;
    let from = match from {
        Some(v) => {
            version::pack_format(v)?;
            v
        }
        None => detect_version(pack, declared, to)?,
    };
    meta::flatten_overlays(pack, version::pack_format(from)?)?;

    let mut ctx = Context::new(from, to);
    let mut applied = Vec::new();
    let all = steps::all();
    if from < to {
        for step in all.iter().filter(|s| from < s.version() && s.version() <= to) {
            step.upgrade(pack, &mut ctx)?;
            applied.push(format!("upgrade {} ({})", step.name(), step.version()));
        }
    } else {
        for step in all.iter().rev().filter(|s| to < s.version() && s.version() <= from) {
            step.downgrade(pack, &mut ctx)?;
            applied.push(format!("downgrade {} ({})", step.name(), step.version()));
        }
    }
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

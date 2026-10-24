//! `pack.mcmeta` handling: source format detection, overlay flattening and target rewrite.

use serde_json::{json, Map, Value};

use crate::error::{Error, Result};
use crate::util::{read_json, write_json};
use crate::version::{PackFormat, MIN_MAX_FORMAT_SINCE};
use crate::vfs::Pack;

const META: &str = "pack.mcmeta";

fn read_meta(pack: &Pack) -> Result<Value> {
    let value = read_json(pack, META)
        .ok_or(Error::MissingPackMeta)?
        .map_err(|e| Error::PackMeta(e.to_string()))?;
    if !value.get("pack").is_some_and(Value::is_object) {
        return Err(Error::PackMeta("missing \"pack\" object".into()));
    }
    Ok(value)
}

/// Parses `N`, `[N]`, `[N, M]` (min_format/max_format notation).
fn parse_format(value: &Value) -> Option<PackFormat> {
    match value {
        Value::Number(n) => n.as_u64().map(|n| PackFormat(n as u32, 0)),
        Value::Array(a) => {
            let major = a.first()?.as_u64()? as u32;
            let minor = a.get(1).and_then(Value::as_u64).unwrap_or(0) as u32;
            Some(PackFormat(major, minor))
        }
        _ => None,
    }
}

/// Inclusive format range from the legacy `formats`/`supported_formats` notations.
fn parse_range(value: &Value) -> Option<(PackFormat, PackFormat)> {
    match value {
        Value::Number(_) => parse_format(value).map(|f| (f, f)),
        Value::Array(a) if a.len() == 2 => Some((parse_format(&a[0])?, parse_format(&a[1])?)),
        Value::Object(o) => Some((
            parse_format(o.get("min_inclusive")?)?,
            parse_format(o.get("max_inclusive")?)?,
        )),
        _ => None,
    }
}

/// Format the pack declares. Newer packs may only carry `min_format`; in that case the
/// newest declared format is used since that is the layout the author targeted.
pub fn declared_format(pack: &Pack) -> Result<PackFormat> {
    let meta = read_meta(pack)?;
    let section = &meta["pack"];
    section
        .get("max_format")
        .and_then(parse_format)
        .or_else(|| section.get("pack_format").and_then(parse_format))
        .or_else(|| section.get("min_format").and_then(parse_format))
        .ok_or_else(|| Error::PackMeta("no pack_format".into()))
}

/// Applies the overlays that would be active for `format` onto the pack root and removes
/// every overlay directory, so steps only see a single layout.
pub fn flatten_overlays(pack: &mut Pack, format: PackFormat) -> Result<()> {
    let mut meta = read_meta(pack)?;
    let Some(entries) = meta
        .get_mut("overlays")
        .and_then(|o| o.as_object_mut())
        .and_then(|o| o.remove("entries"))
    else {
        return Ok(());
    };
    for entry in entries.as_array().into_iter().flatten() {
        let Some(dir) = entry.get("directory").and_then(Value::as_str) else { continue };
        let range = entry.get("formats").and_then(parse_range).or_else(|| {
            let min = entry.get("min_format").and_then(parse_format)?;
            let max = entry.get("max_format").and_then(parse_format)?;
            Some((min, max))
        });
        let active = range.is_some_and(|(min, max)| min <= format && format <= max);
        let prefix = format!("{dir}/");
        for path in pack.paths_with_prefix(&prefix) {
            let data = pack.remove(&path).expect("listed path exists");
            if active {
                pack.insert(path[prefix.len()..].to_string(), data);
            }
        }
    }
    meta.as_object_mut().expect("checked object").remove("overlays");
    write_json(pack, META, &meta);
    Ok(())
}

/// Rewrites `pack.mcmeta` so the target game accepts the pack without an
/// "incompatible" warning.
pub fn write_target_meta(pack: &mut Pack, format: PackFormat) -> Result<()> {
    let mut meta = read_meta(pack)?;
    let section = meta["pack"].as_object_mut().expect("checked object");
    let mut out = Map::new();
    if format >= MIN_MAX_FORMAT_SINCE {
        // Since 25w31a the range replaces `pack_format`. An integer `max_format` covers
        // every minor revision of that major format.
        let min = if format.1 == 0 { json!(format.0) } else { json!([format.0, format.1]) };
        out.insert("min_format".into(), min);
        out.insert("max_format".into(), json!(format.0));
    } else {
        out.insert("pack_format".into(), json!(format.0));
    }
    out.insert("description".into(), section.remove("description").unwrap_or(json!("")));
    // Keep any other author-provided fields (e.g. unknown future keys) untouched.
    for (key, value) in std::mem::take(section) {
        if !matches!(key.as_str(), "pack_format" | "supported_formats" | "min_format" | "max_format") {
            out.insert(key, value);
        }
    }
    *section = out;
    meta.as_object_mut().expect("checked object").remove("overlays");
    write_json(pack, META, &meta);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pack_with_meta(meta: &str) -> Pack {
        let mut pack = Pack::new();
        pack.insert(META, meta.as_bytes().to_vec());
        pack
    }

    #[test]
    fn declared_formats() {
        let p = pack_with_meta(r#"{"pack":{"pack_format":15,"description":"x"}}"#);
        assert_eq!(declared_format(&p).unwrap(), PackFormat(15, 0));
        let p = pack_with_meta(r#"{"pack":{"min_format":[69,0],"max_format":[75,0]}}"#);
        assert_eq!(declared_format(&p).unwrap(), PackFormat(75, 0));
        let p = pack_with_meta("\u{feff}{\"pack\":{\"pack_format\":1}}");
        assert_eq!(declared_format(&p).unwrap(), PackFormat(1, 0));
    }

    #[test]
    fn flattens_active_overlay() {
        let mut p = pack_with_meta(
            r#"{"pack":{"pack_format":18},"overlays":{"entries":[
                {"formats":[18,22],"directory":"new"},
                {"formats":{"min_inclusive":32,"max_inclusive":34},"directory":"newer"}]}}"#,
        );
        p.insert("assets/a.png", vec![1]);
        p.insert("new/assets/a.png", vec![2]);
        p.insert("newer/assets/b.png", vec![3]);
        flatten_overlays(&mut p, PackFormat(18, 0)).unwrap();
        assert_eq!(p.get("assets/a.png"), Some(&[2u8][..]));
        assert!(!p.contains("assets/b.png") && !p.contains("newer/assets/b.png"));
        assert!(read_meta(&p).unwrap().get("overlays").is_none());
    }

    #[test]
    fn writes_target_meta() {
        let mut p = pack_with_meta(
            r#"{"pack":{"pack_format":15,"supported_formats":[15,18],"description":"d"},"language":{}}"#,
        );
        write_target_meta(&mut p, PackFormat(75, 0)).unwrap();
        let m = read_meta(&p).unwrap();
        assert_eq!(m["pack"], json!({"min_format":75,"max_format":75,"description":"d"}));
        assert!(m.get("language").is_some());
    }
}

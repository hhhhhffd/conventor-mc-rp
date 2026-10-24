//! End-to-end run over real resource packs.
//!
//! Set `PACKCONV_FIXTURES` to a directory with `.zip`/`.rar` packs; without it the test is a no-op.
//! Every failure is collected and reported together at the end of the run.

use std::collections::BTreeSet;
use std::io::Read;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};

use packconv_core::archive::read_archive;
use packconv_core::version::pack_format;
use packconv_core::{convert, supported_versions, ConvertOptions, ConvertOutput, Limits, Version};
use serde_json::Value;

const TARGETS: [&str; 6] = ["1.7.10", "1.8.9", "1.12.2", "1.16.5", "1.20.1", "1.21.4"];
const MAX_REPORTED_PATHS: usize = 20;

fn options(from: Option<&str>, to: &str) -> ConvertOptions {
    ConvertOptions { from: from.map(String::from), to: to.into(), limits: Limits::default() }
}

fn run(input: &[u8], from: Option<&str>, to: &str) -> Result<ConvertOutput, String> {
    match catch_unwind(AssertUnwindSafe(|| convert(input, &options(from, to)))) {
        Ok(Ok(out)) => Ok(out),
        Ok(Err(e)) => Err(format!("convert returned Err: {e}")),
        Err(panic) => {
            let msg = panic
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| panic.downcast_ref::<&str>().map(|s| s.to_string()))
                .unwrap_or_default();
            Err(format!("PANIC: {msg}"))
        }
    }
}

fn read_zip(bytes: &[u8]) -> Result<Vec<(String, Vec<u8>)>, String> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| format!("output zip unreadable: {e}"))?;
    let mut files = Vec::new();
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| format!("zip entry {i} unreadable: {e}"))?;
        if entry.is_dir() {
            continue;
        }
        let name = entry.name().map_err(|e| format!("zip entry {i} has bad name: {e}"))?.into_owned();
        let mut data = Vec::new();
        entry.read_to_end(&mut data).map_err(|e| format!("zip entry {name} unreadable: {e}"))?;
        files.push((name, data));
    }
    Ok(files)
}

fn is_json_model(path: &str) -> bool {
    let mut parts = path.split('/');
    parts.next() == Some("assets") && parts.nth(1) == Some("models") && path.ends_with(".json")
}

/// Splits `ns:path` (namespace defaults to `minecraft`).
fn split_ref(reference: &str) -> (&str, &str) {
    reference.split_once(':').unwrap_or(("minecraft", reference))
}

fn collect_texture_refs(value: &Value, out: &mut Vec<String>) {
    if let Some(textures) = value.get("textures").and_then(Value::as_object) {
        out.extend(textures.values().filter_map(Value::as_str).filter(|s| !s.starts_with('#')).map(String::from));
    }
}

/// Checks one conversion result and returns every problem found.
fn check_output(zip: &[u8], target: &str, source_paths: &BTreeSet<String>) -> Vec<String> {
    let mut problems = Vec::new();
    let files = match read_zip(zip) {
        Ok(f) => f,
        Err(e) => return vec![e],
    };
    let paths: BTreeSet<&str> = files.iter().map(|(p, _)| p.as_str()).collect();

    let bad: Vec<&str> = paths.iter().copied().filter(|p| p.split('/').any(|s| s == "..") || p.starts_with('/')).collect();
    if !bad.is_empty() {
        problems.push(format!("paths with `..` or absolute: {bad:?}"));
    }

    let expected = pack_format(target.parse::<Version>().unwrap()).unwrap();
    match files.iter().find(|(p, _)| p == "pack.mcmeta") {
        None => problems.push("pack.mcmeta missing".into()),
        Some((_, data)) => match serde_json::from_slice::<Value>(data) {
            Err(e) => problems.push(format!("pack.mcmeta does not parse: {e}")),
            Ok(meta) => {
                // Newer formats may drop `pack_format` for `min_format`/`max_format` (a number or `[major, minor]`).
                let major = |v: &Value| v.as_u64().or_else(|| v.get(0).and_then(Value::as_u64));
                let pack = &meta["pack"];
                let declared = pack.get("pack_format").and_then(major).or_else(|| pack.get("min_format").and_then(major));
                if declared != Some(u64::from(expected.0)) {
                    problems.push(format!("pack format != expected {}; pack section: {}", expected.0, pack));
                }
            }
        },
    }

    // Vanilla textures cannot be enumerated, so a `minecraft:` reference that is absent from the output is
    // only an error when the source pack had that texture (conversion lost it) or when the reference uses
    // the wrong directory name for the target era (`blocks`/`items` before 1.13, `block`/`item` after).
    let flattening_done = expected.0 > 3;
    let (stale_block, stale_item) = if flattening_done { ("blocks", "items") } else { ("block", "item") };
    let mut missing = Vec::new();
    for (path, data) in files.iter().filter(|(p, _)| is_json_model(p)) {
        // The game's lenient parser ignores anything after the first JSON value (packs append `//` credits).
        let first_value = serde_json::Deserializer::from_slice(data).into_iter::<Value>().next();
        let Some(Ok(model)) = first_value else {
            problems.push(format!("model JSON does not parse: {path}"));
            continue;
        };
        let mut refs = Vec::new();
        collect_texture_refs(&model, &mut refs);
        for reference in refs {
            let (ns, tex) = split_ref(&reference);
            let file = format!("assets/{ns}/textures/{tex}.png");
            if paths.contains(file.as_str()) {
                continue;
            }
            let first = tex.split('/').next().unwrap_or("");
            let wrong_dir = ns == "minecraft" && (first == stale_block || first == stale_item);
            let lost = source_paths.contains(&file);
            let ns_dir = format!("assets/{ns}/textures/");
            let ns_has_textures = paths.iter().any(|p| p.starts_with(&ns_dir));
            if wrong_dir || lost || (ns != "minecraft" && ns_has_textures) {
                missing.push(format!("{path} -> {reference}{}", if wrong_dir { " (wrong dir for target)" } else { "" }));
            }
        }
    }
    if !missing.is_empty() {
        let shown: Vec<_> = missing.iter().take(MAX_REPORTED_PATHS).cloned().collect();
        problems.push(format!(
            "{} dangling texture reference(s), first {}:\n      {}",
            missing.len(),
            shown.len(),
            shown.join("\n      ")
        ));
    }
    problems
}

fn fixture_files(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("cannot read PACKCONV_FIXTURES {}: {e}", dir.display()))
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| matches!(p.extension().and_then(|e| e.to_str()), Some("zip" | "rar")))
        .collect();
    files.sort();
    files
}

fn summary(label: &str, out: &ConvertOutput) -> String {
    let r = &out.report;
    format!(
        "  {label:<22} {}->{} steps={} warnings={} files {}->{}",
        r.from,
        r.to,
        r.steps.len(),
        r.warnings.len(),
        r.files_in,
        r.files_out
    )
}

#[test]
fn e2e_fixtures() {
    let Some(dir) = std::env::var_os("PACKCONV_FIXTURES") else {
        eprintln!("PACKCONV_FIXTURES not set, skipping e2e");
        return;
    };
    let newest = supported_versions().last().cloned().expect("at least one supported version");
    let mut failures: Vec<String> = Vec::new();

    for path in fixture_files(Path::new(&dir)) {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let input = std::fs::read(&path).unwrap();
        println!("{name} ({} KiB)", input.len() / 1024);
        let source_paths: BTreeSet<String> = match read_archive(&input, Limits::default()) {
            Ok(pack) => pack.paths().map(String::from).collect(),
            Err(e) => {
                failures.push(format!("{name}: cannot read source: {e}"));
                continue;
            }
        };

        let mut targets: Vec<&str> = TARGETS.to_vec();
        targets.push(&newest);
        let mut detected_from = None;
        for target in &targets {
            let label = format!("to {target}");
            match run(&input, None, target) {
                Err(e) => {
                    println!("  {label:<22} FAILED");
                    failures.push(format!("{name} {label}: {e}"));
                }
                Ok(out) => {
                    println!("{}", summary(&label, &out));
                    detected_from = Some(out.report.from.clone());
                    for p in check_output(&out.zip, target, &source_paths) {
                        failures.push(format!("{name} {label}: {p}"));
                    }
                }
            }
        }

        let Some(source) = detected_from else { continue };
        let up = run(&input, Some(&source), &newest);
        let round = up.and_then(|up| run(&up.zip, Some(&newest), &source).map(|down| (up, down)));
        match round {
            Err(e) => {
                println!("  roundtrip {source}->{newest}->{source} FAILED");
                failures.push(format!("{name} roundtrip: {e}"));
            }
            Ok((up, down)) => {
                println!("{}", summary("roundtrip up", &up));
                println!("{}", summary("roundtrip down", &down));
                for p in check_output(&down.zip, &source, &source_paths) {
                    failures.push(format!("{name} roundtrip {source}->{newest}->{source}: {p}"));
                }
            }
        }
    }

    assert!(failures.is_empty(), "{} e2e failure(s):\n  {}", failures.len(), failures.join("\n  "));
}

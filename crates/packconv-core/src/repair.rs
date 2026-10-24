//! Repair mode: re-saves valid-but-badly-encoded pack files in a form every game version accepts.
//!
//! It never changes versions or the layout. PNGs are normalized to 8-bit non-interlaced RGB(A)
//! without ancillary chunks, images disguised as `.png` are re-encoded as PNG, and JSON-ish files
//! are made strictly valid. Files that are already clean are left byte-identical.
//!
//! Why: 1.20.1 and older tolerate quirks that newer clients reject (research notes, see report).
//! Mojang's reader (`NativeImage`, stb_image) is documented to handle 16-bit, interlaced and
//! palette PNGs, so those are normalized defensively rather than because a specific bug was
//! confirmed; JSON became stricter when the game moved to strict Gson/codec parsing.

use std::io::Cursor;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use image::ImageFormat;
use serde::Serialize;
use serde_json::{Map, Value};

use crate::archive::{read_archive, write_zip, Limits};
use crate::error::{Error, Result};
use crate::util::MAX_IMAGE_PIXELS;
use crate::vfs::Pack;

#[derive(Debug, Clone)]
pub struct RepairOptions {
    pub limits: Limits,
    /// Set by the caller to abort a running repair (checked per file and every few rows).
    pub cancel: Option<Arc<AtomicBool>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RepairedFile {
    pub path: String,
    pub issues: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RepairReport {
    pub repaired: Vec<RepairedFile>,
    pub warnings: Vec<String>,
    pub files: usize,
}

#[derive(Debug)]
pub struct RepairOutput {
    pub zip: Vec<u8>,
    pub report: RepairReport,
}

/// Repairs a ZIP/RAR resource pack archive and returns the result as a ZIP.
pub fn repair(input: &[u8], options: &RepairOptions) -> Result<RepairOutput> {
    let mut pack = read_archive(input, options.limits)?;
    let never = AtomicBool::new(false);
    let cancel = options.cancel.as_deref().unwrap_or(&never);
    let report = repair_pack(&mut pack, cancel)?;
    Ok(RepairOutput { zip: write_zip(&pack)?, report })
}

/// Same rule as the converter: vanilla namespaces only, no OptiFine/MCPatcher folders.
fn is_official(path: &str) -> bool {
    if path == "pack.mcmeta" || path == "pack.png" {
        return true;
    }
    let Some(rest) = path.strip_prefix("assets/minecraft/").or_else(|| path.strip_prefix("assets/realms/")) else {
        return false;
    };
    !(rest.starts_with("optifine/") || rest.starts_with("mcpatcher/"))
}

/// What happened to one file.
enum Outcome {
    Clean,
    Fixed(Vec<u8>, Vec<String>),
    /// Could not be repaired; the file is left as it was.
    Failed(String),
}

/// Repairs an unpacked pack in place. Fails with [`Error::Cancelled`] once `cancel` is set.
pub fn repair_pack(pack: &mut Pack, cancel: &AtomicBool) -> Result<RepairReport> {
    let mut report = RepairReport { repaired: Vec::new(), warnings: Vec::new(), files: pack.len() };
    let paths: Vec<String> = pack.paths().filter(|p| is_official(p)).map(String::from).collect();
    for path in paths {
        check_cancel(cancel)?;
        let bytes = pack.get(&path).expect("path was just listed");
        let outcome = if path.ends_with(".png") {
            repair_image(bytes, cancel)?
        } else if path.ends_with(".json") || path.ends_with(".mcmeta") {
            let dims = path
                .strip_suffix(".mcmeta")
                .filter(|png| png.ends_with(".png"))
                .and_then(|png| pack.get(png))
                .and_then(png_dimensions);
            repair_json(bytes, dims)
        } else {
            continue;
        };
        match outcome {
            Outcome::Clean => {}
            Outcome::Fixed(data, issues) => {
                pack.insert(path.clone(), data);
                report.repaired.push(RepairedFile { path, issues });
            }
            Outcome::Failed(reason) => report.warnings.push(format!("{path}: {reason}")),
        }
    }
    Ok(report)
}

fn check_cancel(cancel: &AtomicBool) -> Result<()> {
    if cancel.load(Ordering::Relaxed) {
        Err(Error::Cancelled)
    } else {
        Ok(())
    }
}

// ---------------------------------------------------------------------------------------------
// PNG
// ---------------------------------------------------------------------------------------------

const PNG_SIGNATURE: [u8; 8] = *b"\x89PNG\r\n\x1a\n";

/// Width and height from the IHDR of a PNG, without decoding anything.
fn png_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 24 || bytes[..8] != PNG_SIGNATURE || &bytes[12..16] != b"IHDR" {
        return None;
    }
    let width = u32::from_be_bytes(bytes[16..20].try_into().ok()?);
    let height = u32::from_be_bytes(bytes[20..24].try_into().ok()?);
    Some((width, height))
}

struct Chunk<'a> {
    kind: [u8; 4],
    data: &'a [u8],
    /// The whole chunk as stored: length, type, data and CRC.
    raw: &'a [u8],
    crc_ok: bool,
}

impl Chunk<'_> {
    fn is(&self, kind: &[u8; 4]) -> bool {
        &self.kind == kind
    }

    /// Chunks that must survive when only stripping metadata (palette is optional for RGB).
    fn is_essential(&self) -> bool {
        self.is(b"IHDR") || self.is(b"IDAT") || self.is(b"IEND")
    }

    fn is_critical(&self) -> bool {
        self.kind[0].is_ascii_uppercase()
    }
}

struct Scan<'a> {
    chunks: Vec<Chunk<'a>>,
    has_iend: bool,
    trailing_data: bool,
}

/// Splits a PNG into chunks without decoding; stops at IEND or at the first malformed chunk.
fn scan_chunks(bytes: &[u8]) -> Scan<'_> {
    let mut scan = Scan { chunks: Vec::new(), has_iend: false, trailing_data: false };
    let mut pos = PNG_SIGNATURE.len();
    while pos + 12 <= bytes.len() {
        let len = u32::from_be_bytes(bytes[pos..pos + 4].try_into().expect("4 bytes")) as usize;
        let Some(end) = pos.checked_add(12 + len).filter(|&end| end <= bytes.len()) else {
            break;
        };
        let kind: [u8; 4] = bytes[pos + 4..pos + 8].try_into().expect("4 bytes");
        let stored_crc = u32::from_be_bytes(bytes[end - 4..end].try_into().expect("4 bytes"));
        scan.chunks.push(Chunk {
            kind,
            data: &bytes[pos + 8..end - 4],
            raw: &bytes[pos..end],
            crc_ok: crc32(&bytes[pos + 4..end - 4]) == stored_crc,
        });
        pos = end;
        if &kind == b"IEND" {
            scan.has_iend = true;
            scan.trailing_data = pos < bytes.len();
            break;
        }
    }
    scan
}

fn crc32(data: &[u8]) -> u32 {
    const TABLE: [u32; 256] = {
        let mut table = [0u32; 256];
        let mut i = 0;
        while i < 256 {
            let mut c = i as u32;
            let mut k = 0;
            while k < 8 {
                c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
                k += 1;
            }
            table[i] = c;
            i += 1;
        }
        table
    };
    !data.iter().fold(!0u32, |crc, &b| TABLE[((crc ^ u32::from(b)) & 0xFF) as usize] ^ (crc >> 8))
}

fn repair_image(bytes: &[u8], cancel: &AtomicBool) -> Result<Outcome> {
    if bytes.is_empty() {
        return Ok(Outcome::Failed("empty file".into()));
    }
    if bytes.len() < PNG_SIGNATURE.len() || bytes[..8] != PNG_SIGNATURE {
        return Ok(reencode_foreign_image(bytes));
    }
    repair_png(bytes, cancel)
}

/// JPEG, GIF, BMP or WebP data saved under a `.png` name.
fn reencode_foreign_image(bytes: &[u8]) -> Outcome {
    let format = match image::guess_format(bytes) {
        Ok(f @ (ImageFormat::Jpeg | ImageFormat::Gif | ImageFormat::Bmp | ImageFormat::WebP)) => f,
        _ => return Outcome::Failed("not a PNG and not a recognized image format".into()),
    };
    let reader = || image::ImageReader::with_format(Cursor::new(bytes), format);
    let decoded = reader()
        .into_dimensions()
        .map_err(|e| e.to_string())
        .and_then(|(w, h)| {
            if w == 0 || h == 0 || u64::from(w) * u64::from(h) > MAX_IMAGE_PIXELS {
                Err(format!("unsupported image size {w}x{h}"))
            } else {
                Ok(())
            }
        })
        .and_then(|()| reader().decode().map_err(|e| e.to_string()));
    let img = match decoded {
        Ok(img) => img.into_rgba8(),
        Err(e) => return Outcome::Failed(format!("{format:?} data with a .png name could not be decoded: {e}")),
    };
    let mut out = Cursor::new(Vec::new());
    match img.write_to(&mut out, ImageFormat::Png) {
        Ok(()) => Outcome::Fixed(out.into_inner(), vec![format!("{format:?} data saved with a .png name")]),
        Err(e) => Outcome::Failed(format!("could not encode as PNG: {e}")),
    }
}

fn repair_png(bytes: &[u8], cancel: &AtomicBool) -> Result<Outcome> {
    let scan = scan_chunks(bytes);
    let Some(ihdr) = scan.chunks.first().filter(|c| c.is(b"IHDR") && c.data.len() == 13) else {
        return Ok(Outcome::Failed("PNG has no valid IHDR".into()));
    };
    if !scan.has_iend {
        return Ok(Outcome::Failed("PNG is truncated (no IEND)".into()));
    }
    let width = u32::from_be_bytes(ihdr.data[0..4].try_into().expect("4 bytes"));
    let height = u32::from_be_bytes(ihdr.data[4..8].try_into().expect("4 bytes"));
    let (depth, color_type, interlaced) = (ihdr.data[8], ihdr.data[9], ihdr.data[12] != 0);

    let mut issues = Vec::new();
    let mut extra: Vec<String> = Vec::new();
    for chunk in &scan.chunks {
        let name = String::from_utf8_lossy(&chunk.kind).into_owned();
        if !chunk.is_essential() && !extra.contains(&name) {
            extra.push(name);
        }
    }
    let has_trns = scan.chunks.iter().any(|c| c.is(b"tRNS"));
    let critical_crc_bad = scan.chunks.iter().any(|c| c.is_critical() && !c.crc_ok);
    // A color-key tRNS on RGB data is transparency that stripping chunks would lose.
    let standard = depth == 8 && matches!(color_type, 2 | 6) && !interlaced && !(color_type == 2 && has_trns);

    if standard && !critical_crc_bad {
        if extra.is_empty() && !scan.trailing_data {
            return Ok(Outcome::Clean);
        }
        // Only drop chunks: the compressed pixel data is copied as is.
        if !extra.is_empty() {
            issues.push(format!("removed non-essential chunks: {}", extra.join(", ")));
        }
        if scan.trailing_data {
            issues.push("removed trailing data after IEND".into());
        }
        let mut out = PNG_SIGNATURE.to_vec();
        for chunk in scan.chunks.iter().filter(|c| c.is_essential()) {
            out.extend_from_slice(chunk.raw);
        }
        return Ok(Outcome::Fixed(out, issues));
    }

    if depth == 16 {
        issues.push("16-bit depth".into());
    }
    if interlaced {
        issues.push("interlaced (Adam7)".into());
    }
    match color_type {
        0 => issues.push("grayscale".into()),
        3 => issues.push("palette".into()),
        4 => issues.push("grayscale with alpha".into()),
        2 if has_trns => issues.push("RGB with color-key transparency (tRNS)".into()),
        _ => {}
    }
    if depth < 8 && color_type != 3 {
        issues.push(format!("{depth}-bit depth"));
    }
    if critical_crc_bad {
        issues.push("CRC errors".into());
    }
    let ancillary: Vec<&str> = extra.iter().map(String::as_str).filter(|n| !matches!(*n, "PLTE" | "tRNS")).collect();
    if !ancillary.is_empty() {
        issues.push(format!("removed non-essential chunks: {}", ancillary.join(", ")));
    }
    if scan.trailing_data {
        issues.push("removed trailing data after IEND".into());
    }

    match reencode_png(bytes, width, height, interlaced, critical_crc_bad, cancel) {
        Ok(data) => Ok(Outcome::Fixed(data, issues)),
        Err(Stop::Cancelled) => Err(Error::Cancelled),
        Err(Stop::Failed(reason)) => Ok(Outcome::Failed(reason)),
    }
}

enum Stop {
    Cancelled,
    Failed(String),
}

impl From<png::DecodingError> for Stop {
    fn from(e: png::DecodingError) -> Self {
        Stop::Failed(format!("PNG cannot be decoded: {e}"))
    }
}

impl From<png::EncodingError> for Stop {
    fn from(e: png::EncodingError) -> Self {
        Stop::Failed(format!("PNG cannot be encoded: {e}"))
    }
}

impl From<std::io::Error> for Stop {
    fn from(e: std::io::Error) -> Self {
        Stop::Failed(format!("PNG cannot be encoded: {e}"))
    }
}

/// Rows between cancellation checks while streaming.
const CANCEL_CHECK_ROWS: u32 = 256;

/// Re-encodes any PNG as 8-bit non-interlaced RGBA. Non-interlaced input is streamed row by
/// row, so memory stays proportional to the width however tall the image is (animated
/// textures can be 100000 px high). Interlaced input has to be deinterlaced in memory first.
fn reencode_png(
    bytes: &[u8],
    width: u32,
    height: u32,
    interlaced: bool,
    ignore_checksums: bool,
    cancel: &AtomicBool,
) -> std::result::Result<Vec<u8>, Stop> {
    if interlaced && u64::from(width) * u64::from(height) > MAX_IMAGE_PIXELS {
        return Err(Stop::Failed(format!("interlaced image {width}x{height} is too large to repair")));
    }
    let limits = png::Limits { bytes: (MAX_IMAGE_PIXELS as usize) * 4 };
    let mut decoder = png::Decoder::new_with_limits(Cursor::new(bytes), limits);
    // Expand palette, tRNS and low bit depths; keep 8 bits per sample.
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    // Broken or huge color profiles and text must not block recovering the pixels.
    decoder.set_ignore_iccp_chunk(true);
    decoder.set_ignore_text_chunk(true);
    decoder.ignore_checksums(ignore_checksums);
    let mut reader = decoder.read_info()?;
    let channels = match reader.output_color_type() {
        (png::ColorType::Grayscale, png::BitDepth::Eight) => 1,
        (png::ColorType::GrayscaleAlpha, png::BitDepth::Eight) => 2,
        (png::ColorType::Rgb, png::BitDepth::Eight) => 3,
        (png::ColorType::Rgba, png::BitDepth::Eight) => 4,
        other => return Err(Stop::Failed(format!("unexpected PNG output format {other:?}"))),
    };

    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(png::Compression::Fast);
    let mut writer = encoder.write_header()?;
    let mut stream = writer.stream_writer()?;

    let mut rgba_row = Vec::with_capacity(width as usize * 4);
    let mut written = 0u32;
    let mut write_row = |src: &[u8]| -> std::result::Result<(), Stop> {
        if written.is_multiple_of(CANCEL_CHECK_ROWS) && cancel.load(Ordering::Relaxed) {
            return Err(Stop::Cancelled);
        }
        written += 1;
        rgba_row.clear();
        match channels {
            1 => src.iter().for_each(|&g| rgba_row.extend_from_slice(&[g, g, g, 255])),
            2 => src.chunks_exact(2).for_each(|p| rgba_row.extend_from_slice(&[p[0], p[0], p[0], p[1]])),
            3 => src.chunks_exact(3).for_each(|p| rgba_row.extend_from_slice(&[p[0], p[1], p[2], 255])),
            _ => rgba_row.extend_from_slice(src),
        }
        std::io::Write::write_all(&mut stream, &rgba_row)?;
        Ok(())
    };

    if interlaced {
        let mut frame = vec![0u8; reader.output_buffer_size().ok_or_else(|| Stop::Failed("image too large".into()))?];
        let info = reader.next_frame(&mut frame)?;
        for row in frame[..info.buffer_size()].chunks_exact(info.line_size) {
            write_row(row)?;
        }
    } else {
        while let Some(row) = reader.next_row()? {
            write_row(row.data())?;
        }
    }
    stream.finish()?;
    writer.finish()?;
    Ok(out)
}

// ---------------------------------------------------------------------------------------------
// JSON
// ---------------------------------------------------------------------------------------------

/// Makes a JSON-ish file strictly valid. `png_dims` is the size of the sibling texture when the
/// file is a `.png.mcmeta`, used to sanity-check the animation.
fn repair_json(bytes: &[u8], png_dims: Option<(u32, u32)>) -> Outcome {
    let mut issues = Vec::new();
    let mut value = match serde_json::from_slice::<Value>(bytes) {
        Ok(value) => value,
        Err(_) => match parse_leniently(bytes, &mut issues) {
            Ok(value) => value,
            Err(reason) => return Outcome::Failed(reason),
        },
    };
    if let Some(animation) = value.get_mut("animation").and_then(Value::as_object_mut) {
        fix_animation(animation, png_dims, &mut issues);
    }
    if issues.is_empty() {
        return Outcome::Clean;
    }
    Outcome::Fixed(serde_json::to_vec_pretty(&value).expect("JSON value serializes"), issues)
}

fn parse_leniently(bytes: &[u8], issues: &mut Vec<String>) -> std::result::Result<Value, String> {
    let text = std::str::from_utf8(bytes).map_err(|_| "not valid UTF-8, left unchanged".to_string())?;
    if let Some(rest) = text.strip_prefix('\u{feff}') {
        issues.push("UTF-8 byte order mark".into());
        return parse_leniently_text(rest, issues);
    }
    parse_leniently_text(text, issues)
}

fn parse_leniently_text(text: &str, issues: &mut Vec<String>) -> std::result::Result<Value, String> {
    let (cleaned, found) = strip_lenient_syntax(text);
    let mut values = serde_json::Deserializer::from_str(&cleaned).into_iter::<Value>();
    let value = match values.next() {
        None => return Err("empty file".into()),
        Some(Err(e)) => return Err(format!("not valid JSON even leniently, left unchanged: {e}")),
        Some(Ok(value)) => value,
    };
    issues.extend(found);
    if !cleaned[values.byte_offset()..].trim().is_empty() {
        issues.push("removed trailing garbage after the JSON value".into());
    }
    Ok(value)
}

/// Removes `//` and `/* */` comments and trailing commas and turns single-quoted strings into
/// double-quoted ones, leaving the contents of double-quoted strings alone.
fn strip_lenient_syntax(src: &str) -> (String, Vec<String>) {
    let mut out = String::with_capacity(src.len());
    let (mut comments, mut commas, mut quotes) = (false, false, false);
    // Position in `out` of a comma not yet followed by another token.
    let mut pending_comma: Option<usize> = None;
    let mut chars = src.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                pending_comma = None;
                out.push(c);
                while let Some(d) = chars.next() {
                    out.push(d);
                    if d == '\\' {
                        out.extend(chars.next());
                    } else if d == '"' {
                        break;
                    }
                }
            }
            '\'' => {
                pending_comma = None;
                quotes = true;
                out.push('"');
                while let Some(d) = chars.next() {
                    match d {
                        '\\' => match chars.next() {
                            Some('\'') => out.push('\''),
                            Some(e) => {
                                out.push('\\');
                                out.push(e);
                            }
                            None => {}
                        },
                        '\'' => break,
                        '"' => out.push_str("\\\""),
                        _ => out.push(d),
                    }
                }
                out.push('"');
            }
            '/' if chars.peek() == Some(&'/') => {
                comments = true;
                while chars.next_if(|&d| d != '\n').is_some() {}
            }
            '/' if chars.peek() == Some(&'*') => {
                comments = true;
                chars.next();
                let mut prev = ' ';
                for d in chars.by_ref() {
                    if prev == '*' && d == '/' {
                        break;
                    }
                    prev = d;
                }
                out.push(' ');
            }
            ',' => {
                pending_comma = Some(out.len());
                out.push(c);
            }
            '}' | ']' => {
                if let Some(at) = pending_comma.take() {
                    out.remove(at);
                    commas = true;
                }
                out.push(c);
            }
            c if c.is_whitespace() => out.push(c),
            _ => {
                pending_comma = None;
                out.push(c);
            }
        }
    }
    let mut found = Vec::new();
    if comments {
        found.push("removed comments".to_string());
    }
    if commas {
        found.push("removed trailing commas".to_string());
    }
    if quotes {
        found.push("converted single-quoted strings".to_string());
    }
    (out, found)
}

/// Fixes values the game rejects in an animation `.mcmeta`: a non-positive `frametime` and
/// `frames` entries pointing past the end of the texture strip.
fn fix_animation(animation: &mut Map<String, Value>, png_dims: Option<(u32, u32)>, issues: &mut Vec<String>) {
    if animation.get("frametime").and_then(Value::as_f64).is_some_and(|t| t <= 0.0) {
        animation.insert("frametime".into(), Value::from(1));
        issues.push("animation frametime must be positive, set to 1".into());
    }
    let Some((width, height)) = png_dims.filter(|&(w, h)| w > 0 && h > 0) else {
        return;
    };
    let declared = |key: &str| animation.get(key).and_then(Value::as_u64).filter(|&v| v > 0);
    // Frame size as the game derives it: missing sides follow the image aspect ratio and the
    // default is a square the size of the shorter side.
    let (frame_w, frame_h) = match (declared("width"), declared("height")) {
        (Some(w), Some(h)) => (w, h),
        (Some(w), None) => (w, (u64::from(height) * w / u64::from(width)).max(1)),
        (None, Some(h)) => ((u64::from(width) * h / u64::from(height)).max(1), h),
        (None, None) => {
            let side = u64::from(width.min(height));
            (side, side)
        }
    };
    let frame_count = (u64::from(width) / frame_w) * (u64::from(height) / frame_h);
    let Some(frames) = animation.get_mut("frames").and_then(Value::as_array_mut) else {
        return;
    };
    let before = frames.len();
    frames.retain(|frame| {
        let index = match frame {
            Value::Object(o) => o.get("index").and_then(Value::as_i64),
            other => other.as_i64(),
        };
        index.is_none_or(|i| u64::try_from(i).is_ok_and(|i| i < frame_count))
    });
    if frames.len() == before {
        return;
    }
    issues.push(format!(
        "removed {} animation frame(s) outside the texture ({frame_count} frames available)",
        before - frames.len()
    ));
    if frames.is_empty() {
        animation.remove("frames");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(pack: &mut Pack) -> RepairReport {
        repair_pack(pack, &AtomicBool::new(false)).unwrap()
    }

    fn encode(width: u32, height: u32, color: png::ColorType, depth: png::BitDepth, data: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut encoder = png::Encoder::new(&mut out, width, height);
        encoder.set_color(color);
        encoder.set_depth(depth);
        encoder.write_header().unwrap().write_image_data(data).unwrap();
        out
    }

    fn rgba_png() -> Vec<u8> {
        encode(2, 1, png::ColorType::Rgba, png::BitDepth::Eight, &[1, 2, 3, 4, 5, 6, 7, 8])
    }

    fn decode_rgba(bytes: &[u8]) -> (u32, u32, Vec<u8>) {
        let mut reader = png::Decoder::new(Cursor::new(bytes)).read_info().unwrap();
        assert_eq!(reader.output_color_type(), (png::ColorType::Rgba, png::BitDepth::Eight));
        let mut buf = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut buf).unwrap();
        (info.width, info.height, buf)
    }

    fn chunk_names(bytes: &[u8]) -> Vec<String> {
        scan_chunks(bytes).chunks.iter().map(|c| String::from_utf8_lossy(&c.kind).into_owned()).collect()
    }

    fn build_png(ihdr: [u8; 13], chunks: &[(&[u8; 4], Vec<u8>)]) -> Vec<u8> {
        let mut out = PNG_SIGNATURE.to_vec();
        let mut push = |kind: &[u8; 4], data: &[u8]| {
            out.extend_from_slice(&(data.len() as u32).to_be_bytes());
            let mut body = kind.to_vec();
            body.extend_from_slice(data);
            out.extend_from_slice(&body);
            out.extend_from_slice(&crc32(&body).to_be_bytes());
        };
        push(b"IHDR", &ihdr);
        for (kind, data) in chunks {
            push(kind, data);
        }
        push(b"IEND", &[]);
        out
    }

    /// zlib stream made of stored blocks, so tests can craft arbitrary raw scanlines.
    fn zlib_stored(raw: &[u8]) -> Vec<u8> {
        let mut out = vec![0x78, 0x01, 0x01];
        out.extend_from_slice(&(raw.len() as u16).to_le_bytes());
        out.extend_from_slice(&(!(raw.len() as u16)).to_le_bytes());
        out.extend_from_slice(raw);
        let (mut a, mut b) = (1u32, 0u32);
        for &x in raw {
            a = (a + u32::from(x)) % 65521;
            b = (b + a) % 65521;
        }
        out.extend_from_slice(&((b << 16) | a).to_be_bytes());
        out
    }

    fn ihdr(width: u32, height: u32, depth: u8, color: u8, interlace: u8) -> [u8; 13] {
        let mut h = [0u8; 13];
        h[0..4].copy_from_slice(&width.to_be_bytes());
        h[4..8].copy_from_slice(&height.to_be_bytes());
        h[8] = depth;
        h[9] = color;
        h[12] = interlace;
        h
    }

    fn assert_same(a: &Pack, b: &Pack) {
        assert!(a.iter().eq(b.iter()), "packs differ");
    }

    fn pack_with(path: &str, data: Vec<u8>) -> Pack {
        let mut pack = Pack::new();
        pack.insert(path, data);
        pack
    }

    const TEX: &str = "assets/minecraft/textures/block/a.png";

    #[test]
    fn clean_pack_is_untouched() {
        let mut pack = Pack::new();
        pack.insert("pack.mcmeta", br#"{"pack":{"pack_format":15,"description":"x"}}"#.to_vec());
        pack.insert(TEX, rgba_png());
        pack.insert("assets/minecraft/textures/block/b.png", encode(1, 1, png::ColorType::Rgb, png::BitDepth::Eight, &[1, 2, 3]));
        pack.insert("assets/minecraft/models/block/a.json", b"{ \"parent\" :  \"block/cube\" }".to_vec());
        pack.insert("assets/minecraft/lang/en_us.lang", b"x=y".to_vec());
        let before = pack.clone();
        let report = run(&mut pack);
        assert!(report.repaired.is_empty() && report.warnings.is_empty());
        assert_eq!(report.files, 5);
        assert_same(&pack, &before);
    }

    #[test]
    fn non_official_files_pass_through() {
        let mut pack = Pack::new();
        for path in ["assets/mod/textures/a.png", "assets/minecraft/optifine/a.png", "assets/minecraft/mcpatcher/a.json"] {
            pack.insert(path, b"garbage".to_vec());
        }
        let before = pack.clone();
        let report = run(&mut pack);
        assert!(report.repaired.is_empty() && report.warnings.is_empty());
        assert_same(&pack, &before);
    }

    #[test]
    fn strips_ancillary_chunks_without_reencoding() {
        let mut out = Vec::new();
        let mut encoder = png::Encoder::new(&mut out, 2, 1);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_source_gamma(png::ScaledFloat::new(0.45455));
        encoder.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
        encoder.add_text_chunk("Comment".into(), "hi".into()).unwrap();
        encoder.write_header().unwrap().write_image_data(&[1, 2, 3, 4, 5, 6, 7, 8]).unwrap();
        out.extend_from_slice(b"junk after IEND");
        assert!(chunk_names(&out).len() > 3);

        let mut pack = pack_with(TEX, out.clone());
        let report = run(&mut pack);
        let fixed = pack.get(TEX).unwrap();
        assert_eq!(chunk_names(fixed), ["IHDR", "IDAT", "IEND"]);
        assert_eq!(decode_rgba(fixed), (2, 1, vec![1, 2, 3, 4, 5, 6, 7, 8]));
        let issues = &report.repaired[0].issues;
        assert!(issues[0].contains("gAMA") && issues.iter().any(|i| i.contains("trailing")), "{issues:?}");
        // Idempotent: the second run finds nothing to do.
        assert!(run(&mut pack).repaired.is_empty());
    }

    #[test]
    fn sixteen_bit_becomes_eight_bit_rgba() {
        let data = [0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xde, 0xf0];
        let mut pack = pack_with(TEX, encode(1, 1, png::ColorType::Rgba, png::BitDepth::Sixteen, &data));
        let report = run(&mut pack);
        assert_eq!(decode_rgba(pack.get(TEX).unwrap()), (1, 1, vec![0x12, 0x56, 0x9a, 0xde]));
        assert_eq!(report.repaired[0].issues, ["16-bit depth"]);
    }

    #[test]
    fn grayscale_and_gray_alpha_expand() {
        let mut pack = pack_with(TEX, encode(2, 1, png::ColorType::Grayscale, png::BitDepth::Eight, &[10, 20]));
        pack.insert("assets/minecraft/textures/block/b.png", encode(1, 1, png::ColorType::GrayscaleAlpha, png::BitDepth::Eight, &[7, 9]));
        run(&mut pack);
        assert_eq!(decode_rgba(pack.get(TEX).unwrap()).2, [10, 10, 10, 255, 20, 20, 20, 255]);
        assert_eq!(decode_rgba(pack.get("assets/minecraft/textures/block/b.png").unwrap()).2, [7, 7, 7, 9]);
    }

    #[test]
    fn palette_with_transparency_expands() {
        let png = build_png(
            ihdr(2, 1, 8, 3, 0),
            &[
                (b"PLTE", vec![255, 0, 0, 0, 255, 0]),
                (b"tRNS", vec![128]),
                (b"IDAT", zlib_stored(&[0, 0, 1])),
            ],
        );
        let mut pack = pack_with(TEX, png);
        let report = run(&mut pack);
        assert_eq!(decode_rgba(pack.get(TEX).unwrap()).2, [255, 0, 0, 128, 0, 255, 0, 255]);
        assert_eq!(report.repaired[0].issues, ["palette"]);
    }

    #[test]
    fn rgb_color_key_transparency_is_kept() {
        let png = build_png(
            ihdr(2, 1, 8, 2, 0),
            &[(b"tRNS", vec![0, 1, 0, 2, 0, 3]), (b"IDAT", zlib_stored(&[0, 1, 2, 3, 9, 9, 9]))],
        );
        let mut pack = pack_with(TEX, png);
        run(&mut pack);
        assert_eq!(decode_rgba(pack.get(TEX).unwrap()).2, [1, 2, 3, 0, 9, 9, 9, 255]);
    }

    #[test]
    fn interlaced_is_deinterlaced() {
        // 2x2 gray image: Adam7 passes 1, 6 and 7 hold (0,0), (1,0) and row 1.
        let raw = [0, 10, 0, 20, 0, 30, 40];
        let png = build_png(ihdr(2, 2, 8, 0, 1), &[(b"IDAT", zlib_stored(&raw))]);
        let mut pack = pack_with(TEX, png);
        let report = run(&mut pack);
        let gray: Vec<u8> = decode_rgba(pack.get(TEX).unwrap()).2.chunks(4).map(|p| p[0]).collect();
        assert_eq!(gray, [10, 20, 30, 40]);
        assert!(report.repaired[0].issues.contains(&"interlaced (Adam7)".to_string()));
    }

    #[test]
    fn crc_errors_are_recovered() {
        let mut bytes = rgba_png();
        // Corrupt the last byte of the IDAT chunk's CRC, which sits right before the 12-byte IEND.
        let at = bytes.len() - 12 - 1;
        bytes[at] ^= 0xFF;
        let mut pack = pack_with(TEX, bytes);
        let report = run(&mut pack);
        assert_eq!(decode_rgba(pack.get(TEX).unwrap()), (2, 1, vec![1, 2, 3, 4, 5, 6, 7, 8]));
        assert_eq!(report.repaired[0].issues, ["CRC errors"]);
    }

    #[test]
    fn foreign_formats_become_png() {
        let img = image::RgbaImage::from_pixel(3, 2, image::Rgba([200, 100, 50, 255]));
        let mut jpeg = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(img.clone()).to_rgb8().write_to(&mut jpeg, ImageFormat::Jpeg).unwrap();
        let mut bmp = Cursor::new(Vec::new());
        img.write_to(&mut bmp, ImageFormat::Bmp).unwrap();
        let mut pack = pack_with(TEX, jpeg.into_inner());
        pack.insert("assets/minecraft/textures/block/b.png", bmp.into_inner());
        let report = run(&mut pack);
        assert_eq!(report.repaired.len(), 2);
        assert_eq!(decode_rgba(pack.get(TEX).unwrap()).0, 3);
        assert_eq!(decode_rgba(pack.get("assets/minecraft/textures/block/b.png").unwrap()).1, 2);
    }

    #[test]
    fn broken_images_are_left_with_a_warning() {
        let mut pack = pack_with(TEX, Vec::new());
        pack.insert("assets/minecraft/textures/block/b.png", b"definitely not an image".to_vec());
        let mut truncated = rgba_png();
        truncated.truncate(truncated.len() - 20);
        pack.insert("assets/minecraft/textures/block/c.png", truncated);
        let before = pack.clone();
        let report = run(&mut pack);
        assert_eq!(report.warnings.len(), 3, "{:?}", report.warnings);
        assert!(report.repaired.is_empty());
        assert_same(&pack, &before);
    }

    #[test]
    fn tall_sixteen_bit_image_is_streamed() {
        const HEIGHT: u32 = 200_000;
        let mut source = Vec::new();
        let mut encoder = png::Encoder::new(&mut source, 64, HEIGHT);
        encoder.set_color(png::ColorType::Grayscale);
        encoder.set_depth(png::BitDepth::Sixteen);
        encoder.set_compression(png::Compression::Fast);
        let mut writer = encoder.write_header().unwrap();
        let mut stream = writer.stream_writer().unwrap();
        let row = [0x80u8; 128];
        for _ in 0..HEIGHT {
            std::io::Write::write_all(&mut stream, &row).unwrap();
        }
        stream.finish().unwrap();
        writer.finish().unwrap();

        let mut pack = pack_with(TEX, source);
        let report = run(&mut pack);
        let fixed = pack.get(TEX).unwrap();
        assert_eq!(png_dimensions(fixed), Some((64, HEIGHT)));
        assert_eq!(fixed[24..26], [8, 6], "8-bit RGBA");
        assert_eq!(report.repaired[0].issues, ["16-bit depth", "grayscale"]);
    }

    #[test]
    fn cancel_aborts() {
        let mut pack = pack_with(TEX, rgba_png());
        let err = repair_pack(&mut pack, &AtomicBool::new(true)).unwrap_err();
        assert!(matches!(err, Error::Cancelled));
    }

    fn json_fixed(input: &[u8]) -> (String, Vec<String>) {
        match repair_json(input, None) {
            Outcome::Fixed(data, issues) => (String::from_utf8(data).unwrap(), issues),
            Outcome::Clean => panic!("unexpectedly clean"),
            Outcome::Failed(r) => panic!("failed: {r}"),
        }
    }

    #[test]
    fn json_lenient_syntax_is_fixed_and_key_order_kept() {
        let input = b"\xEF\xBB\xBF// header\n{ \"z\": 1, /* mid */ \"a\": [1, 2,], 'k': 'it\\'s \"q\"', }\n}} trailing";
        let (text, issues) = json_fixed(input);
        let value: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["k"], "it's \"q\"");
        assert_eq!(value["a"], serde_json::json!([1, 2]));
        assert_eq!(value.as_object().unwrap().keys().collect::<Vec<_>>(), ["z", "a", "k"]);
        for expected in ["byte order mark", "comments", "trailing commas", "single-quoted", "trailing garbage"] {
            assert!(issues.iter().any(|i| i.contains(expected)), "{expected} in {issues:?}");
        }
    }

    #[test]
    fn json_strings_with_comment_markers_survive() {
        let (text, _) = json_fixed(br#"{"url": "http://x//y", "c": "/* no */", }"#);
        let value: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["url"], "http://x//y");
        assert_eq!(value["c"], "/* no */");
    }

    #[test]
    fn unparsable_json_is_left_with_a_warning() {
        let mut pack = pack_with("assets/minecraft/models/a.json", b"{\"a\": NaN}".to_vec());
        pack.insert("assets/minecraft/models/b.json", b"   ".to_vec());
        let before = pack.clone();
        let report = run(&mut pack);
        assert_eq!(report.warnings.len(), 2);
        assert_same(&pack, &before);
    }

    #[test]
    fn animation_is_sanity_checked() {
        let mut pack = pack_with(TEX, encode(2, 6, png::ColorType::Rgba, png::BitDepth::Eight, &[0; 48]));
        // 2x6 strip with square 2x2 frames: three frames.
        pack.insert(
            format!("{TEX}.mcmeta"),
            br#"{"animation":{"frametime":0,"frames":[0,{"index":2,"time":5},3,7]}}"#.to_vec(),
        );
        let report = run(&mut pack);
        let value: Value = serde_json::from_slice(pack.get(&format!("{TEX}.mcmeta")).unwrap()).unwrap();
        assert_eq!(value["animation"]["frametime"], 1);
        assert_eq!(value["animation"]["frames"], serde_json::json!([0, {"index": 2, "time": 5}]));
        assert_eq!(report.repaired.len(), 1);
        assert_eq!(report.repaired[0].issues.len(), 2);
    }

    #[test]
    fn valid_animation_is_untouched() {
        let mut pack = pack_with(TEX, encode(2, 4, png::ColorType::Rgba, png::BitDepth::Eight, &[0; 32]));
        pack.insert(format!("{TEX}.mcmeta"), br#"{"animation":{"frametime":2,"frames":[0,1]}}"#.to_vec());
        assert!(run(&mut pack).repaired.is_empty());
    }

    #[test]
    fn repair_roundtrips_through_zip() {
        let mut pack = Pack::new();
        pack.insert("pack.mcmeta", b"\xEF\xBB\xBF{\"pack\":{\"pack_format\":15,\"description\":\"x\"}}".to_vec());
        pack.insert(TEX, encode(1, 1, png::ColorType::Grayscale, png::BitDepth::Eight, &[5]));
        let zip = write_zip(&pack).unwrap();
        let options = RepairOptions { limits: Limits::default(), cancel: None };
        let output = repair(&zip, &options).unwrap();
        assert_eq!(output.report.repaired.len(), 2);
        let back = read_archive(&output.zip, Limits::default()).unwrap();
        assert_eq!(decode_rgba(back.get(TEX).unwrap()).2, [5, 5, 5, 255]);
        assert!(serde_json::from_slice::<Value>(back.get("pack.mcmeta").unwrap()).is_ok());
    }
}

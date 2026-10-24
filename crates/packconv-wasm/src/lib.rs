use packconv_core::{ConvertOptions, Limits, RepairOptions};
use wasm_bindgen::prelude::*;

/// wasm32 has a 4 GiB address space shared with the page, so unpacked size is capped lower
/// than on the server.
const WASM_LIMITS: Limits = Limits { max_entries: 100_000, max_total_size: 256 * 1024 * 1024 };

/// Result of a conversion or repair: the resulting pack as ZIP bytes plus a JSON report.
#[wasm_bindgen]
pub struct ConvertResult {
    zip: Vec<u8>,
    report: String,
}

#[wasm_bindgen]
impl ConvertResult {
    /// Converted pack as a ZIP archive (`Uint8Array`).
    #[wasm_bindgen(getter)]
    pub fn zip(&self) -> Vec<u8> {
        self.zip.clone()
    }

    /// Conversion report as a JSON string.
    #[wasm_bindgen(getter)]
    pub fn report(&self) -> String {
        self.report.clone()
    }
}

/// Converts a ZIP or RAR resource pack to version `to`. `from` is detected from
/// `pack.mcmeta` when omitted.
#[wasm_bindgen]
pub fn convert(input: &[u8], to: String, from: Option<String>) -> Result<ConvertResult, JsError> {
    let options = ConvertOptions { from, to, limits: WASM_LIMITS, cancel: None };
    let output = packconv_core::convert(input, &options)?;
    let report = serde_json::to_string(&output.report)?;
    Ok(ConvertResult { zip: output.zip, report })
}

/// Re-encodes badly saved PNG and JSON files without changing the pack version.
/// The returned `report` lists every repaired file and what was wrong with it.
#[wasm_bindgen]
pub fn repair(input: &[u8]) -> Result<ConvertResult, JsError> {
    let options = RepairOptions { limits: WASM_LIMITS, cancel: None };
    let output = packconv_core::repair(input, &options)?;
    let report = serde_json::to_string(&output.report)?;
    Ok(ConvertResult { zip: output.zip, report })
}

/// Version names accepted by `convert`, oldest first.
#[wasm_bindgen(js_name = supportedVersions)]
pub fn supported_versions() -> Vec<String> {
    packconv_core::supported_versions()
}

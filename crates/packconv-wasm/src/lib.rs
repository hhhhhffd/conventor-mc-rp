use packconv_core::{ConvertOptions, Limits};
use wasm_bindgen::prelude::*;

/// Result of a conversion: the converted pack as ZIP bytes plus a JSON report.
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
    let options = ConvertOptions { from, to, limits: Limits::default(), cancel: None };
    let output = packconv_core::convert(input, &options)?;
    let report = serde_json::to_string(&output.report)?;
    Ok(ConvertResult { zip: output.zip, report })
}

/// Version names accepted by `convert`, oldest first.
#[wasm_bindgen(js_name = supportedVersions)]
pub fn supported_versions() -> Vec<String> {
    packconv_core::supported_versions()
}

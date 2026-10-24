//! Minecraft Java Edition resource pack converter.
//!
//! Unpacks ZIP/RAR archives in memory (pure Rust, works on `wasm32`), converts the pack
//! between game versions in either direction through a chain of [`step::Step`]s and
//! repacks it as ZIP.

pub mod archive;
mod convert;
pub mod error;
pub mod meta;
mod rar;
pub mod step;
mod steps;
pub mod util;
pub mod version;
pub mod vfs;

pub use archive::Limits;
pub use convert::{convert, convert_pack, supported_versions, ConvertOptions, ConvertOutput, Report};
pub use error::{Error, Result};
pub use version::{PackFormat, Version};
pub use vfs::Pack;

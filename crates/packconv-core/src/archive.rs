use std::io::{Cursor, Read, Write};

use crate::error::{Error, Result};
use crate::vfs::{normalize_path, Pack};

/// Guards against archive bombs when unpacking untrusted uploads.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub max_entries: usize,
    pub max_total_size: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_entries: 100_000,
            max_total_size: 512 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveKind {
    Zip,
    Rar,
}

pub fn detect(bytes: &[u8]) -> Option<ArchiveKind> {
    if bytes.starts_with(b"PK\x03\x04") || bytes.starts_with(b"PK\x05\x06") {
        Some(ArchiveKind::Zip)
    } else if bytes.starts_with(b"Rar!\x1a\x07") {
        Some(ArchiveKind::Rar)
    } else {
        None
    }
}

/// Unpacks a ZIP or RAR archive into memory and strips a wrapping root directory.
pub fn read_archive(bytes: &[u8], limits: Limits) -> Result<Pack> {
    let mut pack = match detect(bytes) {
        Some(ArchiveKind::Zip) => read_zip(bytes, limits)?,
        Some(ArchiveKind::Rar) => crate::rar::read_rar(bytes, limits)?,
        None => return Err(Error::UnknownArchive),
    };
    pack.strip_wrapping_dir();
    if !pack.contains("pack.mcmeta") {
        return Err(Error::MissingPackMeta);
    }
    Ok(pack)
}

/// Shared bookkeeping for archive readers: enforces limits and normalizes entry names.
pub(crate) struct Collector {
    pack: Pack,
    limits: Limits,
    total: u64,
    /// Raw entries seen, including duplicates and rejected names.
    entries: usize,
}

impl Collector {
    pub(crate) fn new(limits: Limits) -> Self {
        Self { pack: Pack::new(), limits, total: 0, entries: 0 }
    }

    /// Reads one entry through `reader`, never more than the remaining size budget.
    pub(crate) fn add(&mut self, name: &str, reader: &mut dyn Read) -> Result<()> {
        let remaining = self.limits.max_total_size - self.total;
        let mut data = Vec::new();
        reader.take(remaining.saturating_add(1)).read_to_end(&mut data)?;
        self.add_bytes(name, data)
    }

    pub(crate) fn add_bytes(&mut self, name: &str, data: Vec<u8>) -> Result<()> {
        self.entries += 1;
        if self.entries > self.limits.max_entries {
            return Err(Error::LimitExceeded("too many entries"));
        }
        self.total += data.len() as u64;
        if self.total > self.limits.max_total_size {
            return Err(Error::LimitExceeded("uncompressed size"));
        }
        if let Some(path) = normalize_path(name) {
            self.pack.insert(path, data);
        }
        Ok(())
    }

    pub(crate) fn finish(self) -> Pack {
        self.pack
    }
}

fn read_zip(bytes: &[u8], limits: Limits) -> Result<Pack> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))?;
    if archive.len() > limits.max_entries {
        return Err(Error::LimitExceeded("too many entries"));
    }
    let mut collector = Collector::new(limits);
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        if entry.is_dir() {
            continue;
        }
        let name = entry.name()?.into_owned();
        collector.add(&name, &mut entry)?;
    }
    Ok(collector.finish())
}

pub fn write_zip(pack: &Pack) -> Result<Vec<u8>> {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .last_modified_time(zip::DateTime::default());
    for (path, data) in pack.iter() {
        writer.start_file(path, options)?;
        writer.write_all(data)?;
    }
    Ok(writer.finish()?.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zip_roundtrip() {
        let mut pack = Pack::new();
        pack.insert("pack.mcmeta", br#"{"pack":{"pack_format":1,"description":""}}"#.to_vec());
        pack.insert("assets/minecraft/textures/a.png", vec![1, 2, 3]);
        let bytes = write_zip(&pack).unwrap();
        assert_eq!(detect(&bytes), Some(ArchiveKind::Zip));
        let back = read_archive(&bytes, Limits::default()).unwrap();
        assert_eq!(back.get("assets/minecraft/textures/a.png"), Some(&[1u8, 2, 3][..]));
    }

    #[test]
    fn enforces_size_limit() {
        let mut pack = Pack::new();
        pack.insert("pack.mcmeta", vec![0; 100]);
        let bytes = write_zip(&pack).unwrap();
        let limits = Limits { max_entries: 10, max_total_size: 50 };
        assert!(matches!(read_archive(&bytes, limits), Err(Error::LimitExceeded(_))));
    }

    #[test]
    fn rejects_unknown_archive() {
        assert!(matches!(read_archive(b"hello", Limits::default()), Err(Error::UnknownArchive)));
    }
}

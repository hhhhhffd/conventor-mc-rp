use std::cell::RefCell;
use std::io::Write;
use std::rc::Rc;

use crate::archive::{Collector, Limits};
use crate::error::{Error, Result};
use crate::vfs::Pack;

/// Writer handed to `rars` that keeps the extracted bytes accessible after extraction.
struct SharedBuf(Rc<RefCell<Vec<u8>>>);

impl Write for SharedBuf {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.borrow_mut().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Extracts a RAR (1.3/4/5) archive in a single sequential pass, which keeps
/// solid archives linear instead of re-decoding predecessors per member.
pub(crate) fn read_rar(bytes: &[u8], limits: Limits) -> Result<Pack> {
    let options = rars::ArchiveReadOptions::new().with_max_total_output_bytes(limits.max_total_size);
    let archive = rars::ArchiveReader::read_with_options(bytes, options.clone())
        .map_err(|e| Error::Rar(e.to_string()))?;

    let mut entries: Vec<(String, Rc<RefCell<Vec<u8>>>)> = Vec::new();
    archive
        .extract_to_with_options(options, |meta| {
            let buf = Rc::new(RefCell::new(Vec::new()));
            if !meta.is_directory {
                entries.push((meta.name_lossy(), buf.clone()));
            }
            Ok(Box::new(SharedBuf(buf)))
        })
        .map_err(|e| Error::Rar(e.to_string()))?;

    let mut collector = Collector::new(limits);
    for (name, buf) in entries {
        collector.add_bytes(&name, buf.take())?;
    }
    Ok(collector.finish())
}

use crate::error::Result;
use crate::version::Version;
use crate::vfs::Pack;

/// Mutable state shared by the steps of one conversion.
#[derive(Debug)]
pub struct Context {
    pub from: Version,
    pub to: Version,
    warnings: Vec<String>,
}

impl Context {
    pub fn new(from: Version, to: Version) -> Self {
        Self { from, to, warnings: Vec::new() }
    }

    /// Records something the converter could not carry over faithfully.
    pub fn warn(&mut self, message: impl Into<String>) {
        self.warnings.push(message.into());
    }

    pub fn into_warnings(self) -> Vec<String> {
        self.warnings
    }
}

/// One resource pack layout change introduced in a specific game version.
///
/// `upgrade` turns a pack laid out for the version just before `version()` into the
/// layout of `version()`; `downgrade` is its inverse. Steps must leave files they do not
/// understand untouched and report lossy conversions through [`Context::warn`].
pub trait Step: Send + Sync {
    fn name(&self) -> &'static str;
    /// First version that uses the new layout.
    fn version(&self) -> Version;
    fn upgrade(&self, pack: &mut Pack, ctx: &mut Context) -> Result<()>;
    fn downgrade(&self, pack: &mut Pack, ctx: &mut Context) -> Result<()>;
}

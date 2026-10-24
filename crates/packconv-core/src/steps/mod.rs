//! Conversion steps grouped by era. Each era module owns its own directory and exposes
//! `steps()`; ordering across eras is by [`Step::version`].

use crate::step::Step;

mod flattening;
mod legacy;
mod mid;
mod modern;
mod support;
mod yearly;

pub fn all() -> Vec<Box<dyn Step>> {
    let mut steps: Vec<Box<dyn Step>> = Vec::new();
    steps.extend(legacy::steps());
    steps.extend(flattening::steps());
    steps.extend(mid::steps());
    steps.extend(modern::steps());
    steps.extend(yearly::steps());
    // Stable sort: steps sharing a version keep their declaration order.
    steps.sort_by_key(|s| s.version());
    steps
}

//! The 1.13 flattening: renamed resources, `.lang` to JSON and texture sheets.

mod images;
mod lang;
mod maps;
mod v1_13;

use crate::step::Step;

pub fn steps() -> Vec<Box<dyn Step>> {
    vec![Box::new(v1_13::Flattening)]
}

//! Resource pack layout changes from 1.8 to 1.12.2.

use crate::step::Step;

mod common;
mod v1_10;
mod v1_11;
mod v1_12;
mod v1_8;
mod v1_9;

pub fn steps() -> Vec<Box<dyn Step>> {
    vec![
        Box::new(v1_8::V1_8),
        Box::new(v1_9::V1_9),
        Box::new(v1_10::V1_10),
        Box::new(v1_11::V1_11),
        Box::new(v1_12::V1_12),
    ]
}

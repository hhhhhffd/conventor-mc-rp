//! Steps for 1.20 ..= 1.21.8.
//!
//! 1.21, 1.21.6, 1.21.7 and 1.21.8 only added content or changed shaders and GUI behaviour,
//! so they have no step here.

use crate::step::Step;

mod common;
mod item_defs;
mod v1_20;
mod v1_20_2;
mod v1_20_3;
mod v1_20_5;
mod v1_21_2;
mod v1_21_4;
mod v1_21_5;

pub fn steps() -> Vec<Box<dyn Step>> {
    vec![
        Box::new(v1_20::V1_20),
        Box::new(v1_20_2::V1_20_2),
        Box::new(v1_20_3::V1_20_3),
        Box::new(v1_20_5::V1_20_5),
        Box::new(v1_21_2::V1_21_2),
        Box::new(v1_21_4::V1_21_4),
        Box::new(v1_21_5::V1_21_5),
    ]
}

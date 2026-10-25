//! Steps for the year-numbered releases (1.21.9 and later).

mod common;
mod v1_21_11;
mod v1_21_9;
mod v26_1;
mod v26_2;
mod v26_3;

use crate::step::Step;

pub fn steps() -> Vec<Box<dyn Step>> {
    vec![
        Box::new(v1_21_9::IronChain),
        Box::new(v1_21_11::Celestials),
        Box::new(v26_1::EntityFolders),
        Box::new(v26_2::SlicedBlockEntities),
        Box::new(v26_3::PalettesAndShade),
    ]
}

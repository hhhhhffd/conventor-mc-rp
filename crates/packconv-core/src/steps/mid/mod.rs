//! Steps for 1.14 through 1.19.4: sprite sheet splits, chest and particle changes, renames
//! and the 1.19.3 texture atlases.

use crate::step::Step;

mod rename;
mod sheet;
mod v1_14;
mod v1_15;
mod v1_16;
mod v1_17;
mod v1_18;
mod v1_19;
mod v1_19_3;
mod v1_19_4;

pub fn steps() -> Vec<Box<dyn Step>> {
    vec![
        Box::new(v1_14::V1_14),
        Box::new(v1_15::V1_15),
        Box::new(v1_16::V1_16),
        Box::new(v1_17::V1_17),
        Box::new(v1_18::V1_18),
        Box::new(v1_19::V1_19),
        Box::new(v1_19_3::V1_19_3),
        Box::new(v1_19_4::V1_19_4),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_are_in_ascending_version_order() {
        let versions: Vec<_> = steps().iter().map(|s| s.version()).collect();
        assert!(versions.windows(2).all(|pair| pair[0] < pair[1]));
        assert_eq!(versions.len(), 8);
    }
}

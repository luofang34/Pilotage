use super::*;

const TRAINER: [Point; 4] = [[700.0, 0.89], [1100.0, 0.99], [1100.0, 1.18], [700.0, 1.18]];

#[test]
fn every_boundary_point_is_inside() {
    for point in [
        [1100.0, 1.05],
        [700.0, 1.05],
        [900.0, 1.18],
        [900.0, 0.94],
        [700.0, 0.89],
        [1100.0, 1.18],
    ] {
        assert!(contains(&TRAINER, point), "{point:?} is on the limit");
    }
    assert!(contains(&TRAINER, [900.0, 1.0]));
    assert!(!contains(&TRAINER, [900.0, 1.19]));
    assert!(!contains(&TRAINER, [1100.1, 1.05]));
    assert!(!contains(&TRAINER, [900.0, 0.93]));
}

#[test]
fn a_crossing_or_flat_polygon_is_not_simple() {
    assert!(is_simple(&TRAINER));
    let bowtie = [[700.0, 0.89], [1100.0, 1.18], [1100.0, 0.99], [700.0, 1.18]];
    assert!(!is_simple(&bowtie));
    let flat = [[700.0, 1.0], [900.0, 1.0], [1100.0, 1.0]];
    assert!(!is_simple(&flat));
    let rounded_flat = [[700.0, 0.89], [900.0, 0.99], [1100.0, 1.09]];
    assert!(!is_simple(&rounded_flat));
    assert!(!is_simple(&TRAINER[..2]));
}

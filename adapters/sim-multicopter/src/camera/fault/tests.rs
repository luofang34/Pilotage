use super::{CameraFault, FaultKind, FaultWindow, apply, parse_faults};

#[test]
fn a_time_window_and_a_box_parse() {
    let faults = parse_faults("dark@30-45s; blur@40.335,-74.665,40.330,-74.670").expect("parses");
    assert_eq!(
        faults,
        vec![
            CameraFault {
                kind: FaultKind::Dark,
                window: FaultWindow::Time {
                    start_s: 30.0,
                    end_s: 45.0
                },
            },
            CameraFault {
                kind: FaultKind::Blur,
                window: FaultWindow::Area {
                    south: 40.330,
                    west: -74.670,
                    north: 40.335,
                    east: -74.665,
                },
            },
        ]
    );
    assert!(parse_faults("").expect("empty parses").is_empty());
    assert_eq!(
        parse_faults("overexposed@0-1.5s").expect("parses")[0].kind,
        FaultKind::Overexposed
    );
}

#[test]
fn malformed_faults_are_refused() {
    for text in [
        "dark",
        "fog@1-2s",
        "dark@2-1s",
        "dark@1-xs",
        "dark@1-2",
        "blur@40.3,-74.6,40.4",
        "blur@91,-74.6,40.4,-74.5",
        "blur@40.3,-74.6,40.3,-74.5",
    ] {
        assert!(parse_faults(text).is_err(), "{text} parsed");
    }
}

#[test]
fn windows_activate_in_time_and_in_the_box() {
    let faults = parse_faults("dark@10-20s;blur@40.330,-74.670,40.335,-74.665").expect("parses");
    assert!(!faults[0].active(9.99, 0.0, 0.0));
    assert!(faults[0].active(10.0, 0.0, 0.0));
    assert!(!faults[0].active(20.0, 0.0, 0.0));
    assert!(faults[1].active(0.0, 40.332, -74.668));
    assert!(!faults[1].active(0.0, 40.336, -74.668));
}

fn checkerboard(width: usize, height: usize) -> Vec<u8> {
    let mut rgb = Vec::with_capacity(width * height * 3);
    for y in 0..height {
        for x in 0..width {
            let value = if (x / 4 + y / 4) % 2 == 0 { 220 } else { 30 };
            rgb.extend_from_slice(&[value; 3]);
        }
    }
    rgb
}

fn contrast(rgb: &[u8]) -> i32 {
    let max = rgb.iter().copied().max().unwrap_or(0);
    let min = rgb.iter().copied().min().unwrap_or(0);
    i32::from(max) - i32::from(min)
}

#[test]
fn effects_change_the_image_as_named() {
    let (width, height) = (64, 32);
    let original = checkerboard(width, height);
    let mean = |rgb: &[u8]| rgb.iter().map(|&v| u32::from(v)).sum::<u32>() / rgb.len() as u32;

    let mut dark = original.clone();
    apply(FaultKind::Dark, &mut dark, width, height);
    assert!(mean(&dark) < mean(&original) / 5);

    let mut bright = original.clone();
    apply(FaultKind::Overexposed, &mut bright, width, height);
    assert!(mean(&bright) > 200);

    let mut blurred = original.clone();
    apply(FaultKind::Blur, &mut blurred, width, height);
    // The interior loses the checkerboard edges.
    let row = &blurred[(16 * width + 16) * 3..(16 * width + 48) * 3];
    assert!(contrast(row) < 40, "{}", contrast(row));
    assert!(contrast(&original) > 150);
}

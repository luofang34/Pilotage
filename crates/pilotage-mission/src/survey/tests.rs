#![allow(clippy::expect_used, clippy::panic)]

use core::f64::consts::{FRAC_PI_2, PI};

use navigate_contract::{AltitudeConstraint, GeodeticPosition, TurnType};
use navigate_geodesy::{LocalTangentPlane, NedOffset};

use super::{MAX_SURVEY_WAYPOINTS, SurveyError, SurveyRequest, plan_survey};

const HEIGHT_M: f64 = 110.0;

fn anchor() -> GeodeticPosition {
    GeodeticPosition::new(47.0_f64.to_radians(), 8.0_f64.to_radians(), 500.0)
}

fn plane() -> LocalTangentPlane {
    LocalTangentPlane::new(anchor()).expect("anchor is plausible")
}

/// A polygon from planar corners, in degrees.
fn polygon(corners: &[[f64; 2]]) -> Vec<[f64; 2]> {
    let plane = plane();
    corners
        .iter()
        .map(|[north, east]| {
            let position = plane.from_ned(&NedOffset::new(*north, *east, 0.0));
            [
                position.latitude_rad.to_degrees(),
                position.longitude_rad.to_degrees(),
            ]
        })
        .collect()
}

fn rectangle(north_m: f64, east_m: f64) -> Vec<[f64; 2]> {
    polygon(&[[0.0, 0.0], [north_m, 0.0], [north_m, east_m], [0.0, east_m]])
}

/// Waypoint positions back in the plane: north, east.
fn planar_waypoints(plan: &super::SurveyPlan) -> Vec<[f64; 2]> {
    let plane = plane();
    plan.plan
        .waypoints
        .iter()
        .map(|waypoint| {
            let offset = plane.to_ned(&waypoint.position);
            [offset.north_m, offset.east_m]
        })
        .collect()
}

fn footprint(fov_rad: f64) -> f64 {
    2.0 * HEIGHT_M * (fov_rad / 2.0).tan()
}

/// Distance from a point to a segment.
fn distance_to_segment(point: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let length_sq = dx * dx + dy * dy;
    let t = if length_sq == 0.0 {
        0.0
    } else {
        (((point[0] - a[0]) * dx + (point[1] - a[1]) * dy) / length_sq).clamp(0.0, 1.0)
    };
    (point[0] - (a[0] + t * dx)).hypot(point[1] - (a[1] + t * dy))
}

/// Even-odd point-in-polygon in the plane.
fn inside(point: [f64; 2], vertices: &[[f64; 2]]) -> bool {
    let mut inside = false;
    for (index, a) in vertices.iter().enumerate() {
        let b = vertices[(index + 1) % vertices.len()];
        if (a[1] > point[1]) != (b[1] > point[1]) {
            let x = a[0] + (point[1] - a[1]) / (b[1] - a[1]) * (b[0] - a[0]);
            if point[0] < x {
                inside = !inside;
            }
        }
    }
    inside
}

/// Every interior grid point of the planar polygon lies inside half a
/// footprint of some flown line.
fn assert_covered(plan: &super::SurveyPlan, vertices: &[[f64; 2]], half_footprint_m: f64) {
    assert_covered_at(plan, vertices, half_footprint_m, 5.0);
}

/// [`assert_covered`] on a grid of the given step.
fn assert_covered_at(
    plan: &super::SurveyPlan,
    vertices: &[[f64; 2]],
    half_footprint_m: f64,
    step_m: f64,
) {
    let points = planar_waypoints(plan);
    let lines: Vec<([f64; 2], [f64; 2])> =
        points.chunks(2).map(|pair| (pair[0], pair[1])).collect();
    let (mut min_n, mut max_n, mut min_e, mut max_e) = (
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
    );
    for [n, e] in vertices {
        min_n = min_n.min(*n);
        max_n = max_n.max(*n);
        min_e = min_e.min(*e);
        max_e = max_e.max(*e);
    }
    let mut checked = 0;
    let mut n = min_n;
    while n <= max_n {
        let mut e = min_e;
        while e <= max_e {
            if inside([n, e], vertices) {
                let nearest = lines
                    .iter()
                    .map(|(a, b)| distance_to_segment([n, e], *a, *b))
                    .fold(f64::INFINITY, f64::min);
                assert!(
                    nearest <= half_footprint_m + 1e-6,
                    "point ({n}, {e}) is {nearest} m from the nearest line"
                );
                checked += 1;
            }
            e += step_m;
        }
        n += step_m;
    }
    assert!(checked > 100, "the grid sampled the polygon interior");
}

#[test]
fn rectangle_lines_follow_the_long_edge_and_cover_the_width() {
    let request = SurveyRequest::new(rectangle(400.0, 300.0));
    let survey = plan_survey(&request, &anchor(), HEIGHT_M).expect("plans");
    let footprint = footprint(request.across_track_fov_rad);
    assert!((survey.footprint_width_m - footprint).abs() < 1e-9);
    assert!((survey.line_spacing_m - footprint * 0.4).abs() < 1e-9);
    assert_eq!(survey.line_count, 4);
    assert_eq!(survey.plan.waypoints.len(), 8);
    let idents: Vec<&str> = survey
        .plan
        .waypoints
        .iter()
        .map(|waypoint| waypoint.ident.as_str())
        .collect();
    assert_eq!(
        idents,
        [
            "L01A", "L01B", "L02A", "L02B", "L03A", "L03B", "L04A", "L04B"
        ]
    );
    for waypoint in &survey.plan.waypoints {
        assert_eq!(waypoint.turn, TurnType::FlyOver);
        assert_eq!(waypoint.altitude, Some(AltitudeConstraint::At(610.0)));
        assert!((waypoint.position.altitude_m - 610.0).abs() < 1e-6);
    }
    let points = planar_waypoints(&survey);
    // Lines run north along the 400 m edge, 10 m past both ends.
    for pair in points.chunks(2) {
        assert!(
            (pair[0][1] - pair[1][1]).abs() < 1e-3,
            "a line keeps its east offset"
        );
        assert!(((pair[0][0] - pair[1][0]).abs() - 420.0).abs() < 1e-3);
    }
    assert!(
        (points[0][0] + 10.0).abs() < 1e-3,
        "the first line starts south of the polygon"
    );
    // Boustrophedon: a line ends where the next one starts, across the gap.
    for lines in points.chunks(2).collect::<Vec<_>>().windows(2) {
        assert!((lines[0][1][0] - lines[1][0][0]).abs() < 1e-3);
    }
    // The first and last swath edges sit on the polygon's sides.
    assert!((points[0][1] - footprint / 2.0).abs() < 1e-3);
    assert!((points[7][1] - (300.0 - footprint / 2.0)).abs() < 1e-3);
    let planar = [[0.0, 0.0], [400.0, 0.0], [400.0, 300.0], [0.0, 300.0]];
    assert_covered(&survey, &planar, footprint / 2.0);
}

#[test]
fn a_polygon_narrower_than_the_footprint_gets_one_centred_line() {
    let request = SurveyRequest::new(rectangle(400.0, 100.0));
    let survey = plan_survey(&request, &anchor(), HEIGHT_M).expect("plans");
    assert_eq!(survey.line_count, 1);
    let points = planar_waypoints(&survey);
    assert!((points[0][1] - 50.0).abs() < 1e-3 && (points[1][1] - 50.0).abs() < 1e-3);
    assert!((survey.path_length_m - 420.0).abs() < 1e-3);
}

#[test]
fn a_bearing_overrides_the_longest_edge() {
    let mut request = SurveyRequest::new(rectangle(400.0, 300.0));
    request.line_bearing_rad = Some(FRAC_PI_2);
    let survey = plan_survey(&request, &anchor(), HEIGHT_M).expect("plans");
    let points = planar_waypoints(&survey);
    for pair in points.chunks(2) {
        assert!(
            (pair[0][0] - pair[1][0]).abs() < 1e-3,
            "a line keeps its north offset"
        );
        assert!(((pair[0][1] - pair[1][1]).abs() - 320.0).abs() < 1e-3);
    }
    assert!(survey.line_count > 4, "the 400 m width needs more lines");
    let planar = [[0.0, 0.0], [400.0, 0.0], [400.0, 300.0], [0.0, 300.0]];
    assert_covered(
        &survey,
        &planar,
        footprint(request.across_track_fov_rad) / 2.0,
    );
}

#[test]
fn an_l_shaped_polygon_is_covered_by_spanning_lines() {
    let planar = [
        [0.0, 0.0],
        [500.0, 0.0],
        [500.0, 150.0],
        [250.0, 150.0],
        [250.0, 350.0],
        [0.0, 350.0],
    ];
    let request = SurveyRequest::new(polygon(&planar));
    let survey = plan_survey(&request, &anchor(), HEIGHT_M).expect("plans");
    assert!(survey.line_count >= 4);
    assert_covered(
        &survey,
        &planar,
        footprint(request.across_track_fov_rad) / 2.0,
    );
}

#[test]
fn the_path_length_counts_lines_and_the_gaps_between_them() {
    let request = SurveyRequest::new(rectangle(400.0, 300.0));
    let survey = plan_survey(&request, &anchor(), HEIGHT_M).expect("plans");
    let points = planar_waypoints(&survey);
    let expected: f64 = points
        .windows(2)
        .map(|pair| (pair[0][0] - pair[1][0]).hypot(pair[0][1] - pair[1][1]))
        .sum();
    assert!((survey.path_length_m - expected).abs() < 1e-3);
}

#[test]
fn requests_outside_their_ranges_are_refused() {
    let two = SurveyRequest::new(rectangle(400.0, 300.0).into_iter().take(2).collect());
    assert!(matches!(
        plan_survey(&two, &anchor(), HEIGHT_M),
        Err(SurveyError::TooFewVertices { count: 2 })
    ));
    let collinear = SurveyRequest::new(polygon(&[[0.0, 0.0], [100.0, 0.0], [200.0, 0.0]]));
    assert!(matches!(
        plan_survey(&collinear, &anchor(), HEIGHT_M),
        Err(SurveyError::DegenerateArea { .. })
    ));
    let mut nan = SurveyRequest::new(rectangle(400.0, 300.0));
    nan.polygon_deg[1][0] = f64::NAN;
    assert!(matches!(
        plan_survey(&nan, &anchor(), HEIGHT_M),
        Err(SurveyError::ImplausibleVertex { index: 1, .. })
    ));
    let mut polar = SurveyRequest::new(rectangle(400.0, 300.0));
    polar.polygon_deg[2][0] = 100.0;
    assert!(matches!(
        plan_survey(&polar, &anchor(), HEIGHT_M),
        Err(SurveyError::ImplausibleVertex { index: 2, .. })
    ));
    for (fov, sidelap, overshoot, capture, height) in [
        (0.0, 0.6, 10.0, 5.0, HEIGHT_M),
        (PI, 0.6, 10.0, 5.0, HEIGHT_M),
        (1.2, 0.96, 10.0, 5.0, HEIGHT_M),
        (1.2, -0.1, 10.0, 5.0, HEIGHT_M),
        (1.2, 0.6, -1.0, 5.0, HEIGHT_M),
        (1.2, 0.6, 10.0, 0.0, HEIGHT_M),
        (1.2, 0.6, 10.0, 5.0, 0.0),
        (1.2, 0.6, 10.0, 5.0, f64::NAN),
    ] {
        let mut request = SurveyRequest::new(rectangle(400.0, 300.0));
        request.across_track_fov_rad = fov;
        request.sidelap = sidelap;
        request.overshoot_m = overshoot;
        request.capture_radius_m = capture;
        assert!(
            matches!(
                plan_survey(&request, &anchor(), height),
                Err(SurveyError::ParameterOutOfRange { .. })
            ),
            "fov {fov} sidelap {sidelap} overshoot {overshoot} capture {capture} height {height}"
        );
    }
    let mut tight = SurveyRequest::new(rectangle(400.0, 300.0));
    tight.across_track_fov_rad = 0.1;
    tight.sidelap = 0.95;
    assert!(matches!(
        plan_survey(&tight, &anchor(), 1.0),
        Err(SurveyError::SpacingTooSmall { .. })
    ));
    let mut vast = SurveyRequest::new(rectangle(30_000.0, 30_000.0));
    vast.across_track_fov_rad = 0.2;
    assert!(matches!(
        plan_survey(&vast, &anchor(), HEIGHT_M),
        Err(SurveyError::TooManyWaypoints)
    ));
    let mut flat = SurveyRequest::new(rectangle(400.0, 300.0));
    flat.footprint_width_m = Some(0.0);
    assert!(matches!(
        plan_survey(&flat, &anchor(), HEIGHT_M),
        Err(SurveyError::ParameterOutOfRange { .. })
    ));
    let largest = SurveyRequest::new(rectangle(400.0, 300.0));
    assert!(
        plan_survey(&largest, &anchor(), HEIGHT_M)
            .expect("plans")
            .plan
            .waypoints
            .len()
            <= MAX_SURVEY_WAYPOINTS
    );
}

#[test]
fn a_measured_footprint_replaces_the_derived_one_and_allows_a_planar_height() {
    let mut request = SurveyRequest::new(rectangle(40.0, 30.0));
    request.footprint_width_m = Some(12.0);
    let survey = plan_survey(&request, &anchor(), 0.0).expect("plans at zero height");
    assert!((survey.footprint_width_m - 12.0).abs() < 1e-9);
    assert!((survey.line_spacing_m - 4.8).abs() < 1e-9);
    assert_eq!(survey.line_count, 5);
    for waypoint in &survey.plan.waypoints {
        assert_eq!(waypoint.altitude, Some(AltitudeConstraint::At(500.0)));
    }
    let planar = [[0.0, 0.0], [40.0, 0.0], [40.0, 30.0], [0.0, 30.0]];
    assert_covered_at(&survey, &planar, 6.0, 1.0);
    // The gaps between lines are shorter than the 5 m request: 4.8 m
    // steps, then 3.6 m to the last line pulled back onto the far side.
    // The plan is flown with half the shortest leg.
    assert!((survey.capture_radius_m - 1.8).abs() < 1e-9);
    let wide = plan_survey(
        &SurveyRequest::new(rectangle(400.0, 300.0)),
        &anchor(),
        HEIGHT_M,
    )
    .expect("plans");
    assert!(
        (wide.capture_radius_m - 5.0).abs() < 1e-9,
        "long legs keep the request"
    );
}

#[test]
fn the_digest_binds_every_parameter() {
    let request = SurveyRequest::new(rectangle(400.0, 300.0));
    assert_eq!(
        request.digest(),
        SurveyRequest::new(rectangle(400.0, 300.0)).digest()
    );
    let mut sidelap = request.clone();
    sidelap.sidelap = 0.7;
    assert_ne!(sidelap.digest(), request.digest());
    let mut bearing = request.clone();
    bearing.line_bearing_rad = Some(0.0);
    assert_ne!(bearing.digest(), request.digest());
    let mut moved = request.clone();
    moved.polygon_deg[0][1] += 1e-6;
    assert_ne!(moved.digest(), request.digest());
    let mut measured = request.clone();
    measured.footprint_width_m = Some(150.0);
    assert_ne!(measured.digest(), request.digest());
    let survey = plan_survey(&request, &anchor(), HEIGHT_M).expect("plans");
    assert_eq!(survey.digest, request.digest());
    assert_eq!(survey.plan.id, format!("survey:{}", request.digest()));
}

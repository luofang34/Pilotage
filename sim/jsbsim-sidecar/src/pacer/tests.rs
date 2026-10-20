//! The pacer: it holds simulation time to wall time, and gives up a lag
//! rather than racing to make it good.

use std::time::{Duration, Instant};

use super::{MAX_LAG, Pacer};

#[test]
fn a_step_ahead_of_wall_time_sleeps_until_it_is_due() {
    let start = Instant::now();
    let mut pacer = Pacer::new(1.0, start, 0.0);
    assert_eq!(pacer.wait(0.004, start), Some(Duration::from_millis(4)));
    assert_eq!(
        pacer.wait(0.004, start + Duration::from_millis(1)),
        Some(Duration::from_millis(3))
    );
    // At twice real time, 4 ms of simulation take 2 ms.
    let mut fast = Pacer::new(2.0, start, 0.0);
    assert_eq!(fast.wait(0.004, start), Some(Duration::from_millis(2)));
}

#[test]
fn a_small_lag_is_made_good_and_a_large_one_given_up() {
    let start = Instant::now();
    let mut pacer = Pacer::new(1.0, start, 0.0);
    // 50 ms behind: no sleep, and the anchor stays.
    assert_eq!(pacer.wait(1.0, start + Duration::from_millis(1_050)), None);
    assert_eq!(
        pacer.wait(1.1, start + Duration::from_millis(1_060)),
        Some(Duration::from_millis(40))
    );
    // Two seconds behind (a stalled flight controller): the lag is given
    // up, and the next step is paced from where things stand.
    let late = start + Duration::from_secs(4);
    assert_eq!(pacer.wait(2.0, late), None);
    assert_eq!(pacer.wait(2.004, late), Some(Duration::from_millis(4)));
    assert!(MAX_LAG < Duration::from_secs(1));
}

#[test]
fn an_unpaced_run_never_sleeps() {
    let start = Instant::now();
    let mut pacer = Pacer::new(0.0, start, 0.0);
    assert_eq!(pacer.wait(100.0, start), None);
}

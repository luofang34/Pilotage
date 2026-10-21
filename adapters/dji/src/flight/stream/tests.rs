use super::*;

fn time(ms: u64) -> Duration {
    Duration::from_millis(ms)
}

fn binding() -> VirtualStickBinding {
    VirtualStickBinding {
        attachment: [1; 16],
        session: 2,
        generation: 3,
        model_revision: 4,
    }
}

#[test]
fn sustained_four_axis_input_is_paced_without_catch_up_bursts() {
    let mut stream = VirtualStickStream::new(binding(), time(0));
    let demand = VirtualStickVelocity::new(0.6, -0.8, 0.5, -0.4).unwrap();
    let mut writes = Vec::new();
    for tick in 0..1000_u32 {
        let now = time(u64::from(tick) * 10);
        stream
            .submit(binding(), tick, demand, now, now + time(100))
            .unwrap();
        if let Some(write) = stream.next_write(now, tick as u16) {
            assert_eq!(write.packet(), demand.packet(tick as u16));
            assert!(stream.next_write(now, 99).is_none());
            stream.written(write, now);
            writes.push(now);
        }
    }
    assert_eq!(writes.len(), 200);
    assert!(writes.windows(2).all(|pair| pair[1] - pair[0] == time(50)));
    // A fresh sample after a scheduling delay yields one command, not a burst.
    stream
        .submit(binding(), 1000, demand, time(10040), time(10140))
        .unwrap();
    let write = stream.next_write(time(10040), 1000).unwrap();
    stream.written(write, time(10040));
    assert!(stream.next_write(time(10041), 1001).is_none());
}

#[test]
fn newest_input_wins_without_renewing_a_reserved_packet_deadline() {
    let mut stream = VirtualStickStream::new(binding(), time(0));
    let old = VirtualStickVelocity::new(1.0, 0.0, 0.0, 0.0).unwrap();
    let new = VirtualStickVelocity::new(0.0, -1.0, -0.5, 0.5).unwrap();
    stream
        .submit(binding(), 1, old, time(0), time(100))
        .unwrap();
    let reserved = stream.next_write(time(0), 1).unwrap();
    stream
        .submit(binding(), 2, new, time(10), time(200))
        .unwrap();
    assert_eq!(reserved.send_until(), time(50));
    assert_eq!(reserved.packet(), old.packet(1));
    stream.written(reserved, time(10));
    let write = stream.next_write(time(60), 2).unwrap();
    assert_eq!(write.packet(), new.packet(2));
    stream.written(write, time(60));
}

#[test]
fn input_timeout_writes_zero_once_then_requires_mode_release() {
    let mut stream = VirtualStickStream::new(binding(), time(0));
    let demand = VirtualStickVelocity::new(1.0, 0.0, -0.5, 0.5).unwrap();
    stream
        .submit(binding(), 1, demand, time(0), time(100))
        .unwrap();
    let moving = stream.next_write(time(0), 10).unwrap();
    stream.written(moving, time(0));
    let stop = stream.next_write(time(100), 11).unwrap();
    assert_eq!(stream.phase(), VirtualStickStreamPhase::Neutralizing);
    assert_eq!(stop.packet(), VirtualStickVelocity::ZERO.packet(11));
    stream.stop(time(200));
    assert_eq!(stop.send_until(), time(350));
    stream.written(stop, time(201));
    assert_eq!(stream.phase(), VirtualStickStreamPhase::ReleaseRequired);
    assert!(stream.next_write(time(202), 12).is_none());
    assert_eq!(
        stream.submit(binding(), 2, demand, time(202), time(302)),
        Err(VirtualStickStreamError::Closed)
    );
}

#[test]
fn replay_and_different_lease_cannot_keep_an_old_demand_alive() {
    let mut stream = VirtualStickStream::new(binding(), time(0));
    stream
        .submit(
            binding(),
            u32::MAX,
            VirtualStickVelocity::ZERO,
            time(0),
            time(100),
        )
        .unwrap();
    stream
        .submit(binding(), 0, VirtualStickVelocity::ZERO, time(1), time(101))
        .unwrap();
    assert_eq!(
        stream.submit(
            binding(),
            0,
            VirtualStickVelocity::ZERO,
            time(50),
            time(200)
        ),
        Err(VirtualStickStreamError::Sequence)
    );
    let foreign = VirtualStickBinding {
        generation: 4,
        ..binding()
    };
    assert_eq!(
        stream.submit(foreign, 1, VirtualStickVelocity::ZERO, time(90), time(200)),
        Err(VirtualStickStreamError::Binding)
    );
    let stop = stream.next_write(time(101), 1).unwrap();
    assert!(stop.neutralizing);
}

#[test]
fn cancellation_never_splices_a_reserved_motion_packet() {
    let mut stream = VirtualStickStream::new(binding(), time(0));
    let write = stream.next_write(time(0), 1).unwrap();
    stream.stop(time(1));
    assert!(!stream.permits(write, time(1)));
    assert!(stream.next_write(time(1), 2).is_none());
    stream.discard_unwritten(write, time(2));
    let zero = stream.next_write(time(2), 2).unwrap();
    assert_eq!(zero.send_until(), time(251));
    stream.transport_lost();
    assert!(!stream.permits(zero, time(3)));
    assert!(stream.next_write(time(4), 3).is_none());
}

#[test]
fn late_complete_write_is_uncertain_not_a_successful_stop() {
    let mut stream = VirtualStickStream::new(binding(), time(0));
    stream.stop(time(0));
    let zero = stream.next_write(time(0), 1).unwrap();
    stream.written(zero, time(250));
    assert_eq!(stream.phase(), VirtualStickStreamPhase::Uncertain);
}

use std::error::Error;
use std::io::ErrorKind;
use std::os::fd::{AsFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;

use rustix::event::{PollFd, PollFlags, Timespec, poll};
use rustix::fs::{Mode, OFlags, fcntl_getfl, fcntl_setfl, open};
use rustix::io::{FdFlags, fcntl_getfd, fcntl_setfd, read, write};
use rustix::pty::{OpenptFlags, grantpt, openpt, ptsname, unlockpt};
use rustix::termios::{LocalModes, OptionalActions, Termios, tcgetattr, tcsetattr};

use super::RawSerial;

type TestResult = Result<(), Box<dyn Error>>;

struct TerminalPair {
    master: OwnedFd,
    observer: OwnedFd,
    path: PathBuf,
}

impl TerminalPair {
    fn open_blocking() -> Result<Self, Box<dyn Error>> {
        let master = openpt(OpenptFlags::RDWR | OpenptFlags::NOCTTY)?;
        fcntl_setfd(&master, FdFlags::CLOEXEC)?;
        grantpt(&master)?;
        unlockpt(&master)?;
        let name = ptsname(&master, Vec::new())?;
        let path = PathBuf::from(std::ffi::OsStr::from_bytes(name.to_bytes()));
        let observer = open(
            &path,
            OFlags::RDWR | OFlags::NOCTTY | OFlags::NONBLOCK | OFlags::CLOEXEC,
            Mode::empty(),
        )?;
        fcntl_setfl(&master, fcntl_getfl(&master)? | OFlags::NONBLOCK)?;
        Ok(Self {
            master,
            observer,
            path,
        })
    }
}

fn wait_readable_blocking(fd: &impl AsFd) -> TestResult {
    let mut descriptors = [PollFd::new(fd, PollFlags::IN)];
    let timeout = Timespec {
        tv_sec: 2,
        tv_nsec: 0,
    };
    assert_eq!(poll(&mut descriptors, Some(&timeout))?, 1);
    assert!(descriptors[0].revents().contains(PollFlags::IN));
    Ok(())
}

fn assert_restored_blocking(fd: &impl AsFd, expected: &Termios) -> TestResult {
    let mut actual = tcgetattr(fd)?;
    let mut expected = expected.clone();
    // The kernel sets PENDIN when ICANON changes, even with an empty queue.
    actual.local_modes.remove(LocalModes::PENDIN);
    expected.local_modes.remove(LocalModes::PENDIN);
    assert_eq!(format!("{actual:?}"), format!("{expected:?}"));
    Ok(())
}

#[test]
fn raw_binary_io_is_nonblocking_exclusive_and_restores_on_close() -> TestResult {
    let pair = TerminalPair::open_blocking()?;
    let original = tcgetattr(&pair.observer)?;
    let mut serial = RawSerial::open_blocking(&pair.path)?;
    let raw = tcgetattr(&pair.observer)?;
    assert_eq!(raw.input_speed(), original.input_speed());
    assert_eq!(raw.output_speed(), original.output_speed());
    assert!(
        !raw.local_modes
            .intersects(LocalModes::ICANON | LocalModes::ECHO)
    );
    assert!(fcntl_getfl(&serial.fd)?.contains(OFlags::NONBLOCK));
    assert!(fcntl_getfd(&serial.fd)?.contains(FdFlags::CLOEXEC));
    let mut buffer = [0_u8; 32];
    assert!(matches!(
        serial.try_read(&mut buffer),
        Err(error) if error.kind() == ErrorKind::WouldBlock
    ));
    let exclusive_error = RawSerial::open_blocking(&pair.path)
        .err()
        .ok_or("exclusive terminal accepted a second open")?;
    assert!(
        matches!(
            exclusive_error.kind(),
            ErrorKind::ResourceBusy | ErrorKind::WouldBlock
        ),
        "{exclusive_error}"
    );

    let payload = [0, 3, 17, 19, 255, 13, 10];
    assert_eq!(write(&pair.master, &payload)?, payload.len());
    wait_readable_blocking(&serial.fd)?;
    let received = serial.try_read(&mut buffer)?;
    assert_eq!(&buffer[..received], &payload);
    assert_eq!(
        read(&pair.master, &mut buffer),
        Err(rustix::io::Errno::AGAIN)
    );
    assert_eq!(serial.try_write(&payload)?, payload.len());
    wait_readable_blocking(&pair.master)?;
    let received = read(&pair.master, &mut buffer)?;
    assert_eq!(&buffer[..received], &payload);

    serial.close_blocking()?;
    assert_restored_blocking(&pair.observer, &original)?;
    RawSerial::open_blocking(&pair.path)?.close_blocking()?;
    Ok(())
}

#[test]
fn open_keeps_pending_input_and_drop_restores_settings() -> TestResult {
    let pair = TerminalPair::open_blocking()?;
    let mut original = tcgetattr(&pair.observer)?;
    original.make_raw();
    original.local_modes.insert(LocalModes::ISIG);
    original.set_speed(19200)?;
    tcsetattr(&pair.observer, OptionalActions::Now, &original)?;
    let payload = [0, 13, 10, 255];
    assert_eq!(write(&pair.master, &payload)?, payload.len());
    wait_readable_blocking(&pair.observer)?;
    {
        let mut serial = RawSerial::open_blocking(&pair.path)?;
        let mut bytes = [0_u8; 32];
        let received = serial.try_read(&mut bytes)?;
        assert_eq!(&bytes[..received], &payload);
        assert_eq!(tcgetattr(&pair.observer)?.input_speed(), 19200);
        assert!(
            !tcgetattr(&pair.observer)?
                .local_modes
                .contains(LocalModes::ISIG)
        );
    }
    assert_restored_blocking(&pair.observer, &original)?;
    RawSerial::open_blocking(&pair.path)?.close_blocking()?;
    Ok(())
}

#[test]
fn a_full_output_queue_returns_would_block_without_retaining_input() -> TestResult {
    let pair = TerminalPair::open_blocking()?;
    let mut serial = RawSerial::open_blocking(&pair.path)?;
    let payload = [0x55_u8; 65536];
    let mut written: usize = 0;
    let mut blocked = false;
    for _ in 0..64 {
        match serial.try_write(&payload) {
            Ok(count) => {
                assert!(count > 0);
                written = written.wrapping_add(count);
            }
            Err(error) if error.kind() == ErrorKind::WouldBlock => {
                blocked = true;
                break;
            }
            Err(error) => return Err(error.into()),
        }
    }
    assert!(blocked);
    assert!(written > 0);
    let mut remaining = written;
    let mut received = [0_u8; 65536];
    while remaining > 0 {
        wait_readable_blocking(&pair.master)?;
        let count = read(&pair.master, &mut received)?;
        assert!(count > 0 && count <= remaining);
        assert!(received[..count].iter().all(|byte| *byte == 0x55));
        remaining -= count;
    }
    assert_eq!(
        read(&pair.master, &mut received),
        Err(rustix::io::Errno::AGAIN)
    );
    serial.close_blocking()?;
    Ok(())
}

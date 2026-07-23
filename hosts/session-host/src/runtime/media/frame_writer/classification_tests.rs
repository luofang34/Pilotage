//! Error-classification tests for every pinned wtransport variant.

#![allow(clippy::expect_used, clippy::panic)]

use wtransport::VarInt;
use wtransport::error::{ConnectionError, StreamOpeningError, StreamWriteError};

use super::{FatalKind, StreamError, classify_open, classify_open_request, classify_write};

#[test]
fn classify_write_maps_every_pinned_variant() {
    assert_eq!(
        classify_write(&StreamWriteError::Stopped(VarInt::from_u32(9)), "write"),
        StreamError::PeerStop {
            phase: "write",
            code: Some(9),
        },
    );
    assert_eq!(
        classify_write(&StreamWriteError::Closed, "finish"),
        StreamError::LocalClose { phase: "finish" },
    );
    assert_eq!(
        classify_write(&StreamWriteError::NotConnected, "write"),
        StreamError::ConnectionFatal {
            phase: "write",
            kind: FatalKind::NotConnected,
        },
    );
    assert_eq!(
        classify_write(&StreamWriteError::QuicProto, "write"),
        StreamError::ConnectionFatal {
            phase: "write",
            kind: FatalKind::QuicProto,
        },
    );
}

#[test]
fn classify_open_maps_every_pinned_variant() {
    assert_eq!(
        classify_open(&StreamOpeningError::Refused),
        StreamError::PeerStop {
            phase: "open",
            code: None,
        },
    );
    assert_eq!(
        classify_open(&StreamOpeningError::NotConnected),
        StreamError::ConnectionFatal {
            phase: "open",
            kind: FatalKind::NotConnected,
        },
    );
}

#[test]
fn an_open_request_error_preserves_its_concrete_cause() {
    let classified = classify_open_request(ConnectionError::TimedOut);
    assert_eq!(
        classified,
        StreamError::ConnectionFatal {
            phase: "open",
            kind: FatalKind::OpenRequest(ConnectionError::TimedOut),
        },
    );
    let via_source = std::error::Error::source(&classified)
        .and_then(std::error::Error::source)
        .and_then(|error| error.downcast_ref::<ConnectionError>());
    assert_eq!(via_source, Some(&ConnectionError::TimedOut));
    assert!(classified.to_string().contains("timed out"), "{classified}");
}

use super::{Failure, UsbError};

#[test]
fn native_failure_keeps_its_typed_source_after_io_conversion() {
    let error = Failure::native(14, 0xe000_02ed).io();
    assert!(matches!(
        error
            .get_ref()
            .and_then(|source| source.downcast_ref::<UsbError>()),
        Some(UsbError::Native {
            stage: 14,
            status: 0xe000_02ed
        })
    ));
}

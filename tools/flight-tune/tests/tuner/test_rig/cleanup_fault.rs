use flight_tune::AdapterError;

#[allow(dead_code)]
#[derive(Debug, Default)]
pub enum FakeCleanupFault {
    #[default]
    None,
    PanicOn(usize),
    ReturnError,
}

#[allow(dead_code)]
impl FakeCleanupFault {
    pub fn panic_on(&mut self, occurrence: usize) {
        *self = Self::PanicOn(occurrence);
    }

    pub fn return_error(&mut self) {
        *self = Self::ReturnError;
    }

    pub fn clear(&mut self) {
        *self = Self::None;
    }

    pub(super) fn finish(&self, occurrence: usize) -> Result<(), AdapterError> {
        match self {
            Self::PanicOn(expected) if *expected == occurrence => {
                panic!("simulated process stop after outcome publication");
            }
            Self::ReturnError => Err(AdapterError::new("simulated cleanup failure")),
            Self::None | Self::PanicOn(_) => Ok(()),
        }
    }
}

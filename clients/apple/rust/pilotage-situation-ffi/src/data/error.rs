/// A data operation failed at the Apple boundary.
#[derive(Debug, thiserror::Error, uniffi::Error)]
#[uniffi(flat_error)]
pub enum DataError {
    /// A package did not meet its installation contract.
    #[error(transparent)]
    Package(#[from] pilotage_data_packages::PackageError),
    /// A data update could not be completed.
    #[error(transparent)]
    Delivery(#[from] pilotage_data_delivery::DeliveryError),
    /// A host record could not be decoded or encoded.
    #[error("data JSON could not be processed: {0}")]
    Json(#[from] serde_json::Error),
    /// A bundled example claimed a production channel.
    #[error("bundled examples must use the development channel")]
    BundleChannel,
    /// The serial worker is not available.
    #[error("the data session is not available")]
    SessionUnavailable,
}

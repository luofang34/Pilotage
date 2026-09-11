use std::{io::SeekFrom, path::PathBuf};

use pmtiles::{AsyncBackend, BackendResponse, PmtError, PmtResult};
use tokio::io::{AsyncReadExt, AsyncSeekExt};

const MAX_READ_BYTES: usize = 32 * 1024 * 1024;

pub(crate) struct FileRange {
    pub(crate) path: PathBuf,
}

impl AsyncBackend for FileRange {
    async fn read(&self, offset: usize, length: usize) -> PmtResult<BackendResponse> {
        if length > MAX_READ_BYTES {
            return Err(PmtError::InvalidEntry);
        }
        let mut file = tokio::fs::File::open(&self.path).await?;
        file.seek(SeekFrom::Start(offset as u64)).await?;
        let mut bytes = Vec::with_capacity(length);
        file.take(length as u64).read_to_end(&mut bytes).await?;
        Ok(BackendResponse::new(bytes.into()))
    }
}

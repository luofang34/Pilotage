use std::{
    io::ErrorKind,
    path::{Path, PathBuf},
};

use bytes::Bytes;
use pmtiles::{AsyncPmTilesReader, TileCoord};
use tokio::io::AsyncReadExt;

use crate::{ArchiveError, file_range::FileRange};

/// A PMTiles archive read through bounded file ranges.
pub struct PmTiles {
    path: PathBuf,
    reader: AsyncPmTilesReader<FileRange>,
}

impl PmTiles {
    /// Open an installed archive and read its header and root directory.
    pub async fn open(path: impl AsRef<Path>) -> Result<Self, ArchiveError> {
        let path = path.as_ref().to_path_buf();
        validate_header(&path).await?;
        let reader = AsyncPmTilesReader::try_from_source(FileRange { path: path.clone() })
            .await
            .map_err(|source| ArchiveError::Pmtiles {
                path: path.clone(),
                source,
            })?;
        Ok(Self { path, reader })
    }

    /// Read and decompress one tile. A sparse archive can have no tile at a valid coordinate.
    pub async fn tile(&self, zoom: u8, x: u32, y: u32) -> Result<Option<Bytes>, ArchiveError> {
        let coordinate =
            TileCoord::new(zoom, x, y).map_err(|_| ArchiveError::Coordinate { zoom, x, y })?;
        self.reader
            .get_tile_decompressed(coordinate)
            .await
            .map_err(|source| ArchiveError::Pmtiles {
                path: self.path.clone(),
                source,
            })
    }

    /// Read the archive's JSON metadata.
    pub async fn metadata(&self) -> Result<String, ArchiveError> {
        self.reader
            .get_metadata()
            .await
            .map_err(|source| ArchiveError::Pmtiles {
                path: self.path.clone(),
                source,
            })
    }
}

async fn validate_header(path: &Path) -> Result<(), ArchiveError> {
    let io_error = |source| ArchiveError::Io {
        path: path.to_path_buf(),
        source,
    };
    let header_error = |reason| ArchiveError::Header {
        path: path.to_path_buf(),
        reason,
    };
    let mut file = tokio::fs::File::open(path).await.map_err(io_error)?;
    let file_length = file.metadata().await.map_err(io_error)?.len();
    let mut header = [0u8; 127];
    file.read_exact(&mut header).await.map_err(|source| {
        if source.kind() == ErrorKind::UnexpectedEof {
            header_error("truncated header")
        } else {
            io_error(source)
        }
    })?;
    if &header[..7] != b"PMTiles" || header[7] != 3 {
        return Err(header_error("PMTiles version"));
    }
    for index in [8, 24, 40, 56] {
        let offset = read_u64(&header, index);
        let length = read_u64(&header, index + 8);
        if offset
            .checked_add(length)
            .is_none_or(|end| end > file_length)
        {
            return Err(header_error("section outside archive"));
        }
    }
    let root = read_u64(&header, 8);
    let root_length = read_u64(&header, 16);
    if root < 127 || root.checked_add(root_length).is_none_or(|end| end > 16_384) {
        return Err(header_error("root directory outside initial range"));
    }
    Ok(())
}

fn read_u64(header: &[u8; 127], index: usize) -> u64 {
    u64::from_le_bytes(std::array::from_fn(|offset| header[index + offset]))
}

#[cfg(test)]
mod tests;

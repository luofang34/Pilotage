#![allow(clippy::expect_used)]

use pmtiles::{PmTilesWriter, TileCoord, TileType};

use super::PmTiles;

#[tokio::test]
async fn installed_archive_reads_present_tiles_and_reports_sparse_absence() {
    let directory = tempfile::tempdir().expect("directory");
    let path = directory.path().join("tiles.pmtiles");
    let mut writer = PmTilesWriter::new(TileType::Mvt)
        .metadata("{\"name\":\"offline fixture\"}")
        .create(std::fs::File::create(&path).expect("file"))
        .expect("writer");
    writer
        .add_tile(
            TileCoord::new(2, 1, 2).expect("coordinate"),
            b"tile contents",
        )
        .expect("tile");
    writer.finalize().expect("archive");
    let reader = PmTiles::open(&path).await.expect("open");
    assert_eq!(
        reader
            .tile(2, 1, 2)
            .await
            .expect("tile")
            .expect("present")
            .as_ref(),
        b"tile contents"
    );
    assert!(reader.tile(2, 0, 0).await.expect("absent").is_none());
    assert!(reader.tile(2, 4, 0).await.is_err());
    assert!(
        reader
            .metadata()
            .await
            .expect("metadata")
            .contains("offline fixture")
    );
}

#[tokio::test]
async fn malformed_offsets_and_truncated_headers_return_errors() {
    let directory = tempfile::tempdir().expect("directory");
    let path = directory.path().join("invalid.pmtiles");
    std::fs::write(&path, b"PMTiles").expect("write");
    assert!(PmTiles::open(&path).await.is_err());
    let mut header = [0u8; 127];
    header[..7].copy_from_slice(b"PMTiles");
    header[7] = 3;
    header[8..16].copy_from_slice(&u64::MAX.to_le_bytes());
    std::fs::write(&path, header).expect("write");
    assert!(PmTiles::open(&path).await.is_err());
}

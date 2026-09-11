#![allow(clippy::expect_used)]

use super::MbTiles;

#[test]
fn archive_reads_tms_rows_from_disk_and_reports_missing_tiles() {
    let directory = tempfile::tempdir().expect("directory");
    let path = directory.path().join("tiles.mbtiles");
    let connection = rusqlite::Connection::open(&path).expect("create");
    connection.execute_batch("CREATE TABLE tiles (zoom_level INTEGER, tile_column INTEGER, tile_row INTEGER, tile_data BLOB);
        CREATE TABLE metadata (name TEXT, value TEXT);
        INSERT INTO metadata VALUES ('format', 'png');
        INSERT INTO tiles VALUES (3, 2, 6, x'010203');").expect("fixture");
    drop(connection);
    let archive = MbTiles::open_blocking(&path).expect("open file");
    assert_eq!(
        archive.tile_blocking(3, 2, 1).expect("tile"),
        Some(vec![1, 2, 3])
    );
    assert!(
        archive
            .tile_blocking(3, 2, 6)
            .expect("sparse tile")
            .is_none()
    );
    assert!(archive.tile_blocking(3, 8, 0).is_err());
    assert_eq!(
        archive.metadata_blocking("format").expect("metadata"),
        Some("png".to_owned())
    );
    assert!(archive.connection.execute("DELETE FROM tiles", []).is_err());
}

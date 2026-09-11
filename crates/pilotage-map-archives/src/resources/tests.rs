#![allow(clippy::expect_used)]

use std::sync::Arc;

use super::{ResourceBinding, ResourceError, ResourceFormat, ResourceSet};

fn binding(uri: &str, path: std::path::PathBuf, format: ResourceFormat) -> ResourceBinding {
    ResourceBinding {
        uri: uri.to_owned(),
        path,
        format,
    }
}

#[tokio::test]
async fn compressed_vector_tiles_decode_and_reject_corruption() {
    use std::io::Write;
    let directory = tempfile::tempdir().expect("directory");
    let path = directory.path().join("vector.mbtiles");
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    encoder
        .write_all(b"vector protobuf bytes")
        .expect("compress");
    let compressed = encoder.finish().expect("gzip");
    let connection = rusqlite::Connection::open(&path).expect("database");
    connection.execute_batch("CREATE TABLE tiles (zoom_level INTEGER, tile_column INTEGER, tile_row INTEGER, tile_data BLOB);").expect("schema");
    connection
        .execute("INSERT INTO tiles VALUES (1, 0, 1, ?1)", [&compressed])
        .expect("valid tile");
    connection
        .execute(
            "INSERT INTO tiles VALUES (1, 1, 1, ?1)",
            [&compressed[..compressed.len() - 4]],
        )
        .expect("truncated tile");
    drop(connection);
    let set = ResourceSet::new(vec![binding(
        "pilotage://base",
        path,
        ResourceFormat::Mbtiles,
    )])
    .expect("resources");
    assert_eq!(
        set.fetch("pilotage://base/1/0/0").await.expect("decode"),
        Some(b"vector protobuf bytes".to_vec())
    );
    assert!(matches!(
        set.fetch("pilotage://base/1/1/0").await,
        Err(ResourceError::Compression { .. })
    ));
}

#[test]
fn compressed_tiles_cannot_expand_past_the_resource_limit() {
    use std::io::Write;
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    encoder
        .write_all(&vec![0; super::MAX_RESOURCE_BYTES as usize + 1])
        .expect("compress");
    let bytes = encoder.finish().expect("gzip");
    assert!(matches!(
        super::decode_mbtiles(bytes, std::path::Path::new("archive.mbtiles"), [0, 0, 0]),
        Err(ResourceError::Size { .. })
    ));
}

#[tokio::test]
async fn bound_sprite_files_load_without_exposing_other_files() {
    let directory = tempfile::tempdir().expect("directory");
    let image = directory.path().join("point-sprites.png");
    std::fs::write(&image, b"image bytes").expect("image");
    let set = ResourceSet::new(vec![binding(
        "pilotage://symbols/point-sprites.png",
        image,
        ResourceFormat::File,
    )])
    .expect("bindings");
    assert_eq!(
        set.fetch("pilotage://symbols/point-sprites.png")
            .await
            .expect("image"),
        Some(b"image bytes".to_vec())
    );
    for uri in [
        "file:///etc/passwd",
        "pilotage://symbols/../point-sprites.png",
        "pilotage://symbols/%2e%2e/point-sprites.png",
    ] {
        assert!(matches!(
            set.fetch(uri).await,
            Err(ResourceError::Uri { .. })
        ));
    }
    assert!(set.fetch("pilotage://symbols/missing.png").await.is_err());
}

#[tokio::test]
async fn tile_workers_share_an_installed_pmtiles_archive() {
    let directory = tempfile::tempdir().expect("directory");
    let path = directory.path().join("tiles.pmtiles");
    let mut writer = pmtiles::PmTilesWriter::new(pmtiles::TileType::Mvt)
        .create(std::fs::File::create(&path).expect("file"))
        .expect("writer");
    writer
        .add_tile(
            pmtiles::TileCoord::new(2, 1, 2).expect("coordinate"),
            b"tile bytes",
        )
        .expect("tile");
    writer.finalize().expect("archive");
    let set = Arc::new(
        ResourceSet::new(vec![binding(
            "pilotage://network",
            path,
            ResourceFormat::Pmtiles,
        )])
        .expect("bindings"),
    );
    let mut workers = tokio::task::JoinSet::new();
    for _ in 0..8 {
        let resources = Arc::clone(&set);
        workers.spawn(async move { resources.fetch("pilotage://network/2/1/2").await });
    }
    while let Some(result) = workers.join_next().await {
        assert_eq!(
            result.expect("worker").expect("tile"),
            Some(b"tile bytes".to_vec())
        );
    }
    assert!(
        set.fetch("pilotage://network/2/0/0")
            .await
            .expect("sparse tile")
            .is_none()
    );
    assert!(set.fetch("pilotage://network/2/4/0").await.is_err());
    assert!(matches!(
        set.fetch("pilotage://missing/2/1/2").await,
        Err(ResourceError::Unknown { .. })
    ));
}

#[tokio::test]
async fn terrain_resources_convert_xyz_rows_on_a_blocking_worker() {
    let directory = tempfile::tempdir().expect("directory");
    let path = directory.path().join("terrain.mbtiles");
    let connection = rusqlite::Connection::open(&path).expect("database");
    connection.execute_batch("CREATE TABLE tiles (zoom_level INTEGER, tile_column INTEGER, tile_row INTEGER, tile_data BLOB);
        INSERT INTO tiles VALUES (3, 2, 6, x'010203');").expect("fixture");
    drop(connection);
    let set = ResourceSet::new(vec![binding(
        "pilotage://terrain",
        path,
        ResourceFormat::Mbtiles,
    )])
    .expect("bindings");
    assert_eq!(
        set.fetch("pilotage://terrain/3/2/1").await.expect("tile"),
        Some(vec![1, 2, 3])
    );
    assert!(
        set.fetch("pilotage://terrain/3/2/6")
            .await
            .expect("sparse tile")
            .is_none()
    );
}

#[tokio::test]
async fn resource_size_and_duplicate_bindings_fail_explicitly() {
    let directory = tempfile::tempdir().expect("directory");
    let path = directory.path().join("oversized.png");
    std::fs::File::create(&path)
        .expect("file")
        .set_len(super::MAX_RESOURCE_BYTES + 1)
        .expect("length");
    let resource = binding(
        "pilotage://symbols/oversized.png",
        path,
        ResourceFormat::File,
    );
    assert!(matches!(
        ResourceSet::new(vec![resource.clone(), resource.clone()]),
        Err(ResourceError::Duplicate { .. })
    ));
    let set = ResourceSet::new(vec![resource]).expect("bindings");
    assert!(matches!(
        set.fetch("pilotage://symbols/oversized.png").await,
        Err(ResourceError::Size { .. })
    ));
}

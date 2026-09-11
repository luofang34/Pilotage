#![allow(clippy::expect_used)]

use std::{
    io::{Read, Write},
    net::TcpListener,
    thread::{self, JoinHandle},
    time::Duration,
};

use pilotage_data_packages::{Artifact, ArtifactFormat, ContentDigest, PackagePath};

use super::*;

fn artifact(bytes: &[u8]) -> Artifact {
    Artifact {
        path: PackagePath::try_from("chart.bin".to_owned()).expect("artifact path"),
        source: "chart.bin".to_owned(),
        format: ArtifactFormat::Resource,
        bytes: bytes.len() as u64,
        sha256: ContentDigest::of_bytes(bytes),
    }
}

fn serve(response: Vec<u8>) -> (Url, JoinHandle<std::io::Result<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind local HTTP fixture");
    let url = Url::parse(&format!(
        "http://{}/object",
        listener.local_addr().expect("local address")
    ))
    .expect("fixture URL");
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept()?;
        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        let mut request = Vec::new();
        let mut byte = [0u8; 1];
        while !request.ends_with(b"\r\n\r\n") {
            stream.read_exact(&mut byte)?;
            request.push(byte[0]);
        }
        stream.write_all(&response)?;
        Ok(String::from_utf8_lossy(&request).into_owned())
    });
    (url, handle)
}

fn response(status: &str, headers: &str, body: &[u8]) -> Vec<u8> {
    let mut value = format!("HTTP/1.1 {status}\r\nConnection: close\r\n{headers}\r\n").into_bytes();
    value.extend_from_slice(body);
    value
}

#[test]
fn range_resume_and_range_ignored_both_produce_exact_content() {
    for honor_range in [true, false] {
        let root = tempfile::tempdir().expect("temporary store");
        let mut store = PackageStore::open_blocking(root.path()).expect("store");
        let bytes = b"abcdefghijklmnopqrstuvwxyz";
        let item = artifact(bytes);
        store
            .append_blocking(&item, 0, &bytes[..7])
            .expect("partial download");
        let wire = if honor_range {
            response(
                "206 Partial Content",
                "Content-Range: bytes 7-25/26\r\nContent-Length: 19\r\n",
                &bytes[7..],
            )
        } else {
            response("200 OK", "Content-Length: 26\r\n", bytes)
        };
        let (url, server) = serve(wire);
        let mut events = Vec::new();
        download_blocking(
            &Client::new(),
            &mut store,
            &Download {
                artifact: item.clone(),
                offset: 7,
            },
            &url,
            &|| false,
            &mut |event| events.push(event),
        )
        .expect("download");
        let request = server
            .join()
            .expect("server thread")
            .expect("serve request");
        assert!(request.to_ascii_lowercase().contains("range: bytes=7-"));
        assert!(
            request
                .to_ascii_lowercase()
                .contains("accept-encoding: identity")
        );
        let partial = root
            .path()
            .join("staging")
            .join(format!("{}.part", item.sha256.as_str()));
        assert_eq!(std::fs::read(partial).expect("staged data"), bytes);
        assert_eq!(events.last().expect("progress").received, 26);
    }
}

#[test]
fn wrong_range_and_encoded_response_preserve_existing_partial() {
    for headers in [
        "Content-Range: bytes 0-25/26\r\nContent-Length: 26\r\n",
        "Content-Range: bytes 7-25/26\r\nContent-Encoding: gzip\r\nContent-Length: 19\r\n",
    ] {
        let root = tempfile::tempdir().expect("temporary store");
        let mut store = PackageStore::open_blocking(root.path()).expect("store");
        let item = artifact(b"abcdefghijklmnopqrstuvwxyz");
        store
            .append_blocking(&item, 0, b"abcdefg")
            .expect("partial download");
        let (url, server) = serve(response(
            "206 Partial Content",
            headers,
            b"hijklmnopqrstuvwxyz",
        ));
        assert!(
            download_blocking(
                &Client::new(),
                &mut store,
                &Download {
                    artifact: item.clone(),
                    offset: 7
                },
                &url,
                &|| false,
                &mut |_| {}
            )
            .is_err()
        );
        server
            .join()
            .expect("server thread")
            .expect("serve request");
        let partial = root
            .path()
            .join("staging")
            .join(format!("{}.part", item.sha256.as_str()));
        assert_eq!(std::fs::read(partial).expect("staged data"), b"abcdefg");
    }
}

#[test]
fn interrupted_body_retains_bytes_and_cancellation_does_not_start_a_request() {
    let root = tempfile::tempdir().expect("temporary store");
    let mut store = PackageStore::open_blocking(root.path()).expect("store");
    let item = artifact(b"abcdefghijklmnopqrstuvwxyz");
    let download = Download {
        artifact: item.clone(),
        offset: 0,
    };
    let (url, server) = serve(response("200 OK", "Content-Length: 26\r\n", b"abcdefg"));
    assert!(
        download_blocking(
            &Client::new(),
            &mut store,
            &download,
            &url,
            &|| false,
            &mut |_| {}
        )
        .is_err()
    );
    server
        .join()
        .expect("server thread")
        .expect("serve request");
    let partial = root
        .path()
        .join("staging")
        .join(format!("{}.part", item.sha256.as_str()));
    assert_eq!(std::fs::read(partial).expect("staged data"), b"abcdefg");
    assert!(matches!(
        download_blocking(
            &Client::new(),
            &mut store,
            &download,
            &url,
            &|| true,
            &mut |_| {}
        ),
        Err(DeliveryError::Cancelled)
    ));
}

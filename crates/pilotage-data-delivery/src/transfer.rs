use std::io::Read;

use pilotage_data_packages::{Download, PackageStore};
use reqwest::{
    StatusCode, Url,
    blocking::{Client, Response},
    header,
};
use serde::Serialize;

use crate::{
    DeliveryError,
    error::{http_error, response_error},
};

/// Transfer state for one immutable object.
#[derive(Debug, Clone, Serialize)]
pub struct DownloadProgress {
    /// Relative path within the release.
    pub path: String,
    /// Received bytes, including a retained partial download.
    pub received: u64,
    /// Required complete object length.
    pub total: u64,
}

pub(crate) fn download_blocking(
    client: &Client,
    store: &mut PackageStore,
    download: &Download,
    url: &Url,
    cancelled: &dyn Fn() -> bool,
    progress: &mut dyn FnMut(DownloadProgress),
) -> Result<(), DeliveryError> {
    if cancelled() {
        return Err(DeliveryError::Cancelled);
    }
    let artifact = &download.artifact;
    if download.offset == artifact.bytes {
        return Ok(());
    }
    let mut request = client
        .get(url.clone())
        .header(header::ACCEPT_ENCODING, "identity");
    if download.offset > 0 {
        request = request.header(header::RANGE, format!("bytes={}-", download.offset));
    }
    let mut response = request
        .send()
        .and_then(|r| r.error_for_status())
        .map_err(|source| http_error(url.as_str(), source))?;
    let mut offset = response_offset(&response, download, url.as_str())?;
    if offset == 0 && download.offset > 0 {
        store.discard_download_blocking(&artifact.sha256)?;
    }
    let mut buffer = vec![0u8; 1024 * 1024];
    loop {
        if cancelled() {
            return Err(DeliveryError::Cancelled);
        }
        let count = response
            .read(&mut buffer)
            .map_err(|source| DeliveryError::Read {
                url: url.to_string(),
                source,
            })?;
        if count == 0 {
            break;
        }
        store.append_blocking(artifact, offset, &buffer[..count])?;
        offset = offset.saturating_add(count as u64);
        progress(DownloadProgress {
            path: artifact.path.as_str().to_owned(),
            received: offset,
            total: artifact.bytes,
        });
    }
    if offset != artifact.bytes {
        return Err(response_error(
            url.as_str(),
            format!("received {offset} of {} bytes", artifact.bytes),
        ));
    }
    Ok(())
}

fn response_offset(
    response: &Response,
    download: &Download,
    url: &str,
) -> Result<u64, DeliveryError> {
    if response
        .headers()
        .get(header::CONTENT_ENCODING)
        .is_some_and(|v| v != "identity")
    {
        return Err(response_error(url, "encoded artifact response"));
    }
    let offset = match response.status() {
        StatusCode::OK => 0,
        StatusCode::PARTIAL_CONTENT => {
            let expected = format!(
                "bytes {}-{}/{}",
                download.offset,
                download.artifact.bytes.saturating_sub(1),
                download.artifact.bytes
            );
            if response
                .headers()
                .get(header::CONTENT_RANGE)
                .and_then(|v| v.to_str().ok())
                != Some(&expected)
            {
                return Err(response_error(
                    url,
                    "Content-Range does not match the object",
                ));
            }
            download.offset
        }
        status => {
            return Err(response_error(
                url,
                format!("unexpected HTTP status {status}"),
            ));
        }
    };
    if let Some(length) = response.content_length()
        && length != download.artifact.bytes - offset
    {
        return Err(response_error(
            url,
            "Content-Length does not match the object",
        ));
    }
    Ok(offset)
}

#[cfg(test)]
mod tests;

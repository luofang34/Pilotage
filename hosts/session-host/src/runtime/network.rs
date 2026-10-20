//! Bind policy and secret loading for the session transport.

use std::io::Read;
use std::net::{IpAddr, Ipv4Addr};
use std::path::{Path, PathBuf};

use pilotage_protocol::{JoinToken, JoinTokenError};

/// Network admission policy, independent of the selected vehicle adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkOptions {
    /// Interface address to bind. The default accepts only local connections.
    pub bind_ip: IpAddr,
    /// Required admission secret, including on loopback when configured.
    pub join_token: Option<JoinToken>,
}

impl Default for NetworkOptions {
    fn default() -> Self {
        Self {
            bind_ip: Ipv4Addr::LOCALHOST.into(),
            join_token: None,
        }
    }
}

/// Network configuration cannot provide the requested admission boundary.
#[derive(Debug, thiserror::Error)]
pub enum NetworkError {
    /// Remote interfaces require an admission secret.
    #[error("non-loopback bind {bind_ip} requires a join-token file")]
    RemoteTokenRequired {
        /// Requested interface address.
        bind_ip: IpAddr,
    },
    /// The secret file path must be absolute.
    #[error("join-token file path must be absolute: {path}")]
    RelativeTokenPath {
        /// Requested file path.
        path: PathBuf,
    },
    /// The secret file could not be read.
    #[error("cannot read join-token file {path}: {source}")]
    TokenRead {
        /// Requested file path.
        path: PathBuf,
        /// File access error.
        #[source]
        source: std::io::Error,
    },
    /// The secret file does not contain a valid token.
    #[error("invalid join-token file {path}: {source}")]
    TokenFormat {
        /// Requested file path.
        path: PathBuf,
        /// Format error without secret contents.
        #[source]
        source: JoinTokenError,
    },
}

impl NetworkOptions {
    /// Read a 64-digit hexadecimal token file, with optional surrounding whitespace.
    ///
    /// # Errors
    ///
    /// Returns [`NetworkError`] if the path, contents, or bind policy is invalid.
    pub fn from_token_file_blocking(
        bind_ip: IpAddr,
        path: Option<&Path>,
    ) -> Result<Self, NetworkError> {
        let options = Self {
            bind_ip,
            join_token: path.map(read_token_blocking).transpose()?,
        };
        options.validate()?;
        Ok(options)
    }

    /// Require credentials before exposing a non-loopback interface.
    ///
    /// # Errors
    ///
    /// Returns [`NetworkError::RemoteTokenRequired`] for an unprotected remote bind.
    pub fn validate(&self) -> Result<(), NetworkError> {
        if !self.bind_ip.is_loopback() && self.join_token.is_none() {
            return Err(NetworkError::RemoteTokenRequired {
                bind_ip: self.bind_ip,
            });
        }
        Ok(())
    }
}

fn read_token_blocking(path: &Path) -> Result<JoinToken, NetworkError> {
    if !path.is_absolute() {
        return Err(NetworkError::RelativeTokenPath {
            path: path.to_owned(),
        });
    }
    let read_error = |source| NetworkError::TokenRead {
        path: path.to_owned(),
        source,
    };
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .map_err(read_error)?
        .take(256)
        .read_to_end(&mut bytes)
        .map_err(read_error)?;
    if bytes.len() >= 256 {
        return Err(NetworkError::TokenFormat {
            path: path.to_owned(),
            source: JoinTokenError::InvalidLength,
        });
    }
    let value = std::str::from_utf8(&bytes).map_err(|_| NetworkError::TokenFormat {
        path: path.to_owned(),
        source: JoinTokenError::InvalidHex,
    })?;
    JoinToken::from_hex(value.trim()).map_err(|source| NetworkError::TokenFormat {
        path: path.to_owned(),
        source,
    })
}

#[cfg(test)]
mod tests;

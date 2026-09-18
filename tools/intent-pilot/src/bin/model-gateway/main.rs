//! `model-gateway`: gives a client that cannot start a process the model port
//! over HTTP. It holds one model adapter process and serves its declaration
//! and its replies. It holds no authority and sends nothing to a vehicle.

mod cli;

use std::process::ExitCode;
use std::sync::Arc;

use intent_pilot::gateway::{GatewayConfig, GatewayError, serve};
use intent_pilot::{ModelProcess, ModelProcessError};
use tokio::net::TcpListener;
use tokio::sync::Notify;

/// Why the gateway stopped.
#[derive(Debug, thiserror::Error)]
enum MainError {
    /// The async runtime did not start.
    #[error("cannot start the async runtime")]
    Runtime(#[source] std::io::Error),
    /// The listen address is not usable.
    #[error("cannot listen on {address}")]
    Bind {
        /// The requested address.
        address: String,
        #[source]
        source: std::io::Error,
    },
    /// The model adapter did not start.
    #[error("the model adapter did not start")]
    Model(#[source] ModelProcessError),
    /// The server failed.
    #[error("the gateway stopped")]
    Serve(#[source] GatewayError),
}

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_writer(std::io::stderr)
        .init();
    let options = match cli::parse(std::env::args().skip(1)) {
        Ok(cli::Invocation::Run(options)) => options,
        Ok(cli::Invocation::Help) => {
            tracing::info!("{}", cli::USAGE);
            return ExitCode::SUCCESS;
        }
        Err(detail) => {
            tracing::error!(%detail, "cannot start\n{}", cli::USAGE);
            return ExitCode::FAILURE;
        }
    };
    let outcome = tokio::runtime::Runtime::new()
        .map_err(MainError::Runtime)
        .and_then(|runtime| runtime.block_on(run(options)));
    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            let mut chain = String::new();
            let mut source = std::error::Error::source(&error);
            while let Some(cause) = source {
                chain.push_str(&format!(": {cause}"));
                source = cause.source();
            }
            tracing::error!("{error}{chain}");
            ExitCode::FAILURE
        }
    }
}

async fn run(options: cli::Options) -> Result<(), MainError> {
    let model = ModelProcess::spawn(&options.model_cmd)
        .await
        .map_err(MainError::Model)?;
    let listener = TcpListener::bind(&options.listen)
        .await
        .map_err(|source| MainError::Bind {
            address: options.listen.clone(),
            source,
        })?;
    if let Ok(address) = listener.local_addr()
        && !address.ip().is_loopback()
    {
        tracing::warn!(
            %address,
            "the gateway listens beyond loopback; it has no authentication, so each host that reaches it can drive the adapter"
        );
    }
    let declared = model.declaration();
    tracing::info!(
        listen = %options.listen,
        adapter = %declared.adapter,
        model = %declared.model,
        origins = ?options.allowed_origins,
        "model gateway ready"
    );
    let shutdown = Arc::new(Notify::new());
    let on_signal = Arc::clone(&shutdown);
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            on_signal.notify_one();
        }
    });
    let config = GatewayConfig {
        allowed_origins: options.allowed_origins,
    };
    serve(listener, model, config, shutdown)
        .await
        .map_err(MainError::Serve)
}

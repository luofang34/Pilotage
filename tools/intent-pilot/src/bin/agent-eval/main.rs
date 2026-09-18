//! `agent-eval`: scores a model adapter against a case suite.
//!
//! Every adapter gets the same requests and the same scoring rule. The result
//! names the suite by its digest, and an append-only ledger counts how many
//! times an adapter ran that digest. A first run of a held-out suite is thus
//! visible, and a repeated run is visible too.

mod cli;
mod error;
mod ledger;
mod report;

use std::io::Write;
use std::process::ExitCode;

use intent_pilot::ModelProcess;
use pilotage_agent::{CaseResult, ModelRequest, Suite, score_case, summarize};
use sha2::{Digest, Sha256};

use crate::error::EvalError;

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_writer(std::io::stderr)
        .init();
    let options = match cli::parse(std::env::args().skip(1)) {
        Ok(cli::Invocation::Run(options)) => options,
        Ok(cli::Invocation::Help) => {
            emit(cli::USAGE);
            return ExitCode::SUCCESS;
        }
        Err(error) => {
            tracing::error!(%error, "cannot start");
            emit(cli::USAGE);
            return ExitCode::FAILURE;
        }
    };
    let outcome = tokio::runtime::Runtime::new()
        .map_err(EvalError::Runtime)
        .and_then(|runtime| runtime.block_on(run(&options)));
    match outcome {
        Ok(table) => {
            emit(&table);
            ExitCode::SUCCESS
        }
        Err(error) => {
            let mut chain = String::new();
            let mut source = std::error::Error::source(&error);
            while let Some(cause) = source {
                chain.push_str(&format!(": {cause}"));
                source = cause.source();
            }
            tracing::error!("the evaluation stopped: {error}{chain}");
            ExitCode::FAILURE
        }
    }
}

async fn run(options: &cli::Options) -> Result<String, EvalError> {
    let bytes = tokio::fs::read(&options.suite)
        .await
        .map_err(|source| EvalError::Read {
            path: options.suite.clone(),
            source,
        })?;
    let suite_sha256 = hex(&Sha256::digest(&bytes));
    let suite =
        Suite::parse(&String::from_utf8_lossy(&bytes)).map_err(|source| EvalError::Suite {
            path: options.suite.clone(),
            source,
        })?;
    let mut model = ModelProcess::spawn(&options.model_cmd)
        .await
        .map_err(EvalError::Model)?;
    let declaration = model.declaration().clone();
    tracing::info!(adapter = %declaration.adapter, model = %declaration.model, cases = suite.cases.len(), "start");

    let mut results: Vec<CaseResult> = Vec::with_capacity(suite.cases.len());
    for case in &suite.cases {
        let request = ModelRequest {
            id: 0,
            message: case.message.clone(),
            envelope: suite.envelope.clone(),
            legend: suite.legend.clone(),
            frames: case.frames.clone(),
        };
        let too_many = case.frames.len() > usize::from(declaration.frames.max_frames);
        let reply = if too_many {
            Err(format!(
                "the case has {} frames and the adapter accepts {}",
                case.frames.len(),
                declaration.frames.max_frames
            ))
        } else {
            model.ask(&request).await.map_err(|error| error.to_string())
        };
        let result = score_case(
            case,
            &suite.envelope,
            reply.as_ref().map(|(reply, _)| reply).map_err(Clone::clone),
        );
        tracing::info!(case = %case.id, outcome = ?result.outcome, "scored");
        results.push(result);
    }
    model.stop().await;

    let summary = summarize(&results);
    let entry = ledger::Entry::new(&suite, &suite_sha256, &declaration, &summary);
    let run_number = ledger::append(&options.ledger, entry).await?;
    report::write(
        &options.out,
        &suite,
        &suite_sha256,
        &declaration,
        run_number,
        &results,
        &summary,
    )
    .await?;
    Ok(report::table(
        &suite,
        &suite_sha256,
        &declaration,
        run_number,
        &summary,
    ))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Writes program output. A closed stdout is not an error the tool can act on.
fn emit(text: &str) {
    writeln!(std::io::stdout(), "{text}").ok();
}

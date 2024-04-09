//! This is useful for fetching [ProverInput] per block
use prover::ProverInput;
use rpc::{FetchProverInputRequest, fetch_prover_input};
use super::input::BlockSource;
use log::{error, info};
use anyhow::Error;

//==============================================================================
// FetchError
//==============================================================================
#[derive(Debug)]
pub enum FetchError {
    ZeroBinRpcFetchError(Error)
}

impl std::fmt::Display for FetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(&self, f)
    }
}

impl std::error::Error for FetchError {}

//=============================================================================
// Fetching
//=============================================================================

/// Fetches the prover input given the [BlockSource]
pub async fn fetch(
    block_number: u64,
    checkpoint_block_number: Option<u64>,
    source: &BlockSource,
) -> Result<ProverInput, FetchError> {

    match source {
        // Use ZeroBing's RPC fetch
        BlockSource::ZeroBinRpc { rpc_url } => {
            info!("Requesting from block {} from RPC ({})", block_number, rpc_url);
            let fetch_prover_input_request = FetchProverInputRequest {
                rpc_url: rpc_url.as_str(),
                block_number: block_number,
                checkpoint_block_number: match checkpoint_block_number {
                    Some(checkpoint) => checkpoint,
                    None if block_number == 0 => 0,
                    None => block_number - 1,
                }
            };

            match fetch_prover_input(fetch_prover_input_request).await {
                Ok(prover_input) => Ok(prover_input),
                Err(err) => {
                    error!("Failed to fetch prover input: {}", err);
                    Err(FetchError::ZeroBinRpcFetchError(err))
                }

            }
        }
    }
}

//! This is useful for fetching [ProverInput] per block
use anyhow::Error;
use prover::ProverInput;
use rpc::{fetch_prover_input, FetchProverInputRequest};
use tracing::{error, info};

use super::input::BlockSource;

//==============================================================================
// FetchError
//==============================================================================
#[derive(Debug)]
pub enum FetchError {
    ZeroBinRpcFetchError(Error),
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

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub enum Checkpoint {
    Constant(u64),
    BlockNumberNegativeOffset(u64),
}

impl Default for Checkpoint {
    fn default() -> Self {
        Self::BlockNumberNegativeOffset(1)
    }
}

impl Checkpoint {
    pub fn get_checkpoint(&self, block_number: u64) -> u64 {
        match self {
            Self::BlockNumberNegativeOffset(offset) if block_number > *offset => {
                block_number - offset
            }
            Self::BlockNumberNegativeOffset(_) => 0,
            Self::Constant(constant_value) => *constant_value,
        }
    }
}

/// Fetches the prover input given the [BlockSource]
pub async fn fetch(
    block_number: u64,
    checkpoint_method: &Option<Checkpoint>,
    source: &BlockSource,
) -> Result<ProverInput, FetchError> {
    match source {
        // Use ZeroBing's RPC fetch
        BlockSource::ZeroBinRpc { rpc_url } => {
            info!(
                "Requesting from block {} from RPC ({})",
                block_number, rpc_url
            );
            let fetch_prover_input_request = FetchProverInputRequest {
                rpc_url: rpc_url.as_str(),
                block_number,
                checkpoint_block_number: checkpoint_method
                    .unwrap_or_default()
                    .get_checkpoint(block_number),
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

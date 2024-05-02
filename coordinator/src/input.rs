//! This module contains a lot of the important input structs
use serde::{Deserialize, Serialize};

use crate::benchmarking::BenchmarkOutputConfig;

/// The means for terminating.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum TerminateOn {
    /// Terminate after `num_seconds` seconds after the start of proving blocks.
    ///
    /// Note: The manyprover may continue to operate after the `num_seconds`,
    /// but will not begin a new proof or record any proofs finalized after
    /// being considered terminated.
    ElapsedSeconds {
        /// The number of seconds needed to elapse since the beginning of the
        /// proving process before terminating.
        num_seconds: u64,
    },
    /// Prove until the sum of gas of all the blocks we proved is equal to
    /// `until_gas_sum` amount of gas.
    BlockGasUsed {
        /// Sets the gas
        until_gas_sum: u64,
    },
    /// Terminate once proved the end block, given by the `block_number`
    /// (inclusive)
    EndBlock {
        /// The block number considered to be the end block, inclusive.
        block_number: u64,
    },
}

/// The source of Blocks to produce the [prover::ProverInput].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BlockSource {
    /// Utilize the RPC function provided by ZeroBin to get the
    /// [prover::ProverInput]
    ZeroBinRpc {
        /// The url of the RPC
        rpc_url: String,
    },
}

/// The [BlockConcurrencyMode] represents how we handle the block
/// processing.  
///
/// In Sequential mode, we will never send more than
/// one block at a time to the workers, however this may lead to
/// reduced runtime due to unoccupied workers.
///
/// In concurrent mode, we will try to have at most `max_concurrent`
/// blocks with their workloads currently distributed to the
/// workers.
#[derive(Debug, Default, Serialize, Deserialize, Clone, Copy)]
pub enum BlockConcurrencyMode {
    #[default]
    Sequential,
    Parallel {
        max_concurrent: u8,
        /// Represents the maximum number of blocks we can do, if not provided
        /// will have to pick a number on its own
        max_blocks: Option<u64>,
    },
}

impl BlockConcurrencyMode {
    pub fn max_concurrent(&self) -> Option<u8> {
        match self {
            BlockConcurrencyMode::Parallel {
                max_concurrent,
                max_blocks: _,
            } => Some(*max_concurrent),
            _ => None,
        }
    }

    pub fn max_blocks(&self) -> Option<u64> {
        match self {
            Self::Sequential => None,
            Self::Parallel {
                max_concurrent: _,
                max_blocks,
            } => Some(max_blocks.unwrap_or(1000)),
        }
    }
}

use crate::proofout::ProofOutputMethod;

/// The input for starting the many-blocks proving
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProveBlocksInput {
    /// The name of the run
    pub run_name: Option<String>,
    /// The starting block number
    pub start_block_number: u64,
    /// The checkpoint block number.  If not provided, will be the
    /// `start_block_number` - 1.
    pub checkpoint_block_number: Option<u64>,
    /// The termination condition.  If not provided, will not terminate until
    /// exhausted or an error occurs.
    pub terminate_on: Option<TerminateOn>,
    /// How we source the blocks.
    pub block_source: BlockSource,
    /// The conccurency mode
    pub block_concurrency: Option<BlockConcurrencyMode>,
    /// DEPRECATED
    pub check_gas: Option<bool>,
    /// Stores the output of the proofs. If not provided, no proofs will be
    /// stored
    pub proof_output: Option<ProofOutputMethod>,
    /// Stores the output of the benchmark.  If not provided, no benchmarking
    /// stats will be stored
    pub benchmark_output: Option<BenchmarkOutputConfig>,
    /// Whether or not we should forward the previous proof to the next proof.
    ///
    /// NOTE: There may be some problems if set to true.  Default is false.
    pub forward_prev: Option<bool>,
}

impl ProveBlocksInput {
    pub fn get_expected_number_proofs(&self) -> Option<u64> {
        match self.terminate_on {
            Some(TerminateOn::EndBlock { block_number }) => {
                Some((block_number - self.start_block_number) + 1)
            }
            _ => None,
        }
    }

    /// Returns the estimated number of proofs that will be generated.
    /// If unable to produce an estimate, returns [None]
    ///
    /// This is largely based on the termination condition ([TerminateOn])
    pub fn estimate_expected_number_proofs(&self) -> Option<u64> {
        self.get_expected_number_proofs()
    }

    /// Returns either the checkpoint value or the start block number - 1
    pub fn get_checkpoint_block_number(&self) -> u64 {
        match self.checkpoint_block_number {
            Some(checkpoint) => checkpoint,
            None => self.start_block_number - 1,
        }
    }
}

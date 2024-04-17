//! This module contains a lot of the important input structs
use serde::{Deserialize, Serialize};

use crate::benchmarking::BenchmarkOutputConfig;

/// The means for terminating.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
        /// Whether or not we should record a block proof if the proof was
        /// started but not completed before the elapsed time.
        ///
        /// The default value is false.
        include_straddling: Option<bool>,
    },
    /// Prove until the sum of gas of all the blocks we proved is equal to
    /// `until_gas_sum` amount of gas.
    BlockGasUsed {
        /// Sets the gas
        until_gas_sum:u64,
    },
    /// Terminate after proving `num_blocks` number of blocks
    NumBlocks {
        /// The number of blocks to be proved before terminating.
        num_blocks: u64,
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
    pub forward_prev: Option<bool>
}

impl ProveBlocksInput {
    /// Returns the estimated number of proofs that will be generated.
    /// If unable to produce an estimate, returns [None]
    ///
    /// This is largely based on the termination condition ([TerminateOn])
    pub fn estimate_expected_number_proofs(&self) -> Option<usize> {
        match self.terminate_on {
            Some(TerminateOn::EndBlock { block_number }) => {
                Some(((block_number - self.start_block_number) + 1) as usize)
            }
            Some(TerminateOn::NumBlocks { num_blocks }) => Some((num_blocks + 1) as usize),
            _ => None,
        }
    }

    /// Returns either the checkpoint value or the start block number - 1
    pub fn get_checkpoint_block_number(&self) -> u64 {
        match self.checkpoint_block_number {
            Some(checkpoint) => checkpoint,
            None => self.start_block_number - 1,
        }
    }
}

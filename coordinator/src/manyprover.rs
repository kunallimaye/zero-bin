//! This module contains everything to prove multiple blocks.
use std::time::Instant;

use log::{debug, error, info};
use paladin::runtime::Runtime;
use proof_gen::types::PlonkyProofIntern;

use crate::benchmarking::{
    BenchmarkingOutput, BenchmarkingOutputBuildError, BenchmarkingOutputError, BenchmarkingStats,
};
use crate::fetch::{fetch, FetchError};
use crate::input::{ProveBlocksInput, TerminateOn};
use crate::proofout::{ProofOutput, ProofOutputBuildError, ProofOutputError};

//===========================================================================================
// ManyProverError
//===========================================================================================
#[derive(Debug)]
pub enum ManyProverError {
    Fetch(FetchError),
    Proof(anyhow::Error),
    BenchmarkingOutputBuild(BenchmarkingOutputBuildError),
    BenchmarkingOutput(BenchmarkingOutputError),
    ProofOutBuildError(ProofOutputBuildError),
    ProofOutError(ProofOutputError),
}

impl std::fmt::Display for ManyProverError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(&self, f)
    }
}

impl std::error::Error for ManyProverError {}

//===========================================================================================
// Proving function
//===========================================================================================

/// Proves many blocks
pub async fn prove_blocks(
    input: ProveBlocksInput,
    runtime: &Runtime,
) -> Result<(), ManyProverError> {
    info!("Initializing to prove blocks...");

    info!("{:#?}", input);

    //=================================================================================
    // Init & Setup
    //=================================================================================

    debug!("Preparing proof output...");
    // get the proof
    let proof_out = match &input.proof_output {
        Some(proof_method) => match ProofOutput::from_method(proof_method) {
            Ok(proof_out) => Some(proof_out),
            Err(err) => {
                error!("Failed to build proof out");
                return Err(ManyProverError::ProofOutBuildError(err));
            }
        },
        None => None,
    };

    debug!("Preparing benchmark output...");
    let mut benchmark_out = match &input.benchmark_output {
        Some(benchmark_config) => {
            match BenchmarkingOutput::from_config(
                benchmark_config.clone(),
                input.estimate_expected_number_proofs(),
            )
            .await
            {
                Ok(benchmark_output) => Some(benchmark_output),
                Err(err) => {
                    error!("Failed to construct Benchmark Output: {}", err);
                    return Err(ManyProverError::BenchmarkingOutputBuild(err));
                }
            }
        }
        None => {
            info!("Was not provided means to place benchmarking statistics output...");
            None
        }
    };

    let mut remaining_gas: Option<u64> = match input.terminate_on {
        Some(TerminateOn::BlockGasUsed { until_gas_sum }) => {
            info!("Starting with {} gas units.", until_gas_sum);
            Some(until_gas_sum)
        }
        _ => None,
    };

    // Stores the previous PlonkyProofIntern if applicable
    let mut prev: Option<PlonkyProofIntern> = None;
    // Stores the current block number
    let mut cur_block_num: u64 = input.start_block_number;

    //========================================================================
    // Performing the proofs
    //========================================================================

    info!("Starting the proof process");
    let benchmark_start_instance = Instant::now();

    loop {
        // Determine if we should begin a proof depending on the termination policy.
        match input.terminate_on {
            Some(TerminateOn::ElapsedSeconds {
                num_seconds,
                include_straddling: _,
            }) => {
                if benchmark_start_instance.elapsed().as_secs() >= num_seconds {
                    info!(
                        "Terminating on block {} due to reaching time constraint.",
                        cur_block_num
                    );
                    break;
                }
            }
            Some(TerminateOn::EndBlock { block_number }) if cur_block_num > block_number => {
                info!(
                    "Terminating on block {} as we have surpassed the end block.",
                    cur_block_num
                );
                break;
            }
            Some(TerminateOn::NumBlocks { num_blocks })
                if cur_block_num >= (input.start_block_number + num_blocks) =>
            {
                info!(
                    "Terminating on block {} as we have proved {} number of blocks",
                    cur_block_num, num_blocks
                );
                break;
            }
            _ => {
                info!("Starting attempt to prove block {}", cur_block_num);
            }
        }

        //------------------------------------------------------------------------
        // Fetching
        //------------------------------------------------------------------------

        debug!("Attempting to fetch block {}", cur_block_num);
        let fetch_start_instance = Instant::now();
        let prover_input = match fetch(
            cur_block_num,
            input.checkpoint_block_number,
            &input.block_source,
        )
        .await
        {
            Ok(prover_input) => prover_input,
            Err(err) => {
                error!("Failed to fetch block number: {}", cur_block_num);
                return Err(ManyProverError::Fetch(err));
            }
        };
        let fetch_duration = fetch_start_instance.elapsed();
        info!(
            "Fetched block {} in {} seconds",
            cur_block_num,
            fetch_duration.as_secs_f64()
        );

        //------------------------------------------------------------------------
        // Extract some key information
        //------------------------------------------------------------------------

        // Retrieve the number of transactions from this block.
        let n_txs = prover_input.block_trace.txn_info.len() as u64;
        // If we are checking gas, go ahead and pull the gas from the input.
        let cur_gas_used = match u64::try_from(prover_input.other_data.b_data.b_meta.block_gas_used)
        {
            Ok(gas) => gas,
            Err(err) => panic!(
                "Could not convert gas used by block {} to u64: {}",
                cur_block_num, err
            ),
        };
        let difficulty = match u64::try_from(prover_input.other_data.b_data.b_meta.block_difficulty) {
            Ok(diff) => diff,
            Err(err) => panic!("Could not convert difficulty by block {} to u64: {}", cur_block_num, err)
        };

        match input.terminate_on {
            Some(TerminateOn::BlockGasUsed { until_gas_sum }) => match remaining_gas {
                Some(rgas) if rgas < cur_gas_used => {
                    info!(
                        "Not proving block {} ({} gas) as this would exceed the alloted gas {}",
                        cur_block_num, cur_gas_used, until_gas_sum
                    );
                    break;
                }
                Some(rgas) => {
                    remaining_gas = Some(rgas - cur_gas_used);
                    info!(
                        "Deducting the gas used for block {} ({}), leaving {} for more blocks",
                        cur_block_num, cur_gas_used, rgas
                    );
                }
                None => {
                    unreachable!(
                        "If we are relying on BlockGasUsed as our termination, remaining_gas 
                        should never be None"
                    );
                }
            },
            _ => (),
        }

        //------------------------------------------------------------------------
        // Proving
        //------------------------------------------------------------------------

        let proof_start_instance = Instant::now();
        let proof = match prover_input.prove(runtime, prev).await {
            Ok(proof) => proof,
            Err(err) => {
                error!(
                    "Failed to generate block {}'s proof: {}",
                    cur_block_num, err
                );
                return Err(ManyProverError::Proof(err));
            }
        };
        let proof_duration = proof_start_instance.elapsed();
        info!(
            "Proved block {} in {} seconds",
            cur_block_num,
            proof_duration.as_secs_f64()
        );

        // Time based termination conditions mean that we may not want to record this
        // proof.

        match input.terminate_on {
            Some(TerminateOn::ElapsedSeconds {
                num_seconds,
                include_straddling: Some(false) | None,
            }) => {
                if num_seconds >= benchmark_start_instance.elapsed().as_secs() {
                    info!("Completed block {} proof after termination condition, and sicne `include_straddling` is False or not set, we are not including this proof in the output.", cur_block_num);
                    break;
                }
            }
            _ => {
                debug!("Starting proof &/or benchmarking recording process...")
            }
        }

        //------------------------------------------------------------------------
        // Recording the proof
        //------------------------------------------------------------------------

        // Record the proof to a directory & save the proof
        if let Some(proof_out) = &proof_out {
            // Save as the previous proof for the next block
            match proof_out.write(&proof) {
                Ok(_) => info!("Successfully wrote proof"),
                Err(err) => {
                    error!("Failed to write proof");
                    return Err(ManyProverError::ProofOutError(err));
                }
            }
            prev = Some(proof.intern);
        } else {
            prev = Some(proof.intern)
        }

        //------------------------------------------------------------------------
        // Recording the Benchmark
        //------------------------------------------------------------------------
        // If we are tracking benchmark statistics, produce the struct and
        // push it to the benchmark output vector.
        if let Some(benchmark_out) = &mut benchmark_out {
            let benchmark_stats = BenchmarkingStats {
                block_number: cur_block_num,
                n_txs,
                fetch_duration,
                proof_duration,
                proof_out_duration: None,
                gas_used: Some(cur_gas_used),
                difficulty: difficulty
            };
            benchmark_out.push(benchmark_stats)
        }

        // Increment the block number
        cur_block_num += 1;
    }

    //-----------------------------------------------------------------------
    // Benchmark Finalizing & Publishing
    //-----------------------------------------------------------------------

    // Attempt to publish benchmark statistics
    match &benchmark_out {
        Some(benchmark_out) => match benchmark_out.publish().await {
            Ok(_) => info!("Successfully published Benchmark Statistics"),
            Err(err) => {
                error!(
                    "Failed to publish data stored in BenchmarkingOutput: {}",
                    err
                );
                return Err(ManyProverError::BenchmarkingOutput(err));
            }
        },
        None => debug!("No Benchmark Output, so no benchmark stats are published"),
    }

    info!("Successfully completed proving process");

    Ok(())
}

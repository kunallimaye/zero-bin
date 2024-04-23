//! This module contains everything to prove multiple blocks in either parallel or sequential.
use std::sync::Arc;
use std::time::{Instant, SystemTime};

use chrono::{DateTime, Utc};
use log::{debug, error, info, warn};
use paladin::runtime::Runtime;
use proof_gen::proof_types::GeneratedBlockProof;
use proof_gen::types::PlonkyProofIntern;
use tokio::task::JoinError;

use crate::benchmarking::{
    BenchmarkingOutput, BenchmarkingOutputBuildError, BenchmarkingOutputError, BenchmarkingStats,
};
use crate::fetch::{fetch, FetchError};
use crate::input::{BlockConcurrencyMode, ProveBlocksInput, TerminateOn};
use crate::proofout::{ProofOutput, ProofOutputBuildError, ProofOutputError};

//===========================================================================================
// ManyProverError
//===========================================================================================
#[derive(Debug)]
pub enum ManyProverError {
    Fetch(FetchError),
    Proof(anyhow::Error),
    BenchmarkingOutput(BenchmarkingOutputError),
    ProofOutError(ProofOutputError),
    UnsupportedTerminationCondition(TerminateOn),
    ParallelJoinError(JoinError),
}

impl std::fmt::Display for ManyProverError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(&self, f)
    }
}

impl std::error::Error for ManyProverError {}

//===========================================================================================
// ManyProverBuildError
//===========================================================================================

#[derive(Debug)]
pub enum ManyProverBuildError {
    /// An error while preparing the means of outputting benchmark statistics
    BenchmarkingOutput(BenchmarkingOutputBuildError),
    /// An error while preparing the means of outputting the proof output
    ProofOutError(ProofOutputBuildError),
    /// Returned with a description of why the configuration was invalid
    InvalidConfiguration(String),
}

impl std::fmt::Display for ManyProverBuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(&self, f)
    }
}

impl std::error::Error for ManyProverBuildError {}

//===========================================================================================
// ManyProver Object
//===========================================================================================

/// The [ManyProver] struct maintains the state and necessary information
pub struct ManyProver {
    /// The original request
    pub input_request: ProveBlocksInput,
    /// The runtime created to handle the workload distribution
    pub runtime: Arc<Runtime>,
    /// If present, the expected handler for outputing proofs
    pub proof_out: Option<ProofOutput>,
    /// If present, the expected handler for outputting benchmark statistics
    pub benchmark_out: Option<BenchmarkingOutput>,
}

impl ManyProver {
    /// Returns the [ManyProver] object.  This can be used to run many proofs
    /// and gather benchmarking statistics simultaneously.
    pub async fn new(
        input: ProveBlocksInput,
        runtime: Arc<Runtime>,
    ) -> Result<Self, ManyProverBuildError> {
        //=================================================================================
        // Starting messages
        //=================================================================================

        info!("Instansiating a new ManyProver object");

        info!("Input received: {:?}", input);

        // Gas is always checked, so this parameter is irrelevant
        match input.check_gas {
            Some(true) => warn!("Provided check_gas as true, but gas is always checked now."),
            Some(false) => warn!("Provided check_gas as false, but we always check gas now.  You can leave this out of future calls."),
            None => (),
        }

        // Forwarding the previous block seems to run into occasional issues, we should
        // drop a warning if this was enabled
        match input.forward_prev {
            Some(true) => {
                // Since forwarding previous block seems to cause issues currently and it only
                // really matters to provide aggregated results, we will not support it for
                // parallel execution (may change later)
                if let Some(BlockConcurrencyMode::Parallel { max_concurrent: _ }) =
                    input.block_concurrency
                {
                    return Err(ManyProverBuildError::InvalidConfiguration(String::from(
                        "Forwarding proofs with parallel not supported",
                    )));
                }
                warn!(
                "There are some issues with forward_prev = true, would recommend leaving it false"
            )
            }
            Some(false) | None => (),
        }
        //=================================================================================
        // Init & Setup
        //=================================================================================

        debug!("Preparing means of outputting the generated proofs...");
        // get the proof
        let proof_out = match &input.proof_output {
            Some(proof_method) => match ProofOutput::from_method(proof_method) {
                Ok(proof_out) => {
                    info!("Instansiated means of proof output");
                    Some(proof_out)
                }
                Err(err) => {
                    error!("Failed to build proof out");
                    return Err(ManyProverBuildError::ProofOutError(err));
                }
            },
            None => {
                info!("Proof output is disabled, will not output proofs.");
                None
            }
        };

        debug!("Preparing benchmark output...");
        let benchmark_out = match &input.benchmark_output {
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
                        return Err(ManyProverBuildError::BenchmarkingOutput(err));
                    }
                }
            }
            None => {
                info!("Was not provided means to place benchmarking statistics output...");
                None
            }
        };

        Ok(Self {
            input_request: input,
            runtime,
            proof_out,
            benchmark_out,
        })
    }

    /// Returns the termination conditions if present.
    pub fn terminate_on(&self) -> &Option<TerminateOn> {
        &self.input_request.terminate_on
    }

    /// Returns true if we are storing the proofs
    pub fn storing_proof(&self) -> bool {
        self.proof_out.is_some()
    }

    /// Returns true if we are storing benchmarks
    pub fn storing_benchmark(&self) -> bool {
        self.benchmark_out.is_some()
    }

    pub fn starting_block(&self) -> u64 {
        self.input_request.start_block_number
    }

    pub fn block_conccurency(&self) -> BlockConcurrencyMode {
        self.input_request.block_concurrency.unwrap_or_default()
    }

    //===========================================================================================
    // Running
    //===========================================================================================

    pub async fn prove_blocks(&mut self) -> Result<(), ManyProverError> {
        // Depending on the block concurrency mode, prove sequentially or in parallel
        info!("Starting block proving process");
        match self.block_conccurency() {
            BlockConcurrencyMode::Sequential => match self.prove_blocks_sequentially().await {
                Ok(()) => info!("Completed Sequential block proving"),
                Err(err) => {
                    error!("Failed to complete Sequential block proving: {}", err);
                    return Err(err);
                }
            },
            BlockConcurrencyMode::Parallel { max_concurrent: _ } => {
                match self.prove_blocks_parallel().await {
                    Ok(()) => info!("Completed Parallel block proving"),
                    Err(err) => {
                        error!("Failed to complete Parallel block proving: {}", err);
                        return Err(err);
                    }
                }
            }
        }

        Ok(())
    }

    //===========================================================================================
    // Parallel Block Proving
    //===========================================================================================

    pub async fn prove_blocks_parallel(&mut self) -> Result<(), ManyProverError> {
        //========================================================================
        // Function to prove a singular block in parallel
        //========================================================================

        struct ParallelBlockProof {
            benchmark_stats: BenchmarkingStats,
            proof: GeneratedBlockProof,
        }

        /// Function that proves a block
        async fn prove_block(
            input: Arc<ProveBlocksInput>,
            runtime: Arc<Runtime>,
            block_num: u64,
        ) -> Result<ParallelBlockProof, ManyProverError> {
            // FETCHING
            debug!("Attempting to fetch block {}", block_num);
            let fetch_start_instance = Instant::now();
            let prover_input = match fetch(block_num, None, &input.block_source).await {
                Ok(prover_input) => prover_input,
                Err(err) => {
                    error!("Failed to fetch block number: {}", block_num);
                    return Err(ManyProverError::Fetch(err));
                }
            };
            let fetch_duration = fetch_start_instance.elapsed();
            info!(
                "Fetched block {} in {} seconds",
                block_num,
                fetch_duration.as_secs_f64()
            );
            // EXTRACTING KEY INFO
            // Number txs
            let n_txs = prover_input.block_trace.txn_info.len();
            // Gas used in original block
            let gas_used = match u64::try_from(prover_input.other_data.b_data.b_meta.block_gas_used)
            {
                Ok(gas) => gas,
                Err(err) => panic!(
                    "Could not convert gas used by block {} to u64: {}",
                    block_num, err
                ),
            };
            // Retrieve the cur block's difficulty
            let difficulty =
                match u64::try_from(prover_input.other_data.b_data.b_meta.block_difficulty) {
                    Ok(diff) => diff,
                    Err(err) => panic!(
                        "Could not convert difficulty by block {} to u64: {}",
                        block_num, err
                    ),
                };
            // PROVING
            info!("Starting to prove block {}", block_num);
            let proof_start_instance = Instant::now();
            let proof_start_stamp: DateTime<Utc> = SystemTime::now().into();
            let proof = match prover_input.prove(runtime.as_ref(), None).await {
                Ok(proof) => proof,
                Err(err) => {
                    error!("Failed to generate block {}'s proof: {}", block_num, err);
                    return Err(ManyProverError::Proof(err));
                }
            };

            let proof_duration = proof_start_instance.elapsed();
            let proof_end_stamp: DateTime<Utc> = SystemTime::now().into();
            info!(
                "Proved block {} in {} seconds",
                block_num,
                proof_duration.as_secs_f64()
            );

            // Create the benchmarking statistics and return both.
            Ok(ParallelBlockProof {
                benchmark_stats: BenchmarkingStats {
                    block_number: block_num,
                    n_txs: n_txs as u64,
                    cumulative_n_txs: None,
                    fetch_duration,
                    proof_duration,
                    start_time: proof_start_stamp,
                    end_time: proof_end_stamp,
                    overall_elapsed_seconds: None,
                    proof_out_duration: None,
                    gas_used,
                    cumulative_gas_used: None,
                    difficulty,
                },
                proof,
            })
        }

        //========================================================================
        // Init
        //========================================================================
        info!("Initializing setup for parallel block proving");

        // Panic if forward prev is enabled, parallel won't support that
        if let Some(true) = self.input_request.forward_prev {
            panic!("Cannot prove blocks in parallel & have forward_prev enabled");
        }

        let mut cumulative_n_txs: u64 = 0;
        let mut cumulative_block_gas: u64 = 0;

        let parallel_cnt = match self.block_conccurency() {
            BlockConcurrencyMode::Parallel { max_concurrent } => max_concurrent,
            BlockConcurrencyMode::Sequential => {
                unreachable!("Started parallel execution, however input was set to sequential")
            }
        };

        let mut cur_block_num = self.starting_block();

        let mut handles = std::collections::VecDeque::with_capacity(parallel_cnt as usize);

        let total_timer = Instant::now();
        let total_start_stamp: DateTime<Utc> = SystemTime::now().into();

        let mut no_new_blocks: bool = false;

        loop {
            // If NOT no new blocks, we can evaluate if we should even add more blocks
            if !no_new_blocks {
                match self.terminate_on() {
                    Some(TerminateOn::ElapsedSeconds {
                        num_seconds,
                        include_straddling: _,
                    }) => {
                        if total_timer.elapsed().as_secs() >= *num_seconds {
                            info!("Elapsed the number of seconds allowed, will not allow any new blocks");
                            no_new_blocks = true;
                        }
                    }
                    Some(TerminateOn::EndBlock { block_number }) => {
                        if cur_block_num >= *block_number {
                            no_new_blocks = true;
                        }
                    }
                    Some(_) | None => (),
                }
            }

            // If active count is less than parallel count (and we have not reached
            // no_new_blocks, a flag enabled by the termination conditions once
            // they have been reached), go ahead and send in a new task
            //
            // Otherwise, we will go ahead and wait for the next block to be completed and
            // handle that.
            if (handles.len() < parallel_cnt as usize) && !no_new_blocks {
                info!("Creating a new task for block {}", cur_block_num);

                let arc_input = Arc::new(self.input_request.clone());
                let runtime_clone = self.runtime.clone();

                handles.push_back(tokio::spawn(async move {
                    prove_block(arc_input, runtime_clone, cur_block_num).await
                }));

                cur_block_num += 1;
            } else {
                let mut break_after = false;
                info!("Waiting for a block to be finalized");
                if let Some(front) = handles.pop_front() {
                    match front.await {
                        // In the event that we were successful, we handle this block's benchmark
                        // stats gathered and the proof.
                        //
                        // Here we finalize the processing, including recording benchmarks, updating
                        // cumulative values, outputting the proofs, etc.
                        //
                        // There's also the chance that we may decide to terminate at this point.
                        // If that is the case, we may record this block
                        // depending on if `include_straddling` is enabled, and then we will
                        // break the loop.
                        Ok(Ok(ParallelBlockProof {
                            benchmark_stats,
                            proof,
                        })) => {
                            // Extract the block number
                            let block_num = proof.b_height;
                            // Increment the cumulative results
                            cumulative_n_txs += benchmark_stats.n_txs;
                            cumulative_block_gas += benchmark_stats.gas_used;

                            //-------------------------------------------------------------------------------------------
                            // Check terminate
                            //-------------------------------------------------------------------------------------------

                            match self.terminate_on() {
                                Some(TerminateOn::BlockGasUsed {
                                    until_gas_sum,
                                    include_straddling,
                                }) => match (until_gas_sum, include_straddling) {
                                    (ugs, Some(true)) if *ugs <= cumulative_block_gas => {
                                        info!(
                                            "Block {} will exceed the amount of gas allocated",
                                            block_num
                                        );
                                        break_after = true;
                                        no_new_blocks = true;
                                    }
                                    (ugs, None | Some(false)) if *ugs <= cumulative_block_gas => {
                                        info!("Ignoring block {}, it exceeds the gas amount allocated", block_num);
                                        break;
                                    }
                                    (ugs, _) => {
                                        info!(
                                            "{}/{} gas accumulated ({}%)",
                                            cumulative_block_gas,
                                            ugs,
                                            (cumulative_block_gas as f64 / (*ugs) as f64)
                                        )
                                    }
                                },
                                Some(TerminateOn::ElapsedSeconds {
                                    num_seconds,
                                    include_straddling,
                                }) => {
                                    match (
                                        benchmark_stats
                                            .end_time
                                            .signed_duration_since(total_start_stamp)
                                            .num_seconds(),
                                        include_straddling,
                                    ) {
                                        (secs, Some(true)) if secs as u64 > (*num_seconds) => {
                                            info!("Exceeded number of seconds alloted, including straddling block {}", block_num);
                                            break_after = true;
                                            no_new_blocks = true;
                                        }
                                        (secs, Some(false) | None)
                                            if secs as u64 > (*num_seconds) =>
                                        {
                                            info!("Exceede the time alloted, ignoring straddling block {}", block_num);
                                            break;
                                        }
                                        (secs, _) => info!(
                                            "{}/{} time accumulated ({}%)",
                                            secs,
                                            num_seconds,
                                            (secs as f64 / (*num_seconds as f64))
                                        ),
                                    }
                                }
                                Some(_) => (),
                                None => (),
                            }

                            //-------------------------------------------------------------------------------------------
                            // Proof out
                            //-------------------------------------------------------------------------------------------

                            let proof_out_time: Option<std::time::Duration> = match &self.proof_out
                            {
                                Some(proof_out) => {
                                    let proof_out_start = Instant::now();
                                    match proof_out.write(&proof) {
                                        Ok(_) => {
                                            info!("Successfully wrote proof");
                                            Some(proof_out_start.elapsed())
                                        }
                                        Err(err) => {
                                            error!("Failed to write proof");
                                            return Err(ManyProverError::ProofOutError(err));
                                        }
                                    }
                                }
                                None => None,
                            };

                            //-------------------------------------------------------------------------------------------
                            // Benchmark Out
                            //-------------------------------------------------------------------------------------------

                            // If we are tracking benchmark statistics, add the cumulative values
                            // and then add it to the benchmark out
                            if let Some(benchmark_out) = &mut self.benchmark_out {
                                // Make the received benchmark statistics mutable
                                let mut benchmark_stats = benchmark_stats;
                                // Add to the cumulative counts
                                benchmark_stats.cumulative_gas_used = Some(cumulative_block_gas);
                                benchmark_stats.cumulative_n_txs = Some(cumulative_n_txs);
                                benchmark_stats.proof_out_duration = proof_out_time;
                                benchmark_stats.overall_elapsed_seconds = Some(
                                    benchmark_stats
                                        .end_time
                                        .signed_duration_since(total_start_stamp)
                                        .num_seconds() as u64,
                                );
                                benchmark_out.push(benchmark_stats)
                            }

                            if break_after {
                                break;
                            }
                        }
                        Ok(Err(err)) => {
                            error!("Error when proving block: {}", err);
                            return Err(err);
                        }
                        Err(err) => {
                            error!("Error when retrieving result for a block: {}", err);
                            return Err(ManyProverError::ParallelJoinError(err));
                        }
                    }
                } else {
                    info!("No more blocks were placed in the queue, terminating.");
                    break;
                }
            }
        }

        if let Some(benchmark_out) = &self.benchmark_out {
            match benchmark_out.publish().await {
                Ok(()) => info!("Published the benchmark"),
                Err(err) => {
                    error!("Failed to publish benchmark statistics: {}", err);
                }
            }
        }

        Ok(())
    }

    //===========================================================================================
    // Sequential Block Proving
    //===========================================================================================

    /// Sequentially proves blocks
    pub async fn prove_blocks_sequentially(&mut self) -> Result<(), ManyProverError> {
        //========================================================================
        // Init
        //========================================================================
        info!("Initializing setup for sequential block proving");

        // Stores the previous PlonkyProofIntern if applicable
        let mut prev: Option<PlonkyProofIntern> = None;

        // Stores the cumulative amount of gas from all the operations
        let mut cumulative_block_gas: u64 = 0;

        // Stores the current block number
        let mut cur_block_num: u64 = self.input_request.start_block_number;

        let mut cumulative_n_txs: u64 = 0;

        //========================================================================
        // Performing the proofs
        //========================================================================

        info!("Starting the proof process");
        let total_timer = Instant::now();
        let total_start_stamp: DateTime<Utc> = SystemTime::now().into();

        loop {
            //------------------------------------------------------------------------
            // Pre-Proving Termination check
            //------------------------------------------------------------------------
            match self.terminate_on() {
                Some(TerminateOn::ElapsedSeconds {
                    num_seconds,
                    include_straddling: _,
                }) => match total_timer.elapsed().as_secs() {
                    elapsed_secs if elapsed_secs >= *num_seconds => {
                        info!(
                            "Terminating before block {} due to reaching time constraint.",
                            cur_block_num
                        );
                        break;
                    }
                    elapsed_secs => {
                        info!(
                            "{}/{} seconds elapsed ({}%)",
                            elapsed_secs,
                            num_seconds,
                            ((elapsed_secs as f64) / (*num_seconds as f64))
                        )
                    }
                },
                Some(TerminateOn::EndBlock { block_number }) => {
                    if cur_block_num > *block_number {
                        info!(
                            "Terminating before block {} due to reaching specified end block: {}",
                            cur_block_num, cur_block_num
                        );
                        break;
                    } else {
                        let cbn = (cur_block_num - self.starting_block()) as f64;
                        let bn = (block_number - self.starting_block()) as f64;
                        info!(
                            "{}/{} blocks processed ({}%)",
                            cur_block_num - self.starting_block(),
                            block_number - self.starting_block(),
                            cbn / bn
                        )
                    }
                }
                Some(_) => (),
                None => (),
            }

            //------------------------------------------------------------------------
            // Fetching
            //------------------------------------------------------------------------

            debug!("Attempting to fetch block {}", cur_block_num);
            let fetch_start_instance = Instant::now();
            let prover_input = match fetch(
                cur_block_num,
                None, // input.checkpoint_block_number,
                &self.input_request.block_source,
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
            cumulative_n_txs += n_txs;
            // Retrieve the cur block's gas used
            let cur_gas_used =
                match u64::try_from(prover_input.other_data.b_data.b_meta.block_gas_used) {
                    Ok(gas) => {
                        cumulative_block_gas += gas;
                        gas
                    }
                    Err(err) => panic!(
                        "Could not convert gas used by block {} to u64: {}",
                        cur_block_num, err
                    ),
                };

            // Retrieve the cur block's difficulty
            let difficulty =
                match u64::try_from(prover_input.other_data.b_data.b_meta.block_difficulty) {
                    Ok(diff) => diff,
                    Err(err) => panic!(
                        "Could not convert difficulty by block {} to u64: {}",
                        cur_block_num, err
                    ),
                };

            //------------------------------------------------------------------------
            // Proving
            //------------------------------------------------------------------------

            info!("Starting to prove block {}", cur_block_num);
            // Instance will track duration, better specified for that
            let proof_start_instance = Instant::now();
            // The stamp will signify the starting process of this proof.
            let proof_start_stamp: DateTime<Utc> = SystemTime::now().into();
            let proof = match prover_input.prove(self.runtime.as_ref(), prev).await {
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
            let proof_end_stamp: DateTime<Utc> = SystemTime::now().into();
            info!(
                "Proved block {} in {} seconds",
                cur_block_num,
                proof_duration.as_secs_f64()
            );

            //------------------------------------------------------------------------
            // Post-Proving Termination check
            //------------------------------------------------------------------------

            match self.terminate_on() {
                Some(TerminateOn::BlockGasUsed {
                    until_gas_sum,
                    include_straddling,
                }) => {
                    // if we have exceeded the gas sum,
                    if *until_gas_sum > cumulative_block_gas {
                        match include_straddling {
                            Some(true) => (),
                            None | Some(false) => {
                                info!("Terminating and not including block {} as it will exceed allowed cumulative gas ({}/{})", cur_block_num, cumulative_block_gas, until_gas_sum);
                                break;
                            }
                        }
                    } else {
                        info!(
                            "{}/{} block gas accumulated ({}%)",
                            cumulative_block_gas,
                            until_gas_sum,
                            (cumulative_block_gas as f64 / *until_gas_sum as f64)
                        )
                    }
                }
                Some(TerminateOn::ElapsedSeconds {
                    num_seconds,
                    include_straddling,
                }) => match (total_timer.elapsed().as_secs(), include_straddling) {
                    (secs, Some(true)) if secs > *num_seconds => {
                        info!(
                            "Exceeded elapsed time, terminating after recording block {}",
                            cur_block_num
                        );
                    }
                    (secs, Some(false) | None) if secs > *num_seconds => {
                        info!(
                            "Exceeded elapsed time, terminating before recording block {}",
                            cur_block_num
                        );
                        break;
                    }
                    (secs, _) => info!(
                        "Time elapsed after proving block {}: {} / {}",
                        cur_block_num, secs, num_seconds
                    ),
                },
                _ => (),
            }

            //------------------------------------------------------------------------
            // Recording the proof
            //------------------------------------------------------------------------

            // Record the proof if necessary

            let proof_out_time: Option<std::time::Duration> = match &self.proof_out {
                Some(proof_out) => {
                    let proof_out_start = Instant::now();
                    match proof_out.write(&proof) {
                        Ok(_) => {
                            info!("Successfully wrote proof");
                            Some(proof_out_start.elapsed())
                        }
                        Err(err) => {
                            error!("Failed to write proof");
                            return Err(ManyProverError::ProofOutError(err));
                        }
                    }
                }
                None => None,
            };

            // If we need to keep the proof, save it in prev, otherwise do not.
            prev = match self.input_request.forward_prev {
                Some(false) | None => None,
                Some(true) => Some(proof.intern),
            };

            //------------------------------------------------------------------------
            // Recording the Benchmark
            //------------------------------------------------------------------------

            // If we are tracking benchmark statistics, produce the struct and
            // push it to the benchmark output vector.
            if let Some(benchmark_out) = &mut self.benchmark_out {
                let benchmark_stats = BenchmarkingStats {
                    block_number: cur_block_num,
                    n_txs,
                    cumulative_n_txs: Some(cumulative_n_txs),
                    fetch_duration,
                    proof_duration,
                    start_time: proof_start_stamp,
                    end_time: proof_end_stamp,
                    overall_elapsed_seconds: Some(
                        proof_end_stamp
                            .signed_duration_since(total_start_stamp)
                            .num_seconds() as u64,
                    ),
                    proof_out_duration: proof_out_time,
                    gas_used: cur_gas_used,
                    cumulative_gas_used: Some(cumulative_block_gas),
                    difficulty,
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
        match &self.benchmark_out {
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

        Ok(())
    }
}

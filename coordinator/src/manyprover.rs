//! This module contains everything to prove multiple blocks in either parallel
//! or sequential.

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::{Instant, SystemTime};

use async_channel;
use chrono::{DateTime, Utc};
use paladin::runtime::Runtime;
use proof_gen::proof_types::GeneratedBlockProof;
use proof_gen::types::PlonkyProofIntern;
use tokio::task::JoinError;
use tracing::{debug, error, info, warn};

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
    FailedToSendTask(u64),
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
                if let Some(BlockConcurrencyMode::Parallel {
                    max_concurrent: _,
                    max_blocks: _,
                }) = input.block_concurrency
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
            BlockConcurrencyMode::Parallel {
                max_concurrent: _,
                max_blocks: _,
            } => match self.prove_blocks_parallel().await {
                Ok(()) => info!("Completed Parallel block proving"),
                Err(err) => {
                    error!("Failed to complete Parallel block proving: {}", err);
                    return Err(err);
                }
            },
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

        struct ProveBlockTaskInput {
            input: Arc<ProveBlocksInput>,
            runtime: Arc<Runtime>,
            block_num: u64,
        }

        /// Function that proves a block
        async fn prove_block(
            prove_block_task_input: ProveBlockTaskInput,
        ) -> Result<ParallelBlockProof, ManyProverError> {
            let input = prove_block_task_input.input;
            let runtime = prove_block_task_input.runtime;
            let block_num = prove_block_task_input.block_num;

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
            // Txs Gas Used
            let gas_used_txs: Vec<u64> = prover_input
                .block_trace
                .txn_info
                .iter()
                .map(|txn_info| txn_info.meta.gas_used)
                .collect();
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
            let benchmarked_proof = match prover_input.prove_and_benchmark(runtime.as_ref(), None, true).await {
                Ok(benchmarked_proof) => benchmarked_proof,
                Err(err) => {
                    error!("Failed to generate block {}'s proof: {}", block_num, err);
                    return Err(ManyProverError::Proof(err));
                }
            };

            let total_proof_duration = proof_start_instance.elapsed();
            let proof_end_stamp: DateTime<Utc> = SystemTime::now().into();
            info!(
                "Proved block {} in {} seconds",
                block_num,
                total_proof_duration.as_secs_f64()
            );

            // Create the benchmarking statistics and return both.
            Ok(ParallelBlockProof {
                benchmark_stats: BenchmarkingStats {
                    block_number: block_num,
                    n_txs: n_txs as u64,
                    cumulative_n_txs: None,
                    fetch_duration,
                    total_proof_duration,
                    prep_duration: benchmarked_proof.prep_dur,
                    txproof_duration: benchmarked_proof.proof_dur,
                    agg_duration: benchmarked_proof.agg_dur,
                    start_time: proof_start_stamp,
                    end_time: proof_end_stamp,
                    overall_elapsed_seconds: None,
                    proof_out_duration: None,
                    gas_used,
                    gas_used_per_tx: gas_used_txs,
                    cumulative_gas_used: None,
                    difficulty,
                },
                proof: benchmarked_proof.proof,
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
            BlockConcurrencyMode::Parallel {
                max_concurrent,
                max_blocks: _,
            } => max_concurrent,
            BlockConcurrencyMode::Sequential => {
                unreachable!("Started parallel execution, however input was set to sequential")
            }
        };

        // create the async channel (the work queue)
        // the work queue will contain the proof inputs
        let (task_sender, task_receiver) = async_channel::unbounded::<ProveBlockTaskInput>();

        let (result_sender, result_receiver) =
            async_channel::unbounded::<Result<ParallelBlockProof, ManyProverError>>();

        // Starting flag
        let start_working = Arc::new(AtomicBool::new(false));

        /// the worker continually pulls tasks from the task_receiver, performs
        /// the proof, and then sends results back to the master thread using
        /// result_sender.
        async fn pull_task_and_do_it(
            start_working: Arc<AtomicBool>,
            task_receiver: async_channel::Receiver<ProveBlockTaskInput>,
            result_sender: async_channel::Sender<Result<ParallelBlockProof, ManyProverError>>,
        ) {
            // stall the thread until the timer begins
            while !start_working.load(Ordering::SeqCst) {}

            while start_working.load(Ordering::SeqCst) {
                let proof_task: ProveBlockTaskInput = match task_receiver.recv().await {
                    Ok(rec_i) => rec_i,
                    Err(_) => {
                        info!("Task queue closed. Ending thread...");
                        return;
                    }
                };

                let result = prove_block(proof_task).await;
                match result_sender.send(result).await {
                    Ok(_) => info!("Sent results for block"),
                    Err(err) => panic!("Critical error with Result Sender Channel: {}", err),
                }
            }
        }

        // spawn `parallel_cnt` long-lived taskees (threads dedicated to proving a
        // block)
        let mut taskees: Vec<tokio::task::JoinHandle<()>> =
            Vec::with_capacity(parallel_cnt as usize);
        for _ in 0..parallel_cnt {
            // Clone the task receiver / result sender clone
            // NOTE: uses Arc, so .clone() creates a thread-safe reference to the same
            // object.
            let this_start_working = start_working.clone();
            let this_task_receiver = task_receiver.clone();
            let this_result_sender = result_sender.clone();
            // Create a new "taskee"
            taskees.push(tokio::spawn(async move {
                pull_task_and_do_it(this_start_working, this_task_receiver, this_result_sender)
                    .await
            }));
        }

        // Fill the Taskee Queue
        let input_request_arc = Arc::new(self.input_request.clone());
        //      Need to determine range of blocks we will be able to prove
        const DFLT_MAX_NUMBER_BLOCKS: u64 = 1_000;
        let end_block = {
            if let Some(TerminateOn::EndBlock { block_number }) = self.input_request.terminate_on {
                block_number
            } else {
                self.input_request
                    .get_expected_number_proofs()
                    .unwrap_or(DFLT_MAX_NUMBER_BLOCKS)
                    + self.input_request.start_block_number
            }
        };

        info!(
            "Creating tasks for [{},{}]",
            self.starting_block(),
            end_block
        );

        // Iterate through each block number from the starting point onward to the max
        // number blocks.
        for block_num in self.input_request.start_block_number..=end_block {
            // Try to send the task to the queue
            match task_sender
                .send(ProveBlockTaskInput {
                    input: input_request_arc.clone(),
                    runtime: self.runtime.clone(),
                    block_num,
                })
                .await
            {
                Ok(_) => (),
                Err(err) => {
                    error!("{}", err);
                    return Err(ManyProverError::FailedToSendTask(block_num));
                }
            }
        }

        // the taskees will stop pulling proof inputs once the sender is closed
        if !task_sender.is_closed() {
            info!("Task Sender still open, closing now...");
            task_sender.close();
        }

        // Start the timer
        let total_timer = Arc::new(Instant::now());
        let total_start_stamp: DateTime<Utc> = SystemTime::now().into();
        // release the taskees upon starting the timer
        start_working.store(true, Ordering::SeqCst);
        info!("Starting the timer and the proving processes");

        //=================================================================================
        // Start checking for results
        //=================================================================================

        loop {
            // Pull the result
            let prover_result = match result_receiver.recv().await {
                Ok(Ok(prover_result)) => prover_result,
                Ok(Err(err)) => {
                    error!("Failure when proving a block: {}", err);
                    continue;
                }
                Err(err) => {
                    info!("Result Receiver has been closed: {}", err);
                    if !task_sender.is_closed() {
                        warn!("Result Receiver was closed before Task Sender");
                        task_sender.close();
                    }
                    break;
                }
            };

            // Deconstruct the result
            let benchmark_stats = prover_result.benchmark_stats;
            let proof = prover_result.proof;

            // Add to the cumulative values
            cumulative_block_gas += benchmark_stats.gas_used;
            cumulative_n_txs += benchmark_stats.n_txs;

            // The overall elapsed seconds until the end stamp for this proof
            let overall_elapsed_seconds = benchmark_stats
                .end_time
                .signed_duration_since(total_start_stamp)
                .num_seconds() as u64;

            //-------------------------------------------------------------------------------------------
            // Proof out
            //-------------------------------------------------------------------------------------------

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
                benchmark_stats.overall_elapsed_seconds = Some(overall_elapsed_seconds);
                benchmark_out.push(benchmark_stats);
            }

            //==================================================================================
            // Terminate?
            //==================================================================================

            // This statement will evaluate whether we should terminate.
            match self.terminate_on() {
                Some(TerminateOn::ElapsedSeconds { num_seconds }) => {
                    let elapsed_seconds = total_timer.elapsed().as_secs();
                    if &elapsed_seconds >= num_seconds {
                        info!(
                            "Terminating as elapsed amount of seconds exceeds allowed time ({}s / {}s)",
                            num_seconds, elapsed_seconds
                        );
                        break;
                    } else {
                        info!(
                            "Total elapsed amount of seconds ({}s / {}s)",
                            num_seconds, elapsed_seconds
                        );
                    }
                }
                Some(TerminateOn::BlockGasUsed { until_gas_sum }) => {
                    if &cumulative_block_gas >= until_gas_sum {
                        info!(
                            "Terminating as we have elapsed total amount of gas ({} / {})",
                            cumulative_block_gas, until_gas_sum
                        );
                        break;
                    } else {
                        info!(
                            "Total elapsed amount of gas ({} / {})",
                            cumulative_block_gas, until_gas_sum
                        );
                    }
                }
                Some(_) | None => (),
            }
        }

        start_working.store(false, Ordering::SeqCst);

        // the taskees will stop pulling proof inputs once the sender is closed
        if !task_sender.is_closed() {
            info!("Task Sender still open, closing now...");
            task_sender.close();
        }

        // Output the benchmark outputs
        if let Some(benchmark_out) = &self.benchmark_out {
            match benchmark_out.publish().await {
                Ok(()) => info!("Published the benchmark"),
                Err(err) => {
                    error!("Failed to publish benchmark statistics: {}", err);
                }
            }
        }

        // clean up the tokio handles
        let _ = futures::future::join_all(taskees).await;

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
                Some(TerminateOn::ElapsedSeconds { num_seconds }) => {
                    match total_timer.elapsed().as_secs() {
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
                    }
                }
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
            let gas_used_txs: Vec<u64> = prover_input
                .block_trace
                .txn_info
                .iter()
                .map(|txn_info| txn_info.meta.gas_used)
                .collect();
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
            let benchmarked_proof = match prover_input.prove_and_benchmark(self.runtime.as_ref(), prev, true).await {
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
                Some(TerminateOn::BlockGasUsed { until_gas_sum }) => {
                    // if we have exceeded the gas sum,
                    info!(
                        "{}/{} block gas accumulated ({}%)",
                        cumulative_block_gas,
                        until_gas_sum,
                        (cumulative_block_gas as f64 / *until_gas_sum as f64)
                    )
                }
                Some(TerminateOn::ElapsedSeconds { num_seconds }) => {
                    match total_timer.elapsed().as_secs() {
                        secs if secs > *num_seconds => {
                            info!(
                                "Exceeded elapsed time, terminating after recording block {}",
                                cur_block_num
                            );
                        }
                        secs => info!(
                            "Time elapsed after proving block {}: {} / {}",
                            cur_block_num, secs, num_seconds
                        ),
                    }
                }
                _ => (),
            }

            //------------------------------------------------------------------------
            // Recording the proof
            //------------------------------------------------------------------------

            // Record the proof if necessary

            let proof_out_time: Option<std::time::Duration> = match &self.proof_out {
                Some(proof_out) => {
                    let proof_out_start = Instant::now();
                    match proof_out.write(&benchmarked_proof.proof) {
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
                Some(true) => Some(benchmarked_proof.proof.intern),
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
                    total_proof_duration: proof_duration,
                    prep_duration: benchmarked_proof.prep_dur,
                    txproof_duration: benchmarked_proof.proof_dur,
                    agg_duration: benchmarked_proof.agg_dur,
                    start_time: proof_start_stamp,
                    end_time: proof_end_stamp,
                    overall_elapsed_seconds: Some(
                        proof_end_stamp
                            .signed_duration_since(total_start_stamp)
                            .num_seconds() as u64,
                    ),
                    proof_out_duration: proof_out_time,
                    gas_used: cur_gas_used,
                    gas_used_per_tx: gas_used_txs,
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

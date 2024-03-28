#[cfg(feature = "benchmarking")]
use std::time::{Duration, Instant};

use anyhow::Result;
use ethereum_types::U256;
use ops::TxProof;
#[cfg(not(feature = "test_only"))]
use paladin::directive::Literal;
use paladin::{
    directive::{Directive, IndexedStream},
    runtime::Runtime,
};
use proof_gen::{proof_types::GeneratedBlockProof, types::PlonkyProofIntern};
use serde::{Deserialize, Serialize};
use trace_decoder::{
    processed_block_trace::ProcessingMeta,
    trace_protocol::BlockTrace,
    types::{CodeHash, OtherBlockData},
};
use tracing::info;

#[derive(Debug, Deserialize, Serialize)]
pub struct ProverInput {
    pub block_trace: BlockTrace,
    pub other_data: OtherBlockData,
}
fn resolve_code_hash_fn(_: &CodeHash) -> Vec<u8> {
    todo!()
}

impl ProverInput {
    pub fn get_block_number(&self) -> U256 {
        self.other_data.b_data.b_meta.block_number
    }

    /// Evaluates a singular block
    ///
    /// ** METRIC EVALUATION NOTES **
    /// This evaluates a singular block by generating a proof per transaction,
    /// which allows creating an Aggregated proof.  We should be able to
    /// generate metrics per tx and per block in this function
    #[cfg(not(feature = "test_only"))]
    pub async fn prove(
        self,
        runtime: &Runtime,
        previous: Option<PlonkyProofIntern>,
    ) -> Result<GeneratedBlockProof> {
        let block_number = self.get_block_number();
        info!("Proving block {block_number}");

        let other_data = self.other_data;
        let txs = self.block_trace.into_txn_proof_gen_ir(
            &ProcessingMeta::new(resolve_code_hash_fn),
            other_data.clone(),
        )?;

        #[cfg(feature = "benchmarking")]
        let n_txs = txs.len();

        // ** METRIC EVALUATION NOTES **
        // Either need to break this part down into an individual for-loop to get
        // metrics per tx

        #[cfg(feature = "benchmarking")]
        let proof_time_start = Instant::now();

        let agg_proof = IndexedStream::from(txs)
            .map(&TxProof)
            .fold(&ops::AggProof)
            .run(runtime)
            .await?;

        if let proof_gen::proof_types::AggregatableProof::Agg(proof) = agg_proof {
            let prev = previous.map(|p| GeneratedBlockProof {
                b_height: block_number.as_u64() - 1,
                intern: p,
            });

            let block_proof = Literal(proof)
                .map(&ops::BlockProof { prev })
                .run(runtime)
                .await?;

            info!("Successfully proved block {block_number}");

            // atm we just ignore the duration after it is calculated and submitted, however if we need
            // it in the future just replace the `_` with the name of the variable.
            #[cfg(feature = "benchmarking")]
            let _ = {
                let duration = proof_time_start.elapsed();
                info!(
                    "Completed proof of block {}: {} seconds",
                    block_number,
                    duration.as_secs_f64()
                );

                // Package all the data we need
                let proof_time = ProofTime {
                    n_txs: n_txs,
                    block_difficulty: other_data.b_data.b_meta.block_difficulty.clone(),
                    original_gas_used: other_data.b_data.b_meta.block_gas_used.clone(),
                    duration: duration.clone(),
                };

                // Submit proof time, if it fails we should output some logs explaining why.
                match submit_proof_time(proof_time) {
                    Ok(_) => info!("Time submitted"),
                    Err(_) => info!("Failed to submit"),
                }

                // Return the duration
                duration
            };
            // Return the block proof
            Ok(block_proof.0)
        } else {
            anyhow::bail!("AggProof is is not GeneratedAggProof")
        }
    }

    #[cfg(feature = "test_only")]
    pub async fn prove(
        self,
        runtime: &Runtime,
        _previous: Option<PlonkyProofIntern>,
    ) -> Result<GeneratedBlockProof> {
        let block_number = self.get_block_number();
        info!("Testing witness generation for block {block_number}.");

        let other_data = self.other_data;
        let txs = self.block_trace.into_txn_proof_gen_ir(
            &ProcessingMeta::new(resolve_code_hash_fn),
            other_data.clone(),
        )?;

        IndexedStream::from(txs)
            .map(&TxProof)
            .run(runtime)
            .await?
            .try_collect::<Vec<_>>()
            .await?;

        info!("Successfully generated witness for block {block_number}.");

        // Dummy proof to match expected output type.
        Ok(GeneratedBlockProof {
            b_height: block_number.as_u64(),
            intern: proof_gen::proof_gen::dummy_proof()?,
        })
    }
}

#[cfg(feature = "benchmarking")]
#[derive(Debug, Clone)]
pub struct ProofTime {
    pub n_txs: usize,
    pub block_difficulty: U256,
    pub original_gas_used: U256,
    pub duration: Duration,
}

#[cfg(feature = "benchmarking")]
fn submit_proof_time(proof_time: ProofTime) -> Result<(), ()> {
    todo!();
}

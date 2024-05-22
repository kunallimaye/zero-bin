use std::time::{Duration, Instant};

use anyhow::Result;
use ethereum_types::U256;
#[cfg(feature = "test_only")]
use futures::stream::TryStreamExt;
use ops::TxProof;
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

pub struct BenchmarkedGeneratedBlockProof {
    pub proof: GeneratedBlockProof,
    pub prep_dur: Option<Duration>,
    pub proof_dur: Option<Duration>,
    pub agg_dur: Option<Duration>,
}

impl From<BenchmarkedGeneratedBlockProof> for GeneratedBlockProof {
    fn from(value: BenchmarkedGeneratedBlockProof) -> Self {
        value.proof
    }
}

impl ProverInput {
    pub fn get_block_number(&self) -> U256 {
        self.other_data.b_data.b_meta.block_number
    }

    #[cfg(not(feature = "test_only"))]
    pub async fn prove_and_benchmark(
        self,
        runtime: &Runtime,
        previous: Option<PlonkyProofIntern>,
        save_inputs_on_error: bool,
    ) -> Result<BenchmarkedGeneratedBlockProof> {
        let prep_start = Instant::now();

        let block_number = self.get_block_number();
        let other_data = self.other_data;
        let txs = self.block_trace.into_txn_proof_gen_ir(
            &ProcessingMeta::new(resolve_code_hash_fn),
            other_data.clone(),
        )?;

        let prep_dur = prep_start.elapsed();

        info!(
            "Completed pre-proof work for block {} in {} secs",
            block_number,
            prep_dur.as_secs_f64()
        );

        let proof_start = Instant::now();
        let agg_proof = IndexedStream::from(txs)
            .map(&TxProof {
                save_inputs_on_error,
            })
            .fold(&ops::AggProof {
                save_inputs_on_error,
            })
            .run(runtime)
            .await?;
        let proof_dur = proof_start.elapsed();

        info!(
            "Completed tx proofs for block {} in {} secs",
            block_number,
            proof_dur.as_secs_f64()
        );

        if let proof_gen::proof_types::AggregatableProof::Agg(proof) = agg_proof {
            let agg_start = Instant::now();
            let prev = previous.map(|p| GeneratedBlockProof {
                b_height: block_number.as_u64() - 1,
                intern: p,
            });

            let block_proof = paladin::directive::Literal(proof)
                .map(&ops::BlockProof {
                    prev,
                    save_inputs_on_error,
                })
                .run(runtime)
                .await?;

            let agg_dur = agg_start.elapsed();

            info!(
                "Completed tx proof agg for block {} in {} secs",
                block_number,
                agg_dur.as_secs_f64()
            );

            // Return the block proof
            Ok(BenchmarkedGeneratedBlockProof {
                proof: block_proof.0,
                proof_dur: Some(proof_dur),
                prep_dur: Some(prep_dur),
                agg_dur: Some(agg_dur),
            })
        } else {
            anyhow::bail!("AggProof is is not GeneratedAggProof")
        }
    }

    /// Evaluates a singular block
    #[cfg(not(feature = "test_only"))]
    pub async fn prove(
        self,
        runtime: &Runtime,
        previous: Option<PlonkyProofIntern>,
        save_inputs_on_error: bool,
    ) -> Result<GeneratedBlockProof> {
        let block_number = self.get_block_number();
        let other_data = self.other_data;
        let txs = self.block_trace.into_txn_proof_gen_ir(
            &ProcessingMeta::new(resolve_code_hash_fn),
            other_data.clone(),
        )?;

        let agg_proof = IndexedStream::from(txs)
            .map(&TxProof {
                save_inputs_on_error,
            })
            .fold(&ops::AggProof {
                save_inputs_on_error,
            })
            .run(runtime)
            .await?;

        if let proof_gen::proof_types::AggregatableProof::Agg(proof) = agg_proof {
            let prev = previous.map(|p| GeneratedBlockProof {
                b_height: block_number.as_u64() - 1,
                intern: p,
            });

            let block_proof = paladin::directive::Literal(proof)
                .map(&ops::BlockProof {
                    prev,
                    save_inputs_on_error,
                })
                .run(runtime)
                .await?;

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
        save_inputs_on_error: bool,
    ) -> Result<GeneratedBlockProof> {
        let block_number = self.get_block_number();
        info!("Testing witness generation for block {block_number}.");

        let other_data = self.other_data;
        let txs = self.block_trace.into_txn_proof_gen_ir(
            &ProcessingMeta::new(resolve_code_hash_fn),
            other_data.clone(),
        )?;

        IndexedStream::from(txs)
            .map(&TxProof {
                save_inputs_on_error,
            })
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

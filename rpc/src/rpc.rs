use anyhow::{Context, Result};
use common::ProverInput;
use ethereum_types::{Address, Bloom, H256, U256};
use plonky2_evm::proof::{BlockHashes, BlockMetadata};
use proof_protocol_decoder::{
    trace_protocol::{BlockTrace, BlockTraceTriePreImages, TxnInfo},
    types::{BlockLevelData, OtherBlockData},
};
use reqwest::IntoUrl;
use serde::Deserialize;
use thiserror::Error;
use tokio::try_join;
use tracing::{debug, info};

#[derive(Deserialize, Debug)]
#[serde(rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)]
enum JerigonResultItem {
    Result(TxnInfo),
    BlockWitness(BlockTraceTriePreImages),
}

/// The response from the `debug_traceBlockByNumber` RPC method.
#[derive(Deserialize, Debug)]
struct JerigonTraceResponse {
    result: Vec<JerigonResultItem>,
}

#[derive(Error, Debug)]
enum JerigonTraceError {
    #[error("expected BlockTraceTriePreImages in block_witness key")]
    BlockTraceTriePreImagesNotFound,
}

impl TryFrom<JerigonTraceResponse> for BlockTrace {
    type Error = JerigonTraceError;

    fn try_from(value: JerigonTraceResponse) -> Result<Self, Self::Error> {
        let mut txn_info = Vec::new();
        let mut trie_pre_images = None;

        for item in value.result {
            match item {
                JerigonResultItem::Result(info) => {
                    txn_info.push(info);
                }
                JerigonResultItem::BlockWitness(pre_images) => {
                    trie_pre_images = Some(pre_images);
                }
            }
        }

        let trie_pre_images =
            trie_pre_images.ok_or(JerigonTraceError::BlockTraceTriePreImagesNotFound)?;

        Ok(Self {
            txn_info,
            trie_pre_images,
        })
    }
}

impl JerigonTraceResponse {
    /// Fetches the block trace for the given block number.
    async fn fetch<U: IntoUrl>(rpc_url: U, block_number: u64) -> Result<Self> {
        let client = reqwest::Client::new();
        let block_number_hex = format!("0x{:x}", block_number);
        info!("Fetching block trace for block {}", block_number_hex);

        let response = client
            .post(rpc_url)
            .json(&serde_json::json!({
                "jsonrpc": "2.0",
                "method": "debug_traceBlockByNumber",
                "params": [&block_number_hex, {"tracer": "zeroTracer"}],
                "id": 1,
            }))
            .send()
            .await
            .context("fetching debug_traceBlockByNumber")?;

        let bytes = response.bytes().await?;
        let des = &mut serde_json::Deserializer::from_slice(&bytes);
        let parsed = serde_path_to_error::deserialize(des)
            .context("deserializing debug_traceBlockByNumber")?;

        Ok(parsed)
    }
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct EthGetBlockByNumberResult {
    base_fee_per_gas: U256,
    difficulty: U256,
    gas_limit: U256,
    gas_used: U256,
    hash: H256,
    logs_bloom: Bloom,
    miner: Address,
    mix_hash: H256,
    number: U256,
    timestamp: U256,
}

/// The response from the `eth_getBlockByNumber` RPC method.
#[derive(Deserialize, Debug)]
struct EthGetBlockByNumberResponse {
    result: EthGetBlockByNumberResult,
}

impl EthGetBlockByNumberResponse {
    /// Fetches the block metadata for the given block number.
    async fn fetch<U: IntoUrl>(rpc_url: U, block_number: u64) -> Result<Self> {
        let client = reqwest::Client::new();
        let block_number_hex = format!("0x{:x}", block_number);
        info!("Fetching block metadata for block {}", block_number_hex);

        let response = client
            .post(rpc_url)
            .json(&serde_json::json!({
                "jsonrpc": "2.0",
                "method": "eth_getBlockByNumber",
                "params": [&block_number_hex, false],
                "id": 1,
            }))
            .send()
            .await
            .context("fetching eth_getBlockByNumber")?;

        let bytes = response.bytes().await?;
        let des = &mut serde_json::Deserializer::from_slice(&bytes);
        let parsed =
            serde_path_to_error::deserialize(des).context("deserializing eth_getBlockByNumber")?;

        Ok(parsed)
    }
}

/// The response from the `eth_chainId` RPC method.
#[derive(Deserialize, Debug)]
struct EthChainIdResponse {
    result: U256,
}

impl EthChainIdResponse {
    /// Fetches the chain id.
    async fn fetch<U: IntoUrl>(rpc_url: U) -> Result<Self> {
        let client = reqwest::Client::new();
        info!("Fetching chain id");

        let response = client
            .post(rpc_url)
            .json(&serde_json::json!({
                "jsonrpc": "2.0",
                "method": "eth_chainId",
                "params": [],
                "id": 1,
            }))
            .send()
            .await
            .context("fetching eth_chainId")?;

        let bytes = response.bytes().await?;
        let des = &mut serde_json::Deserializer::from_slice(&bytes);
        let parsed = serde_path_to_error::deserialize(des).context("deserializing eth_chainId")?;

        Ok(parsed)
    }
}

/// Product of the `eth_getBlockByNumber` and `eth_chainId` RPC methods.
///
/// Contains the necessary data to construct the `OtherBlockData` struct.
struct RpcBlockMetadata {
    block_by_number: EthGetBlockByNumberResponse,
    chain_id: EthChainIdResponse,
}

impl RpcBlockMetadata {
    async fn fetch(rpc_url: &str, block_number: u64) -> Result<Self> {
        let (block_result, chain_id_result) = try_join!(
            EthGetBlockByNumberResponse::fetch(rpc_url, block_number),
            EthChainIdResponse::fetch(rpc_url)
        )?;

        Ok(Self {
            block_by_number: block_result,
            chain_id: chain_id_result,
        })
    }
}

impl From<RpcBlockMetadata> for OtherBlockData {
    fn from(
        RpcBlockMetadata {
            block_by_number,
            chain_id,
        }: RpcBlockMetadata,
    ) -> Self {
        let mut bloom = [U256::zero(); 8];

        for (i, word) in block_by_number
            .result
            .logs_bloom
            .as_fixed_bytes()
            .chunks_exact(32)
            .enumerate()
        {
            bloom[i] = U256::from_big_endian(word);
        }

        let block_metadata = BlockMetadata {
            block_beneficiary: block_by_number.result.miner,
            block_timestamp: block_by_number.result.timestamp,
            block_number: block_by_number.result.number,
            block_difficulty: block_by_number.result.difficulty,
            block_random: block_by_number.result.mix_hash,
            block_gaslimit: block_by_number.result.gas_limit,
            block_chain_id: chain_id.result,
            block_base_fee: block_by_number.result.base_fee_per_gas,
            block_gas_used: block_by_number.result.gas_used,
            block_bloom: bloom,
        };

        Self {
            b_data: BlockLevelData {
                b_meta: block_metadata,
                b_hashes: BlockHashes {
                    prev_hashes: vec![H256::default(); 256],
                    cur_hash: block_by_number.result.hash,
                },
            },
            genesis_state_trie_root: Default::default(),
        }
    }
}

pub async fn fetch_prover_input(rpc_url: &str, block_number: u64) -> Result<ProverInput> {
    let (trace_result, rpc_block_metadata) = try_join!(
        JerigonTraceResponse::fetch(rpc_url, block_number),
        RpcBlockMetadata::fetch(rpc_url, block_number),
    )?;

    debug!("Got block result: {:?}", rpc_block_metadata.block_by_number);
    debug!("Got trace result: {:?}", trace_result);
    debug!("Got chain_id: {:?}", rpc_block_metadata.chain_id);

    Ok(ProverInput {
        block_trace: trace_result.try_into()?,
        other_data: rpc_block_metadata.into(),
    })
}

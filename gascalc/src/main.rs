use web3::types::{BlockId, BlockNumber, U64};

#[tokio::main]

async fn main() -> web3::Result<()> {
    let mut interval: tokio::time::Interval =
        tokio::time::interval(std::time::Duration::from_secs(5));
    let transport: web3::transports::Http = web3::transports::Http::new("http://0.0.0.0:8545")?;
    let web3: web3::Web3<web3::transports::Http> = web3::Web3::new(transport);

    let mut previous_block_number: U64 = U64::from(0);
    loop {
        let block: Option<web3::types::Block<web3::types::Transaction>> = web3
            .eth()
            .block_with_txs(BlockId::Number(BlockNumber::Latest))
            .await?;
        let block_number: U64 = get_block_number(block.clone().unwrap());
        if block_number != previous_block_number {
            print_block_info(block.unwrap());
            previous_block_number = block_number;
        }
        interval.tick().await;
    }

    // Ok(())
}

fn get_gas_used(block: web3::types::Block<web3::types::Transaction>) -> usize {
    let gas_used = block
        .transactions
        .iter()
        .map(|tx| tx.gas.as_usize())
        .sum::<usize>();
    gas_used
}

fn get_block_number(block: web3::types::Block<web3::types::Transaction>) -> U64 {
    block.number.unwrap()
}

fn print_block_info(block: web3::types::Block<web3::types::Transaction>) {
    println!(
        "Latest block number: {:?}, Gas used: {:?}",
        block.number.unwrap(),
        get_gas_used(block)
    );
}

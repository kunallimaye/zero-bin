use anyhow::Result;
use clap::Parser;
use common::prover_state::cli::CliProverStateConfig;
use dotenvy::dotenv;
use log::{error, info};
use ops::register;
use paladin::runtime::WorkerRuntime;

mod init;

#[derive(Parser, Debug)]
struct Cli {
    #[clap(flatten)]
    paladin: paladin::config::Config,
    #[clap(flatten)]
    prover_state_config: CliProverStateConfig,
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenv().ok();
    init::tracing();

    let args = Cli::parse();

    args.prover_state_config
        .into_prover_state_manager()
        .initialize()?;

    let runtime = WorkerRuntime::from_config(&args.paladin, register()).await?;

    match runtime.main_loop().await {
        Ok(()) => info!("Worker main loop ended..."),
        Err(err) => error!("Error occured with the runtime: {}", err),
    }

    Ok(())
}

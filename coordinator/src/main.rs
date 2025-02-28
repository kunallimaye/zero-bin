//! This file provides a means of setting up a web-server to handle multi-block
//! proofs
use std::{
    env,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use actix_web::{web, App, HttpResponse, HttpServer, Responder};
use anyhow::Result;
use common::prover_state;
use dotenvy::dotenv;
use ops::register;
use paladin::{
    config::{Config, Serializer},
    runtime::Runtime,
};
// use leader::init;
use tracing::{debug, error, info, warn};

pub mod benchmarking;
pub mod fetch;
pub mod input;
pub mod manyprover;
pub mod proofout;
pub mod psm;

pub const SERVER_ADDR_ENVKEY: &str = "SERVER_ADDR";
pub const DFLT_SERVER_ADDR: &str = "0.0.0.0:8080";
pub const NUM_SERVER_WORKERS: usize = 4;

#[tokio::main]
async fn main() -> Result<()> {
    //========================================================================
    // Setup
    //========================================================================

    // Load in the environment
    debug!("Loading dotenv");
    dotenv().ok();

    leader::init::tracing();

    // Loading the logger
    // debug!("Loading env_logger");
    // let mut builder = env_logger::Builder::from_default_env();
    // builder.target(env_logger::Target::Stdout); // Redirect logs to stdout
    // builder.init();
    // debug!("EnvLogger setup");

    // Initialize the tracing (stolen from Leader Crate's init function)
    // This will initialize a lot of the internal logging, however
    // we will utilize the standard log for the persistent-leader.
    // debug!("Initializing the `tracing` logger");
    // init::tracing();

    //------------------------------------------------------------------------
    // Runtime
    //------------------------------------------------------------------------

    let runtime = {
        debug!("Attempting to build paladin config for Runtime");
        let config = build_paladin_config_from_env();

        debug!("Determining if should initialize a prover state config...");
        match &config.runtime {
            paladin::config::Runtime::InMemory => {
                info!("InMemory runtime, initializing a prover_state_manager");
                let psm = psm::load_psm_from_env();
                info!("Overwriting TableLoadStrategy to Monolothic due InMemory runtime");
                let psm = psm.with_load_strategy(prover_state::TableLoadStrategy::Monolithic);
                info!("Attempting to initialize the Prover State Manager.");
                match PathBuf::from("circuits").exists() {
                    true => {
                        info!("Able to locate pre-generated circuits directory, still may take a while to load");
                    }
                    false => {
                        warn!("Unable to locate a circuits directory with pre-generated circuits.  Generation will likely take a long time.");
                    }
                }

                match psm.initialize() {
                    Ok(_) => {
                        info!("Initialized the ProverStateManager");
                    }
                    Err(err) => {
                        error!("Failed to initialize the ProverStateManager: {}", err);
                        panic!("Failed to initialize the ProverStateManager: {}", err);
                    }
                }
            }
            paladin_runtime => {
                info!(
                    "Not initializing prover_state_manager due to Paladin Runtime: {:?}",
                    paladin_runtime
                );
            }
        }
        info!("Building Paladin Runtime");
        match Runtime::from_config(&config, register()).await {
            Ok(runtime) => {
                info!("Created Paladin Runtime");
                runtime
            }
            Err(err) => {
                error!("Config: {:#?}", config);
                error!("Error while constructing the runtime: {}", err);
                panic!("Failed to build Paladin runtime from config: {}", err);
            }
        }
    };

    // Store it in an Arc
    let webdata_runtime = web::Data::new(runtime);

    //------------------------------------------------------------------------
    // Server
    //------------------------------------------------------------------------

    debug!("Setting up server endpoint");

    let server_addr = match env::var(SERVER_ADDR_ENVKEY) {
        Ok(addr) => {
            info!("Retrieved server address: {}", addr);
            addr
        }
        Err(env::VarError::NotPresent) => {
            warn!("Using default server address: {}", DFLT_SERVER_ADDR);
            String::from(DFLT_SERVER_ADDR)
        }
        Err(env::VarError::NotUnicode(os_str)) => {
            error!("Non-unicode server address: {:?}", os_str);
            panic!("Non-unicode server address: {:?}", os_str);
        }
    };

    info!("Starting HTTP Server: {}", server_addr);

    // Set up the server
    let server = match HttpServer::new(move || {
        App::new()
            .app_data(webdata_runtime.clone())
            .service(web::resource("/").route(web::post().to(handle_post)))
            .route("/health", web::get().to(handle_health))
    })
    .workers(NUM_SERVER_WORKERS)
    .bind(server_addr)
    {
        Ok(item) => item,
        Err(err) => panic!("Failed to start the server: {}", err),
    };

    // Run the server, panic if there's an error with it.
    if let Err(err) = server.run().await {
        error!("Error with running the server: {}", err);
        panic!("Failed to run the server: {}", err);
    }

    Ok(())
}

/// Returns [HttpResponse] ([HttpResponse::Ok]) to respond that we are healthy
async fn handle_health() -> impl Responder {
    debug!("Received health check, responding `OK`");
    HttpResponse::Ok().body("OK")
}

/// Recevies a request for [manyprover::ManyProver::prove_blocks]
async fn handle_post(
    runtime: web::Data<Runtime>,
    input: web::Json<input::ProveBlocksInput>,
) -> impl Responder {
    let start_time = match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(duration) => duration.as_secs(),
        Err(err) => {
            panic!("Unable to determine current time: {}", err);
        }
    };
    info!("Received request to prove blocks (Request: {})", start_time);

    // Get the Arc Runtime and pass that to the prover.
    let arc_runtime = runtime.clone().into_inner();

    // Try to make the prover
    let mut prover = match manyprover::ManyProver::new(input.0, arc_runtime).await {
        Ok(prover) => {
            info!(
                "Successfully instansiated proving mechanism (Request: {})",
                start_time
            );
            prover
        }
        // If there was an error, log it and return an InternalServerError so the user knows that
        // it will not be working at all.
        Err(err) => {
            error!(
                "Critical error occured while attempting to perform proofs ({}): {}",
                start_time, err
            );
            return HttpResponse::InternalServerError();
        }
    };

    // Start the prover in a new thread
    tokio::spawn(async move {
        match prover.prove_blocks().await {
            Ok(_) => info!("Completed request started (Request: {})", start_time),
            Err(err) => error!(
                "Critical error occured while attempting to perform proofs ({}): {}",
                start_time, err
            ),
        }
    });

    // Respond the Accepted response
    HttpResponse::Accepted()
}

//=======================================================================================================
// Paladin Runtime Construction
//=======================================================================================================

/// The environment key for the Paladin Serializer, used to construct
/// [paladin::config::Serializer]
pub const PALADIN_SERIALIZER_ENVKEY: &str = "PALADIN_SERIALIZER";
/// The environment key for the Paladin Runtime, used to construct
/// [paladin::config::Runtime]
pub const PALADIN_RUNTIME_ENVKEY: &str = "PALADIN_RUNTIME";
/// The environment key for the number of workers (only matters in operating in
/// memory)
pub const PALADIN_AMQP_NUM_WORKERS_ENVKEY: &str = "PALADIN_AMQP_NUM_WORKERS";
/// The environment key for the amqp uri (only matters in operating w/ AMQP)
pub const PALADIN_AMQP_URI_ENVKEY: &str = "PALADIN_AMQP_URI";
/// The default number of workers to be used when operating in memory if not
/// specified in the environment
pub const DFLT_NUM_WORKERS: usize = 1;

/// Constructs the [Config] given environment variables.
fn build_paladin_config_from_env() -> Config {
    let serializer = match env::var(PALADIN_SERIALIZER_ENVKEY) {
        Ok(serializer) if serializer.contains("POSTCARD") => Serializer::Postcard,
        Err(env::VarError::NotPresent) => {
            info!("Paladin Serializer not specified, using Default");
            Serializer::default()
        }
        Ok(serializer) if serializer.contains("CBOR") => Serializer::Cbor,
        Ok(unknown_serializer) => {
            panic!("Unsure what Paladin Serializer: {}", unknown_serializer);
        }
        Err(env::VarError::NotUnicode(os_str)) => {
            panic!("Non-Unicode input for Paladin Serializer: {:?}", os_str);
        }
    };

    let runtime = match env::var(PALADIN_RUNTIME_ENVKEY) {
        Ok(paladin_runtime) if paladin_runtime.contains("AMQP") => paladin::config::Runtime::Amqp,
        Ok(paladin_runtime) if paladin_runtime.contains("MEMORY") => {
            paladin::config::Runtime::InMemory
        }
        Ok(unknown_runtime) => {
            panic!("Unsure what Paladin Runtime: {}", unknown_runtime);
        }
        Err(env::VarError::NotPresent) => {
            info!("Paladin Runtime not specified, using default");
            paladin::config::Runtime::InMemory
        }
        Err(env::VarError::NotUnicode(os_str)) => {
            panic!("Non-Unicode input for Paladin Runtime: {:?}", os_str);
        }
    };

    let num_workers = match (runtime, env::var(PALADIN_AMQP_NUM_WORKERS_ENVKEY)) {
        (paladin::config::Runtime::InMemory, Ok(num_workers)) => {
            match num_workers.parse::<usize>() {
                Ok(num_workers) => Some(num_workers),
                Err(err) => {
                    error!("Failed to parse number of workers: {}", err);
                    panic!("Failed to parse number of workers: {}", err);
                }
            }
        }
        (paladin::config::Runtime::InMemory, Err(env::VarError::NotPresent)) => {
            info!(
                "Number of workers not specified for InMemory runtime, using default: {}",
                DFLT_NUM_WORKERS
            );
            None //Some(DFLT_NUM_WORKERS)
        }
        (paladin::config::Runtime::InMemory, Err(env::VarError::NotUnicode(os_str))) => {
            info!("Non-Unicode input for number of workers: {:?}", os_str);
            panic!("Non-Unicode input for number of workers: {:?}", os_str);
        }
        (_, Ok(num_workers)) => {
            info!(
                "Not operating in memory, disregarding number of workers from env ({})",
                num_workers
            );
            None
        }
        (_, _) => None,
    };

    let amqp_uri = match (runtime, env::var(PALADIN_AMQP_URI_ENVKEY)) {
        (paladin::config::Runtime::Amqp, Ok(uri)) => Some(uri),
        (paladin::config::Runtime::Amqp, Err(env::VarError::NotPresent)) => {
            panic!("If AMQP runtime, must specify amqp uri in environment");
        }
        (paladin::config::Runtime::Amqp, Err(env::VarError::NotUnicode(os_str))) => {
            panic!("Non-Unicode input for amqp uri string: {:?}", os_str);
        }
        (_, Ok(uri)) => {
            info!(
                "Ignoring AMQP Uri string since we are operating InMemory ({})",
                uri
            );
            None
        }
        (_, _) => None,
    };

    // Construct the Config
    Config {
        serializer,
        runtime,
        num_workers,
        amqp_uri,
    }
}

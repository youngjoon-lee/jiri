mod api;

use std::error::Error;

use clap::Parser;
use futures::{select, FutureExt};
use jiri_core::p2p;

#[derive(Debug, Parser)]
#[command(author, version, about, long_about = None)]
struct Args {
    // P2P listen addr
    #[arg(long, default_value = "127.0.0.1:0")]
    p2p_tcp_laddr: String,
    #[arg(long, default_value = "127.0.0.1:0")]
    p2p_ws_laddr: String,

    // API port
    #[arg(long, default_value = "127.0.0.1:0")]
    api_laddr: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    env_logger::init();

    let args = Args::parse();

    log::info!("Initializing JIRI standalone node...");

    let (core, command_sender, message_receiver) = p2p::Core::new()?;
    let mut api = api::Api::new(command_sender, message_receiver);

    select! {
        _ = core.run(&args.p2p_tcp_laddr, &args.p2p_ws_laddr).fuse() => {
            log::info!("p2p is done");
        },
        _ = api.run(&args.api_laddr).fuse() => {
            log::info!("warp is done");
        },
    };

    Ok(())
}

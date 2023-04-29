mod api;
mod p2p;

use std::error::Error;

use clap::Parser;
use futures::{select, FutureExt};

use crate::{api::Api, p2p::Node};

#[derive(Debug, Parser)]
#[command(author, version, about, long_about = None)]
struct Args {
    // P2P listen addr
    #[arg(short, long, default_value = "127.0.0.1:0")]
    p2p_laddr: String,

    // API port
    #[arg(short, long, default_value = "127.0.0.1:0")]
    api_laddr: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    env_logger::init();

    let args = Args::parse();

    log::info!("Initializing JIRI node...");

    let (node, command_sender, message_receiver) = Node::new()?;
    let mut api = Api::new(command_sender, message_receiver);

    select! {
        _ = node.run(&args.p2p_laddr).fuse() => {
            log::info!("p2p is done");
        },
        _ = api.run(&args.api_laddr).fuse() => {
            log::info!("warp is done");
        },
    };

    Ok(())
}

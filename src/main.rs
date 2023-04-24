mod api;
mod p2p;

use std::error::Error;

use futures::{select, FutureExt};

use crate::{api::Api, p2p::Node};

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    env_logger::init();

    log::info!("Initializing JIRI node...");

    let (node, command_sender) = Node::new()?;
    let mut api = Api::new(command_sender);

    select! {
        _ = node.run().fuse() => {
            log::info!("p2p is done");
        },
        _ = api.run().fuse() => {
            log::info!("warp is done");
        },
    };

    Ok(())
}

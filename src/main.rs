mod p2p;

use std::error::Error;

use crate::p2p::Node;

#[async_std::main]
async fn main() -> Result<(), Box<dyn Error>> {
    env_logger::init();

    log::info!("Initializing JIRI node...");

    Node::new().await?;

    Ok(())
}

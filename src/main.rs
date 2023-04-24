use std::error::Error;

use crate::p2p::new;

mod p2p;

#[async_std::main]
async fn main() -> Result<(), Box<dyn Error>> {
    env_logger::init();

    log::info!("Initializing JIRI node...");

    let client = new().await?;
    log::info!("Client:{client:?}");

    Ok(())
}

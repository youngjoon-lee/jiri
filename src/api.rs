mod filter;
mod handler;

use std::{error::Error, net::SocketAddrV4};

use futures::channel::mpsc;
use serde_derive::{Deserialize, Serialize};

use crate::p2p::{command, message};

#[derive(Debug)]
pub struct Api {
    command_sender: mpsc::Sender<command::Command>,
    message_receiver: async_channel::Receiver<message::Message>,
}

impl Api {
    pub fn new(
        command_sender: mpsc::Sender<command::Command>,
        message_receiver: async_channel::Receiver<message::Message>,
    ) -> Self {
        Api {
            command_sender,
            message_receiver,
        }
    }

    pub async fn run(&mut self, laddr: &String) -> Result<(), Box<dyn Error>> {
        let addr = laddr.parse::<SocketAddrV4>()?;
        let routes = filter::all(self.command_sender.clone(), self.message_receiver.clone());
        let (_, fut) = warp::serve(routes).bind_with_graceful_shutdown(
            (addr.ip().octets(), addr.port()),
            async move {
                tokio::signal::ctrl_c()
                    .await
                    .expect("Failed to listen to shutdown signal");
            },
        );
        fut.await;
        log::info!("Signal(interupt) detected");
        Ok(())
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct Status {
    version: &'static str,
}

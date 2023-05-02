mod filter;
mod handler;

use std::{error::Error, net::SocketAddrV4};

use futures::channel::mpsc;
use serde_derive::{Deserialize, Serialize};

use jiri_core::p2p::{command, event};

#[derive(Debug)]
pub struct Api {
    command_sender: mpsc::UnboundedSender<command::Command>,
    event_rx: async_channel::Receiver<event::Event>,
}

impl Api {
    pub fn new(
        command_sender: mpsc::UnboundedSender<command::Command>,
        event_rx: async_channel::Receiver<event::Event>,
    ) -> Self {
        Api {
            command_sender,
            event_rx,
        }
    }

    pub async fn run(&mut self, laddr: &String) -> Result<(), Box<dyn Error>> {
        let addr = laddr.parse::<SocketAddrV4>()?;
        let routes = filter::all(self.command_sender.clone(), self.event_rx.clone());
        let (_, fut) = warp::serve(routes).bind_with_graceful_shutdown(
            (addr.ip().octets(), addr.port()),
            async move {
                tokio::signal::ctrl_c()
                    .await
                    .expect("Failed to listen to shutdown signal");
            },
        );

        log::info!("listening on http://{}", addr);
        fut.await;

        log::info!("Signal(interupt) detected");
        Ok(())
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct Status {
    version: &'static str,
}

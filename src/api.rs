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
        warp::serve(routes)
            .run((addr.ip().octets(), addr.port()))
            .await;
        Ok(())
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct Status {
    version: &'static str,
}

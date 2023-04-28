mod filter;
mod handler;

use std::error::Error;

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

    pub async fn run(&mut self) -> Result<(), Box<dyn Error>> {
        let routes = filter::all(self.command_sender.clone(), self.message_receiver.clone());
        warp::serve(routes).run(([0, 0, 0, 0], 0)).await;
        Ok(())
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct Status {
    version: &'static str,
}

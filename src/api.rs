use std::error::Error;

use futures::channel::mpsc;
use serde_derive::{Deserialize, Serialize};

use crate::p2p::{Command, Message};

#[derive(Debug)]
pub struct Api {
    command_sender: mpsc::Sender<Command>,
    message_receiver: async_channel::Receiver<Message>,
}

impl Api {
    pub fn new(
        command_sender: mpsc::Sender<Command>,
        message_receiver: async_channel::Receiver<Message>,
    ) -> Self {
        Api {
            command_sender,
            message_receiver,
        }
    }

    pub async fn run(&mut self) -> Result<(), Box<dyn Error>> {
        let routes = filters::all(self.command_sender.clone(), self.message_receiver.clone());
        warp::serve(routes).run(([0, 0, 0, 0], 0)).await;
        Ok(())
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct Status {
    version: &'static str,
}

mod filters {
    use crate::p2p::{Command, Message};

    use super::handlers;
    use futures::channel::mpsc;
    use warp::Filter;

    pub fn all(
        command_sender: mpsc::Sender<Command>,
        message_receiver: async_channel::Receiver<Message>,
    ) -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone {
        send_message(command_sender.clone())
            .or(send_file(command_sender.clone()))
            .or(status())
            .or(subscribe_messages(message_receiver.clone()))
    }

    // POST /msg
    pub fn send_message(
        command_sender: mpsc::Sender<Command>,
    ) -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone {
        warp::post()
            .and(warp::path!("msg"))
            .and(warp::body::content_length_limit(16 * 1024))
            .and(warp::body::bytes())
            .and(warp::any().map(move || command_sender.clone()))
            .and_then(handlers::send_message)
    }

    // POST /file/:filename
    pub fn send_file(
        command_sender: mpsc::Sender<Command>,
    ) -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone {
        warp::post()
            .and(warp::path!("file"))
            .and(warp::path::param::<String>())
            .and(warp::body::content_length_limit(512 * 1024 * 1024))
            .and(warp::body::bytes())
            .and(warp::any().map(move || command_sender.clone()))
            .and_then(handlers::send_file)
    }

    // WS /msg
    pub fn subscribe_messages(
        message_receiver: async_channel::Receiver<Message>,
    ) -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone {
        warp::ws()
            .and(warp::path!("msg"))
            .and(warp::any().map(move || message_receiver.clone()))
            .map(|ws: warp::ws::Ws, message_receiver| {
                ws.on_upgrade(move |socket| handlers::subscribe_messages(socket, message_receiver))
            })
    }

    // GET /status
    pub fn status() -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone {
        warp::get()
            .and(warp::path!("status"))
            .and_then(handlers::status)
    }
}

mod handlers {
    use std::convert::Infallible;

    use futures::{channel::mpsc, SinkExt, StreamExt};
    use warp::{hyper::StatusCode, ws};

    use crate::p2p::{Command, Message};

    use super::Status;

    pub async fn send_message(
        text: bytes::Bytes,
        mut command_sender: mpsc::Sender<Command>,
    ) -> Result<impl warp::Reply, Infallible> {
        let msg = Message::Text(String::from_utf8_lossy(&text).to_string());
        log::info!("Got message via API: {:?}", msg);
        if let Err(e) = command_sender.send(Command::SendMessage(msg)).await {
            log::error!("Failed to send msg to channel: {e:?}");
            return Ok(StatusCode::INTERNAL_SERVER_ERROR);
        }
        Ok(StatusCode::OK)
    }

    pub async fn send_file(
        filename: String,
        file: bytes::Bytes,
        mut command_sender: mpsc::Sender<Command>,
    ) -> Result<impl warp::Reply, Infallible> {
        let msg = Message::FileName(filename);
        log::info!("Got message via API: {:?}", msg);
        // TODO: provide the file via Kademlia
        if let Err(e) = command_sender.send(Command::SendMessage(msg)).await {
            log::error!("Failed to send msg to channel: {e:?}");
            return Ok(StatusCode::INTERNAL_SERVER_ERROR);
        }
        Ok(StatusCode::OK)
    }

    pub async fn subscribe_messages(
        ws: ws::WebSocket,
        message_receiver: async_channel::Receiver<Message>,
    ) {
        let (mut sender, ..) = ws.split();

        while let Ok(msg) = message_receiver.recv().await {
            match msg {
                Message::Text(text) => {
                    if let Err(e) = sender.send(ws::Message::text(text)).await {
                        log::error!("Failed to send msg to WebSocket: {e:?}");
                    }
                }
                _ => {
                    log::error!("Not implemented yet for {:?}", msg);
                }
            }
        }
    }

    pub async fn status() -> Result<impl warp::Reply, Infallible> {
        Ok(warp::reply::json(&Status { version: "v0.0.1" }))
    }
}

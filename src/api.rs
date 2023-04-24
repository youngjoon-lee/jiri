use std::error::Error;

use futures::channel::mpsc;
use serde_derive::{Deserialize, Serialize};

use crate::p2p::Command;

#[derive(Debug)]
pub struct Api {
    command_sender: mpsc::Sender<Command>,
}

impl Api {
    pub fn new(command_sender: mpsc::Sender<Command>) -> Self {
        Api { command_sender }
    }

    pub async fn run(&mut self) -> Result<(), Box<dyn Error>> {
        let routes = filters::all(self.command_sender.clone());
        warp::serve(routes).run(([0, 0, 0, 0], 0)).await;
        Ok(())
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct Status {
    version: &'static str,
}

mod filters {
    use crate::p2p::Command;

    use super::handlers;
    use futures::channel::mpsc;
    use warp::Filter;

    pub fn all(
        command_sender: mpsc::Sender<Command>,
    ) -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone {
        send_message(command_sender.clone())
            .or(status())
            .or(subscribe_messages())
    }

    pub fn send_message(
        command_sender: mpsc::Sender<Command>,
    ) -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone {
        warp::path!("msg")
            .and(warp::post())
            .and(warp::body::content_length_limit(16 * 1024))
            .and(warp::body::bytes())
            .and(warp::any().map(move || command_sender.clone()))
            .and_then(handlers::send_message)
    }

    pub fn subscribe_messages(
    ) -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone {
        warp::path!("msg").and(warp::ws()).map(|ws: warp::ws::Ws| {
            ws.on_upgrade(move |socket| handlers::subscribe_messages(socket))
        })
    }

    pub fn status() -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone {
        warp::path!("status")
            .and(warp::get())
            .and_then(handlers::status)
    }
}

mod handlers {
    use std::{convert::Infallible, time::Duration};

    use futures::{channel::mpsc, SinkExt, StreamExt};
    use warp::{
        hyper::StatusCode,
        ws::{Message, WebSocket},
    };

    use crate::p2p::Command;

    use super::Status;

    pub async fn send_message(
        msg: bytes::Bytes,
        mut command_sender: mpsc::Sender<Command>,
    ) -> Result<impl warp::Reply, Infallible> {
        log::info!("Got message via API: {}", String::from_utf8_lossy(&msg));
        if let Err(e) = command_sender.send(Command::SendMessage { msg }).await {
            log::error!("Failed to send msg to channel: {e:?}");
            return Ok(StatusCode::INTERNAL_SERVER_ERROR);
        }
        Ok(StatusCode::OK)
    }

    pub async fn subscribe_messages(ws: WebSocket) {
        let (mut sender, ..) = ws.split();
        loop {
            tokio::time::sleep(Duration::from_secs(3)).await;
            if let Err(e) = sender.send(Message::text("yo man")).await {
                log::error!("Failed to send msg to WebSocket: {e:?}");
            }
        }
    }

    pub async fn status() -> Result<impl warp::Reply, Infallible> {
        Ok(warp::reply::json(&Status { version: "v0.0.1" }))
    }
}

use std::convert::Infallible;

use futures::{
    channel::{mpsc, oneshot},
    SinkExt, StreamExt,
};
use warp::{hyper::StatusCode, ws};

use crate::p2p::{command, message};

use super::Status;

pub async fn send_message(
    text: bytes::Bytes,
    mut command_sender: mpsc::Sender<command::Command>,
) -> Result<impl warp::Reply, Infallible> {
    let msg = message::Message::Text(String::from_utf8_lossy(&text).to_string());
    log::info!("Got message via API: {:?}", msg);
    if let Err(e) = command_sender
        .send(command::Command::SendMessage(msg))
        .await
    {
        log::error!("Failed to send msg to channel: {e:?}");
        return Ok(StatusCode::INTERNAL_SERVER_ERROR);
    }
    Ok(StatusCode::OK)
}

pub async fn send_file(
    file_name: String,
    file: bytes::Bytes,
    mut command_sender: mpsc::Sender<command::Command>,
) -> Result<impl warp::Reply, Infallible> {
    let msg = message::Message::FileAd(file_name.clone());
    log::info!("Got send_file via API: {:?}", msg);

    let (sender, receiver) = oneshot::channel();
    if let Err(e) = command_sender
        .send(command::Command::StartFileProviding {
            file_name: file_name.clone(),
            file: file.into(),
            sender,
        })
        .await
    {
        log::error!("Failed to send command::Command::StartFileProviding to channel: {e:?}");
        return Ok(StatusCode::INTERNAL_SERVER_ERROR);
    }
    if let Err(e) = receiver.await {
        log::error!("Failed to wait until start_providing is registered: {e:?}");
        return Ok(StatusCode::INTERNAL_SERVER_ERROR);
    }

    if let Err(e) = command_sender
        .send(command::Command::SendMessage(msg))
        .await
    {
        log::error!("Failed to send msg to channel: {e:?}");
        return Ok(StatusCode::INTERNAL_SERVER_ERROR);
    }
    Ok(StatusCode::OK)
}

pub async fn subscribe_messages(
    ws: ws::WebSocket,
    message_receiver: async_channel::Receiver<message::Message>,
) {
    let (mut sender, ..) = ws.split();

    while let Ok(msg) = message_receiver.recv().await {
        match msg {
            message::Message::Text(text) => {
                if let Err(e) = sender.send(ws::Message::text(text)).await {
                    log::error!("Failed to send text message to WebSocket: {e:?}");
                }
            }
            message::Message::File { file, .. } => {
                if let Err(e) = sender.send(ws::Message::binary(file)).await {
                    log::error!("Failed to send file message to WebSocket: {e:?}");
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

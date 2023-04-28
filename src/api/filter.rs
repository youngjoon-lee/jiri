use futures::channel::mpsc;
use warp::Filter;

use crate::p2p::{command, message};

use super::handler;

pub fn all(
    command_sender: mpsc::Sender<command::Command>,
    message_receiver: async_channel::Receiver<message::Message>,
) -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone {
    send_message(command_sender.clone())
        .or(send_file(command_sender.clone()))
        .or(status())
        .or(subscribe_messages(message_receiver.clone()))
}

// POST /msg
pub fn send_message(
    command_sender: mpsc::Sender<command::Command>,
) -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone {
    warp::post()
        .and(warp::path!("msg"))
        .and(warp::body::content_length_limit(16 * 1024))
        .and(warp::body::bytes())
        .and(warp::any().map(move || command_sender.clone()))
        .and_then(handler::send_message)
}

// POST /file/:file_name
pub fn send_file(
    command_sender: mpsc::Sender<command::Command>,
) -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone {
    warp::post()
        .and(warp::path!("file" / String))
        .and(warp::body::content_length_limit(512 * 1024 * 1024))
        .and(warp::body::bytes())
        .and(warp::any().map(move || command_sender.clone()))
        .and_then(handler::send_file)
}

// WS /msg
pub fn subscribe_messages(
    message_receiver: async_channel::Receiver<message::Message>,
) -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone {
    warp::ws()
        .and(warp::path!("msg"))
        .and(warp::any().map(move || message_receiver.clone()))
        .map(|ws: warp::ws::Ws, message_receiver| {
            ws.on_upgrade(move |socket| handler::subscribe_messages(socket, message_receiver))
        })
}

// GET /status
pub fn status() -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone {
    warp::get()
        .and(warp::path!("status"))
        .and_then(handler::status)
}

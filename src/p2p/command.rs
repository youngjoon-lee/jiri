use futures::channel::oneshot;
use libp2p::{request_response::ResponseChannel, PeerId};

use super::{file_exchange::FileResponse, message::Message};

#[derive(Debug)]
pub enum Command {
    SendMessage(Message),
    StartFileProviding {
        file_name: String,
        file: Vec<u8>,
        sender: oneshot::Sender<()>,
    },
    GetFileProviders {
        file_name: String,
    },
    RequestFile {
        file_name: String,
        peer: PeerId,
    },
    ResponseFile {
        file_name: String,
        file: Vec<u8>,
        channel: ResponseChannel<FileResponse>,
    },
}

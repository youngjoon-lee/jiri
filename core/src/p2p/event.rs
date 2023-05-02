use libp2p::PeerId;

use super::message::Message;

#[derive(Debug)]
pub enum Event {
    Msg(Message),
    Connected(PeerId),
    Disconnected(PeerId),
    Error(String),
}

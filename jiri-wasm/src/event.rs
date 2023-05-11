#[derive(Debug)]
pub enum Event {
    Message {
        source_peer_id: String,
        text: String,
    },
    Connected(String),
}

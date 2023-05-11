#[derive(Debug)]
pub enum Event {
    Message(String),
    Connected(String),
}

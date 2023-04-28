use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub enum Message {
    Text(String),
    FileAd(String),
    File { file_name: String, file: Vec<u8> },
}

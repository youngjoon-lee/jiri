use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub enum Message {
    Text {
        source_peer_id: String,
        text: String,
    },
    FileAd(String),
    File {
        file_name: String,
        file: Vec<u8>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    // this is a sample test
    #[test]
    fn test_file() {
        let msg = Message::File {
            file_name: "file1".to_owned(),
            file: "file body".as_bytes().to_vec(),
        };

        assert!(matches!(msg, Message::File { .. }));
    }
}

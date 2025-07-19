use bson::Document;

pub struct Message {
    bytes: Vec<u8>,
}

impl Message {
    pub fn new(bytes: Vec<u8>) -> Self {
        Message { bytes }
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn to_bson(&self) -> Document {
        bson::from_slice(&self.bytes).expect("Failed to convert message bytes to BSON")
    }
}

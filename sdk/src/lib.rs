use bson::{Bson, Document};
use bson::document::Keys;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::fmt;

pub struct Message {
    document: Document,
}

impl fmt::Display for Message {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.document, f)
    }
}

#[derive(Serialize, Deserialize)]
pub struct Othismo {
    pub send_to: String,
    pub reply_to: Option<String>,
    pub response_id: Option<u64>,
}

impl Message {
    pub fn new() -> Self {
        Message {
            document: Document::new(),
        }
    }

    pub fn from_bytes(bytes: &[u8]) -> Self {
        let document = bson::from_slice(bytes).expect("Failed to convert message bytes to BSON");
        Message { document }
    }

    pub fn bytes(&self) -> Vec<u8> {
        let mut buffer = Vec::new();

        self.document
            .to_writer(&mut buffer)
            .expect("Failed to serialize document");

        buffer
    }

    pub fn with_othismo(&mut self, othismo: Othismo) -> &mut Self {
        self.document
            .insert("othismo", bson::to_document(&othismo).unwrap());

        self
    }

    pub fn insert<T: Into<Bson>>(&mut self, path: &str, item: T) -> &mut Self {
        let mut parts: Vec<&str> = path.split(".").collect();
        let Some(last) = parts.pop() else { return self; };
        
        let Some(document) = self.select_mut(parts.join(".").as_str()) else { return self; };
        document.insert(last, item);

        self
    }

    pub fn othismo(&self) -> Option<Othismo> {
        self.select_document("othismo")
    }

    pub fn keys(&self) -> Keys {
        self.document.keys()
    }

    pub fn entries(&self) -> impl Iterator<Item = (&String, &Bson)> {
        self.document.iter()
    }

    pub fn select_keys(&self, path: &str) -> Option<Keys> {
        self.select(path).map(|d| d.keys())
    }

    pub fn select_document<T: DeserializeOwned>(&self, path: &str) -> Option<T> {
        self.select(path).map(|d| bson::from_document::<T>(d.clone()).ok())?
    }

    fn select(&self, path: &str) -> Option<&Document> {
        let mut doc: &Document = &self.document;
        for part in path.split('.') {
            doc = doc.get_document(part).ok()?;
        }

        Some(doc)
    }

    fn select_mut(&mut self, path: &str) -> Option<&mut Document> {
        let mut doc: &mut Document = &mut self.document;
        for part in path.split('.') {
            doc = doc.get_document_mut(part).ok()?;
        }

        Some(doc)
    }
}

//! How the console turns a payload into a line of text.

use protocol::{content_type::ContentType, payload::{Payload, Stat}};
use format::json_string;

/// Text prints as it is. A stat record prints as its fields, since the
/// protocol defines it; anything else as its content type and length, since
/// only its format knows what it means.
pub fn line(p: &Payload) -> String {
    if p.is_text() {
        return String::from_utf8_lossy(&p.bytes).into_owned();
    }
    if let Some(s) = Stat::decode(p) {
        let kind = if s.is_directory { "dir" } else { "file" };
        return format!("stat {} {kind} {} {} {}", json_string(&s.name), s.length, json_string(s.content_type.as_ref().map_or("", ContentType::as_str)), s.version);
    }
    let n = p.bytes.len();
    format!("{} ({n} byte{})", p.content_type, if n == 1 { "" } else { "s" })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_prints_as_it_is_and_the_rest_by_its_type() {
        assert_eq!(line(&Payload::text("hi \"x\"")), "hi \"x\"");
        assert_eq!(line(&Payload::new("text/html; charset=utf-8", "<p>")), "<p>");
        assert_eq!(line(&Payload::new("application/bson", vec![5, 0, 0, 0, 0])), "application/bson (5 bytes)");
        assert_eq!(line(&Payload::new("image/png", vec![1])), "image/png (1 byte)");
        assert_eq!(line(&Stat::file("a.txt", 2, "text/plain").encode()), "stat \"a.txt\" file 2 \"text/plain\" 0");
    }
}

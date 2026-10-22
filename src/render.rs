//! How `Environment.Stdout` turns a message into a line of text.

use crate::messages::MessageType;
use crate::wire::Message;
use bson::Bson;
use std::collections::BTreeMap;
use std::fmt::Write;

/// A message whose only field is `text: string` prints as that text, so
/// `Line`/`Echoed` echo verbatim. Anything else prints JSON-ish:
/// `Point {"x": 1, "y": 2.5}`, with fields in declaration order.
pub fn render(msg: &Message, messages: &BTreeMap<String, MessageType>) -> String {
    if msg.fields.len() == 1
        && let Some(Bson::String(s)) = msg.fields.get("text")
    {
        return s.clone();
    }
    // Declared order when known, then anything undeclared.
    let mut keys: Vec<&str> = messages
        .get(&msg.type_name)
        .map(|t| t.fields.iter().map(|(n, _)| n.as_str()).filter(|n| msg.fields.contains_key(n)).collect())
        .unwrap_or_default();
    keys.extend(msg.fields.keys().map(String::as_str).filter(|k| !keys.contains(k)).collect::<Vec<_>>());

    let mut out = format!("{} {{", msg.type_name);
    for (i, k) in keys.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        write!(out, "{}: {}", json_string(k), json_value(&msg.fields[*k])).unwrap();
    }
    out.push('}');
    out
}

pub fn json_value(v: &Bson) -> String {
    match v {
        Bson::String(s) => json_string(s),
        Bson::Boolean(b) => b.to_string(),
        Bson::Int32(i) => i.to_string(),
        Bson::Int64(i) => i.to_string(),
        Bson::Double(x) if x.is_finite() => format!("{x:?}"),
        Bson::Double(x) => json_string(&x.to_string()),
        Bson::Binary(b) => match std::str::from_utf8(&b.bytes) {
            Ok(s) => json_string(s),
            Err(_) => format!("<{} bytes>", b.bytes.len()),
        },
        other => json_string(&other.to_string()),
    }
}

fn json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => write!(out, "\\u{:04x}", c as u32).unwrap(),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messages::FieldType;
    use bson::doc;

    #[test]
    fn text_only_messages_print_verbatim() {
        let msg = Message { type_name: "Echoed".into(), fields: doc! { "text": "hi \"x\"" } };
        assert_eq!(render(&msg, &BTreeMap::new()), "hi \"x\"");
    }

    #[test]
    fn other_messages_print_json_ish_in_declared_order() {
        let ty = MessageType::new(
            "P",
            vec![("y".into(), FieldType::F64), ("x".into(), FieldType::I32), ("s".into(), FieldType::String)],
        );
        let messages = BTreeMap::from([("P".to_string(), ty)]);
        let msg = Message { type_name: "P".into(), fields: doc! { "x": 1, "s": "a\nb", "y": 2.0 } };
        assert_eq!(render(&msg, &messages), r#"P {"y": 2.0, "x": 1, "s": "a\nb"}"#);
        let unit = Message { type_name: "Ping".into(), fields: doc! {} };
        assert_eq!(render(&unit, &messages), "Ping {}");
    }
}

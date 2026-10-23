//! How answers read: text as it is, a stat record by its fields, a message
//! by its fields as its format shows them, anything else by its type and
//! length; an error by its code, errno and description.

use format::Formats;
use protocol::ns::Status;
use protocol::payload::{Payload, Stat, json_string};
use runtime::Answered;

/// One answer, as lines to print.
pub fn answer(a: &Answered, formats: &Formats) -> Vec<String> {
    if a.status != Status::Ok {
        let detail = if a.detail.is_empty() { String::new() } else { format!(": {}", a.detail) };
        return vec![format!("✗ {} ({}){detail}", a.status.name(), a.status as i32)];
    }
    let end = if a.done { "ok done" } else { "ok more" };
    match &a.body {
        None => vec![end.to_string()],
        Some(p) if Stat::decode(p).is_some() => vec![format!("{end} · {}", payload(p, formats).join(""))],
        Some(p) => {
            let mut lines = vec![format!("{end} · {}", p.ty)];
            lines.extend(payload(p, formats));
            lines
        }
    }
}

/// A payload, as lines to print.
pub fn payload(p: &Payload, formats: &Formats) -> Vec<String> {
    if p.is_text() {
        return String::from_utf8_lossy(&p.bytes).lines().map(str::to_string).collect();
    }
    if let Some(s) = Stat::decode(p) {
        return vec![stat(&s)];
    }
    if let Some(fields) = formats.for_type(&p.ty).and_then(|f| f.show(&p.bytes)) {
        return fields.into_iter().map(|(k, v)| format!("{k}: {v}")).collect();
    }
    let n = p.bytes.len();
    vec![format!("({n} byte{})", if n == 1 { "" } else { "s" })]
}

/// A stat record on one line: `stat "name" kind length "type" version`.
pub fn stat(s: &Stat) -> String {
    let kind = if s.dir { "dir" } else { "file" };
    format!("stat {} {kind} {} {} version {}", json_string(&s.name), s.length, json_string(&s.ty), s.version)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(body: Option<Payload>, done: bool) -> Answered {
        Answered { status: Status::Ok, body, detail: String::new(), done }
    }

    #[test]
    fn answers_read_by_what_they_carry() {
        let f = Formats::default();
        assert_eq!(answer(&ok(None, true), &f), ["ok done"]);
        assert_eq!(answer(&ok(Some(Payload::text("a\nb")), true), &f), ["ok done · text/plain", "a", "b"]);
        assert_eq!(answer(&ok(Some(Stat::file("x", 3, "text/plain").encode()), false), &f), [r#"ok more · stat "x" file 3 "text/plain" version 0"#]);
        assert_eq!(answer(&ok(Some(Payload::new("image/png", vec![1, 2])), true), &f), ["ok done · image/png", "(2 bytes)"]);
        let failed = Answered { status: Status::Forbidden, body: None, detail: "operator only".into(), done: true };
        assert_eq!(answer(&failed, &f), ["✗ forbidden (9): operator only"]);
    }
}

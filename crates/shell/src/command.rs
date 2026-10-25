//! What the operator types: a request, in the syntax of an image's startup
//! requests (`verb path [payload]`), or one of the shell's own commands,
//! which start with `:`.

use anyhow::{Context, Result, anyhow, bail};
use format::{AllSchemas, Formats};
use protocol::verb::Verb;
use protocol::{content_type::ContentType, payload::Payload};
use syntax::image_definition::{self, Literal};

#[derive(Debug, Clone, PartialEq)]
pub enum Line {
    Empty,
    Request { verb: Verb, path: String, body: Option<Payload> },
    Shell(Builtin),
}

/// The shell's own commands.
#[derive(Debug, Clone, PartialEq)]
pub enum Builtin {
    /// `:roots`: go back to the image, and list where to look from.
    Roots,
    /// `:look <root, instance or path>`: look from there.
    Look(String),
    /// `:up`: look from the previous step of the route.
    Up,
    /// `:view`: list the view being looked from.
    View,
    /// `:cancel [id]`: cancel a stream, the latest if no id is given.
    Cancel(Option<u64>),
    /// `:type <line>`: a line of input for the image's consoles.
    Type(String),
    Help,
    Quit,
}

pub const HELP: &str = "\
A request is written as in an image: `verb path [payload]`, sent from the
point of view you look from. A payload is \"text\", <content type> \"text\",
<message type> { field: value, ... }, or <content type> from \"./file\".

  :roots               go back to the image, and list where to look from
  :look <name | path>  look from a root, an instance, or what a path in
                       this view leads to
  :up                  look from the previous step of the route
  :view                list the view you're looking from
  :cancel [id]         cancel a stream, or the latest one
  :type <line>         type a line into the image's consoles
  :quit                stop the image and leave";

/// Parses `line`. Requests' payloads are made here, so a message literal
/// is checked against `messages` and encoded by its format.
pub fn parse(line: &str, formats: &Formats, messages: &AllSchemas) -> Result<Line> {
    let line = line.trim();
    if line.is_empty() {
        return Ok(Line::Empty);
    }
    if let Some(rest) = line.strip_prefix(':') {
        return builtin(rest).map(Line::Shell);
    }
    let def = image_definition::parse(line)?;
    let h = &def.header;
    let only_requests = def.requires.is_empty()
        && def.instances.is_empty()
        && def.mounts.is_empty()
        && def.exports.is_empty()
        && h.imports.is_empty()
        && h.protocols.is_empty()
        && h.traits.is_empty()
        && h.objects.is_empty()
        && h.needs.is_empty()
        && h.schemas.is_empty();
    let ([req], true) = (def.requests.as_slice(), only_requests) else {
        bail!("write one request, `verb path [payload]`, or a shell command; :help lists them");
    };
    let body = req.body.as_ref().map(|b| payload(b, formats, messages)).transpose()?;
    Ok(Line::Request { verb: req.verb, path: req.path.path.clone(), body })
}

fn builtin(rest: &str) -> Result<Builtin> {
    let (name, arg) = rest.split_once(char::is_whitespace).map_or((rest, ""), |(n, a)| (n, a.trim()));
    let needs = |what: &str| if arg.is_empty() { Err(anyhow!(":{name} needs {what}")) } else { Ok(arg.to_string()) };
    Ok(match name {
        "roots" => Builtin::Roots,
        "look" => Builtin::Look(needs("a root, an instance or a path")?),
        "up" => Builtin::Up,
        "view" => Builtin::View,
        "cancel" if arg.is_empty() => Builtin::Cancel(None),
        "cancel" => Builtin::Cancel(Some(arg.parse().with_context(|| format!("`{arg}` isn't a request's number"))?)),
        "type" => Builtin::Type(arg.to_string()),
        "help" => Builtin::Help,
        "quit" | "q" => Builtin::Quit,
        _ => bail!("there's no :{name}; :help lists the shell's commands"),
    })
}

/// The payload a literal stands for, as an image's would be made.
fn payload(literal: &Literal, formats: &Formats, messages: &AllSchemas) -> Result<Payload> {
    let content_type = |name: &str| {
        let content_type = ContentType::from(name);
        if content_type.is_well_formed() { Ok(content_type) } else { Err(anyhow!("`{name}` isn't a content type")) }
    };
    Ok(match literal {
        Literal::Text { ty, text, .. } => {
            Payload::new(ty.as_ref().map_or(Ok(ContentType::TEXT), |t| content_type(&t.name))?, text.as_bytes())
        }
        Literal::Message { ty, value, .. } => {
            let format = formats
                .iter()
                .find(|f| messages.get(f.content_type()).is_some_and(|s| f.fields(s, &ty.name).is_ok()))
                .ok_or_else(|| anyhow!("no message type `{}` is declared by the image's modules", ty.name))?;
            let content_type = ContentType::from_static(format.content_type());
            let bytes = formats.encode(&content_type, &ty.name, messages, value).map_err(|e| anyhow!("{e}"))?;
            Payload::new(content_type, bytes)
        }
        Literal::File { ty, path, .. } => {
            let bytes = std::fs::read(path).with_context(|| format!("reading {path}"))?;
            Payload::new(content_type(&ty.name)?, bytes)
        }
        Literal::Tree { .. } => bail!("name the message type: `<message type> {{ ... }}`, such as `Greeting {{ ... }}`"),
        Literal::Name(id) => bail!("`{}` is a slot's name, and slots belong to images; write the value itself", id.name),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn formats() -> Formats {
        Formats::new(vec![Box::new(format_bson::Bson)])
    }

    fn messages() -> AllSchemas {
        let f = format_bson::Bson;
        let schemas = format::Format::parse(&f, "message Greeting { prefix: string\n excited: bool }", &mut |_| unreachable!()).unwrap();
        AllSchemas::from([(format::Format::content_type(&f).to_string(), schemas)])
    }

    fn parse(line: &str) -> Result<Line> {
        super::parse(line, &formats(), &messages())
    }

    #[test]
    fn requests_are_written_as_in_an_image() {
        assert_eq!(parse("  ").unwrap(), Line::Empty);
        assert_eq!(parse("read /a/b").unwrap(), Line::Request { verb: Verb::Read, path: "/a/b".into(), body: None });
        let Line::Request { body: Some(p), .. } = parse(r#"write /x "hi""#).unwrap() else { panic!() };
        assert_eq!(p, Payload::text("hi"));
        let Line::Request { body: Some(p), .. } = parse(r#"write /x text/html "<p>""#).unwrap() else { panic!() };
        assert_eq!(p.content_type.as_str(), "text/html");
        let Line::Request { body: Some(p), .. } = parse(r#"write /g Greeting { prefix: "Hi", excited: true }"#).unwrap() else { panic!() };
        assert_eq!(p.content_type.as_str(), "application/bson");
    }

    #[test]
    fn messages_are_checked_before_they_are_sent() {
        let e = |line: &str| format!("{:#}", parse(line).unwrap_err());
        assert!(e(r#"write /g Greeting { prefix: 3, excited: true }"#).contains("prefix"));
        assert!(e(r#"write /g Greeting { prefix: "x" }"#).contains("excited"));
        assert!(e(r#"write /g Nope { }"#).contains("no message type `Nope`"));
        assert!(e(r#"write /g { a: 1 }"#).contains("name the message type"));
    }

    #[test]
    fn only_one_request_at_a_time() {
        assert!(parse("read /a read /b").is_err());
        assert!(parse("instance x : X").is_err());
        assert!(parse("needs /x Console").is_err());
    }

    #[test]
    fn shell_commands_start_with_a_colon() {
        assert_eq!(parse(":roots").unwrap(), Line::Shell(Builtin::Roots));
        assert_eq!(parse(":look web.http").unwrap(), Line::Shell(Builtin::Look("web.http".into())));
        assert_eq!(parse(":cancel").unwrap(), Line::Shell(Builtin::Cancel(None)));
        assert_eq!(parse(":cancel 4").unwrap(), Line::Shell(Builtin::Cancel(Some(4))));
        assert_eq!(parse(":type  hello there").unwrap(), Line::Shell(Builtin::Type("hello there".into())));
        assert!(parse(":look").is_err());
        assert!(parse(":nope").is_err());
    }
}

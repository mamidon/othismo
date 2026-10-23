//! The BSON format's AssemblyScript bindings: a class for each message
//! type, with its encoder and decoder.

use crate::Messages;
use crate::messages::{FieldType, MessageType};
use format::Bindings;
use std::fmt::Write;

/// The classes for `types`, each of which `schemas` has, in that order.
/// `library` is where `Message`, `Reader` and `Writer` are imported from.
pub fn assemblyscript(schemas: &Messages, types: &[String], library: &str) -> Bindings {
    let imports = format!("import {{ Message, Reader, Writer, registerMessage }} from \"{library}\"\n");
    let mut code = String::new();
    for name in types {
        let ty = &schemas.types[name];
        code.push('\n');
        class(&mut code, ty);
    }
    Bindings { imports, code }
}

fn class(out: &mut String, ty: &MessageType) {
    let name = &ty.name;
    doc(out, "", &ty.doc);
    writeln!(out, "export class {name} extends Message {{").unwrap();
    for ((field, fty), fdoc) in ty.fields.iter().zip(&ty.field_docs) {
        doc(out, "  ", fdoc);
        writeln!(out, "  {field}: {}", as_type(*fty)).unwrap();
    }
    if !ty.fields.is_empty() {
        let params: Vec<String> = ty.fields.iter().map(|(f, t)| format!("{f}: {}", as_type(*t))).collect();
        writeln!(out, "\n  constructor({}) {{\n    super()", params.join(", ")).unwrap();
        for (field, _) in &ty.fields {
            writeln!(out, "    this.{field} = {field}").unwrap();
        }
        out.push_str("  }\n");
    }
    let gap = if ty.fields.is_empty() { "" } else { "\n" };
    writeln!(out, "{gap}  typeName(): string {{ return \"{name}\" }}").unwrap();
    let writes: Vec<String> = ty.fields.iter().map(|(f, t)| write_call(f, *t)).collect();
    match writes.as_slice() {
        [] => out.push_str("  encode(w: Writer): void {}\n"),
        [one] => writeln!(out, "  encode(w: Writer): void {{ {one} }}").unwrap(),
        many => {
            out.push_str("  encode(w: Writer): void {\n");
            for w in many {
                writeln!(out, "    {w}").unwrap();
            }
            out.push_str("  }\n");
        }
    }
    out.push_str("}\n");
    let reads: Vec<String> = ty.fields.iter().map(|(f, t)| read_call(f, *t)).collect();
    writeln!(out, "registerMessage(\"{name}\", (r: Reader): Message => new {name}({}))", reads.join(", ")).unwrap();
}

/// A JSDoc comment, if there is anything to say. A `*/` in it, as in
/// `*/*`, would end it, so it is written `*\/`.
fn doc(out: &mut String, indent: &str, text: &str) {
    let text = text.replace("*/", "*\\/");
    match text.lines().collect::<Vec<_>>().as_slice() {
        [] => {}
        [one] => writeln!(out, "{indent}/** {one} */").unwrap(),
        lines => {
            writeln!(out, "{indent}/**").unwrap();
            for line in lines {
                writeln!(out, "{indent} *{}{line}", if line.is_empty() { "" } else { " " }).unwrap();
            }
            writeln!(out, "{indent} */").unwrap();
        }
    }
}

fn as_type(t: FieldType) -> &'static str {
    match t {
        FieldType::Bytes => "ArrayBuffer",
        FieldType::String => "string",
        FieldType::Bool => "bool",
        FieldType::I8 => "i8",
        FieldType::I16 => "i16",
        FieldType::I32 => "i32",
        FieldType::I64 => "i64",
        FieldType::U8 => "u8",
        FieldType::U16 => "u16",
        FieldType::U32 => "u32",
        FieldType::U64 => "u64",
        FieldType::F32 => "f32",
        FieldType::F64 => "f64",
    }
}

/// The `Writer` call for one field, following `wire.rs`'s mapping.
fn write_call(f: &str, t: FieldType) -> String {
    match t {
        FieldType::String => format!("w.string(\"{f}\", this.{f})"),
        FieldType::Bool => format!("w.bool(\"{f}\", this.{f})"),
        FieldType::Bytes => format!("w.bytes(\"{f}\", this.{f})"),
        FieldType::I32 => format!("w.i32(\"{f}\", this.{f})"),
        FieldType::I8 | FieldType::I16 | FieldType::U8 | FieldType::U16 => format!("w.i32(\"{f}\", <i32>this.{f})"),
        FieldType::I64 => format!("w.i64(\"{f}\", this.{f})"),
        FieldType::U32 | FieldType::U64 => format!("w.i64(\"{f}\", <i64>this.{f})"),
        FieldType::F64 => format!("w.f64(\"{f}\", this.{f})"),
        FieldType::F32 => format!("w.f64(\"{f}\", <f64>this.{f})"),
    }
}

fn read_call(f: &str, t: FieldType) -> String {
    match t {
        FieldType::String => format!("r.string(\"{f}\")"),
        FieldType::Bool => format!("r.bool(\"{f}\")"),
        FieldType::Bytes => format!("r.bytes(\"{f}\")"),
        FieldType::I32 => format!("r.i32(\"{f}\")"),
        FieldType::I64 => format!("r.i64(\"{f}\")"),
        FieldType::F64 => format!("r.f64(\"{f}\")"),
        FieldType::F32 => format!("<f32>r.f64(\"{f}\")"),
        other => format!("<{}>r.i64(\"{f}\")", as_type(other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Bson;

    #[test]
    fn generates_a_class_per_declared_message() {
        let src = r#"
            ## Configures a Greeter.
            message Greeting {
              ## Put before each name.
              prefix: string
              excited: bool
            }
            ## Two lines
            ##
            ## of docs.
            message Odd { a: u8, b: f32, c: u64, d: bytes }
            message Ping {}
        "#;
        let schemas = Bson::parse_messages(src, &mut |_| unreachable!()).unwrap();
        let Bindings { imports, code: out } = assemblyscript(&schemas, &schemas.declared, "othismo/bson");
        assert_eq!(imports, "import { Message, Reader, Writer, registerMessage } from \"othismo/bson\"\n");
        let greeting = r#"/** Configures a Greeter. */
export class Greeting extends Message {
  /** Put before each name. */
  prefix: string
  excited: bool

  constructor(prefix: string, excited: bool) {
    super()
    this.prefix = prefix
    this.excited = excited
  }

  typeName(): string { return "Greeting" }
  encode(w: Writer): void {
    w.string("prefix", this.prefix)
    w.bool("excited", this.excited)
  }
}
registerMessage("Greeting", (r: Reader): Message => new Greeting(r.string("prefix"), r.bool("excited")))
"#;
        assert!(out.contains(greeting), "{out}");
        assert!(out.contains("/**\n * Two lines\n *\n * of docs.\n */\nexport class Odd"), "{out}");
        assert!(out.contains("w.i32(\"a\", <i32>this.a)"), "{out}");
        assert!(out.contains("new Odd(<u8>r.i64(\"a\"), <f32>r.f64(\"b\"), <u64>r.i64(\"c\"), r.bytes(\"d\"))"), "{out}");
        assert!(out.contains("export class Ping extends Message {\n  typeName(): string { return \"Ping\" }\n  encode(w: Writer): void {}\n}"), "{out}");
    }
}

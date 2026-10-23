//! The BSON format's AssemblyScript bindings: a class for each message
//! type, with its encoder and decoder.

use crate::Messages;
use crate::messages::{FieldType, MessageType};
use format::Bindings;
use std::collections::BTreeMap;
use std::fmt::Write;

/// The classes for `types`, each of which `schemas` has, in that order.
/// `library` is the SDK, `"othismo"`; `Message`, `Reader` and `Writer` are
/// imported from its `bson`. Each class has what every format's do, for
/// the server bases and clients: `fromPayload`, the message in a payload
/// if it is one of this type, and `toPayload`.
pub fn assemblyscript(schemas: &Messages, types: &[String], library: &str) -> Bindings {
    let imports = format!(
        "import {{ Payload }} from \"{library}\"\nimport {{ Message, Reader, Writer, bson, registerMessage }} from \"{library}/bson\"\n"
    );
    // And every type their fields name, which have to be classes too.
    let mut all: Vec<String> = vec![];
    for name in types {
        for t in schemas.closure(name) {
            if !all.contains(&t) {
                all.push(t);
            }
        }
    }
    let mut code = String::new();
    let mut helpers = Helpers::default();
    for name in &all {
        let ty = &schemas.types[name];
        code.push('\n');
        class(&mut code, ty, &mut helpers);
    }
    for helper in helpers.0.values() {
        code.push('\n');
        code.push_str(helper);
    }
    Bindings { imports, code }
}

/// Functions that write and read lists and maps, by name: AssemblyScript has no
/// closures, so each element type gets its own.
#[derive(Default)]
struct Helpers(BTreeMap<String, String>);

fn class(out: &mut String, ty: &MessageType, helpers: &mut Helpers) {
    let name = &ty.name;
    doc(out, "", &ty.doc);
    writeln!(out, "export class {name} extends Message {{").unwrap();
    for (f, fdoc) in ty.fields.iter().zip(&ty.field_docs) {
        doc(out, "  ", fdoc);
        writeln!(out, "  {}: {}", f.name, as_type(&f.ty)).unwrap();
    }
    if !ty.fields.is_empty() {
        let params: Vec<String> = ty.fields.iter().map(|f| format!("{}: {}", f.name, as_type(&f.ty))).collect();
        writeln!(out, "\n  constructor({}) {{\n    super()", params.join(", ")).unwrap();
        for f in &ty.fields {
            writeln!(out, "    this.{0} = {0}", f.name).unwrap();
        }
        out.push_str("  }\n");
    }
    let gap = if ty.fields.is_empty() { "" } else { "\n" };
    writeln!(out, "{gap}  /** The `{name}` in `p`, or null if it holds none. */").unwrap();
    writeln!(out, "  static fromPayload(p: Payload | null): {name} | null {{").unwrap();
    writeln!(out, "    const m = bson.decode(p)\n    return m instanceof {name} ? changetype<{name}>(m) : null\n  }}").unwrap();
    writeln!(out, "  toPayload(): Payload {{ return bson.payload(this) }}\n").unwrap();
    writeln!(out, "  typeName(): string {{ return \"{name}\" }}").unwrap();
    let writes: Vec<String> = ty.fields.iter().map(|f| helpers.write(&format!("\"{}\"", f.name), &format!("this.{}", f.name), &f.ty)).collect();
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
    let reads: Vec<String> = ty.fields.iter().map(|f| helpers.read(&format!("\"{}\"", f.name), &f.ty)).collect();
    writeln!(out, "  static decode(r: Reader): {name} {{ return new {name}({}) }}", reads.join(", ")).unwrap();
    out.push_str("}\n");
    writeln!(out, "registerMessage(\"{name}\", (r: Reader): Message => {name}.decode(r))").unwrap();
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

fn as_type(t: &FieldType) -> String {
    match t {
        FieldType::Named(n) => n.clone(),
        FieldType::List(e) => format!("Array<{}>", as_type(e)),
        FieldType::Map(e) => format!("Map<string, {}>", as_type(e)),
        _ => scalar(t).to_string(),
    }
}

fn scalar(t: &FieldType) -> &'static str {
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
        FieldType::Named(_) | FieldType::List(_) | FieldType::Map(_) => unreachable!("not a scalar"),
    }
}

/// A name for `t` that can go in an identifier.
fn mangle(t: &FieldType) -> String {
    match t {
        FieldType::Named(n) => n.clone(),
        FieldType::List(e) => format!("list_{}", mangle(e)),
        FieldType::Map(e) => format!("map_{}", mangle(e)),
        _ => scalar(t).to_string(),
    }
}

impl Helpers {
    /// The `Writer` call that writes `value`, of type `t`, as key `key` of
    /// `w`, following `wire.rs`'s mapping.
    fn write(&mut self, key: &str, value: &str, t: &FieldType) -> String {
        match t {
            FieldType::String => format!("w.string({key}, {value})"),
            FieldType::Bool => format!("w.bool({key}, {value})"),
            FieldType::Bytes => format!("w.bytes({key}, {value})"),
            FieldType::I32 => format!("w.i32({key}, {value})"),
            FieldType::I8 | FieldType::I16 | FieldType::U8 | FieldType::U16 => format!("w.i32({key}, <i32>{value})"),
            FieldType::I64 => format!("w.i64({key}, {value})"),
            FieldType::U32 | FieldType::U64 => format!("w.i64({key}, <i64>{value})"),
            FieldType::F64 => format!("w.f64({key}, {value})"),
            FieldType::F32 => format!("w.f64({key}, <f64>{value})"),
            FieldType::Named(_) => format!("w.doc({key}, {value})"),
            FieldType::List(e) => {
                let f = format!("__write_{}", mangle(t));
                if !self.0.contains_key(&f) {
                    self.0.insert(f.clone(), String::new());
                    let each = self.write("i.toString()", "v[i]", e);
                    let body = format!(
                        "function {f}(v: {}): Writer {{\n  const w = new Writer()\n  for (let i = 0; i < v.length; i++) {each}\n  return w\n}}\n",
                        as_type(t)
                    );
                    self.0.insert(f.clone(), body);
                }
                format!("w.array({key}, {f}({value}))")
            }
            FieldType::Map(e) => {
                let f = format!("__write_{}", mangle(t));
                if !self.0.contains_key(&f) {
                    self.0.insert(f.clone(), String::new());
                    let each = self.write("keys[i]", "v.get(keys[i])", e);
                    let body = format!(
                        "function {f}(v: {}): Writer {{\n  const w = new Writer()\n  const keys = v.keys()\n  for (let i = 0; i < keys.length; i++) {each}\n  return w\n}}\n",
                        as_type(t)
                    );
                    self.0.insert(f.clone(), body);
                }
                format!("w.map({key}, {f}({value}))")
            }
        }
    }

    /// The expression that reads key `key` of `r` as type `t`.
    fn read(&mut self, key: &str, t: &FieldType) -> String {
        match t {
            FieldType::String => format!("r.string({key})"),
            FieldType::Bool => format!("r.bool({key})"),
            FieldType::Bytes => format!("r.bytes({key})"),
            FieldType::I32 => format!("r.i32({key})"),
            FieldType::I64 => format!("r.i64({key})"),
            FieldType::F64 => format!("r.f64({key})"),
            FieldType::F32 => format!("<f32>r.f64({key})"),
            FieldType::I8 | FieldType::I16 | FieldType::U8 | FieldType::U16 | FieldType::U32 | FieldType::U64 => {
                format!("<{}>r.i64({key})", scalar(t))
            }
            FieldType::Named(n) => format!("{n}.decode(r.doc({key}))"),
            FieldType::List(e) => {
                let f = format!("__read_{}", mangle(t));
                if !self.0.contains_key(&f) {
                    self.0.insert(f.clone(), String::new());
                    let each = self.read("keys[i]", e);
                    let body = format!(
                        "function {f}(r: Reader): {ty} {{\n  const keys = r.keys()\n  const out = new {ty}()\n  for (let i = 0; i < keys.length; i++) out.push({each})\n  return out\n}}\n",
                        ty = as_type(t)
                    );
                    self.0.insert(f.clone(), body);
                }
                format!("{f}(r.array({key}))")
            }
            FieldType::Map(e) => {
                let f = format!("__read_{}", mangle(t));
                if !self.0.contains_key(&f) {
                    self.0.insert(f.clone(), String::new());
                    let each = self.read("keys[i]", e);
                    let body = format!(
                        "function {f}(r: Reader): {ty} {{\n  const keys = r.keys()\n  const out = new {ty}()\n  for (let i = 0; i < keys.length; i++) out.set(keys[i], {each})\n  return out\n}}\n",
                        ty = as_type(t)
                    );
                    self.0.insert(f.clone(), body);
                }
                format!("{f}(r.map({key}))")
            }
        }
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
            message Nested { greeting: Greeting }
            message Lists { names: [string], grid: [[u8]], people: [Greeting] }
            message Maps { env: {string}, groups: {[Greeting]} }
        "#;
        let schemas = Bson::parse_messages(src, &mut |_| unreachable!()).unwrap();
        let Bindings { imports, code: out } = assemblyscript(&schemas, &schemas.declared, "othismo");
        assert_eq!(imports, "import { Payload } from \"othismo\"\nimport { Message, Reader, Writer, bson, registerMessage } from \"othismo/bson\"\n");
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

  /** The `Greeting` in `p`, or null if it holds none. */
  static fromPayload(p: Payload | null): Greeting | null {
    const m = bson.decode(p)
    return m instanceof Greeting ? changetype<Greeting>(m) : null
  }
  toPayload(): Payload { return bson.payload(this) }

  typeName(): string { return "Greeting" }
  encode(w: Writer): void {
    w.string("prefix", this.prefix)
    w.bool("excited", this.excited)
  }
  static decode(r: Reader): Greeting { return new Greeting(r.string("prefix"), r.bool("excited")) }
}
registerMessage("Greeting", (r: Reader): Message => Greeting.decode(r))
"#;
        assert!(out.contains(greeting), "{out}");
        assert!(out.contains("/**\n * Two lines\n *\n * of docs.\n */\nexport class Odd"), "{out}");
        assert!(out.contains("w.i32(\"a\", <i32>this.a)"), "{out}");
        assert!(out.contains("new Odd(<u8>r.i64(\"a\"), <f32>r.f64(\"b\"), <u64>r.i64(\"c\"), r.bytes(\"d\"))"), "{out}");
        assert!(out.contains("w.doc(\"greeting\", this.greeting)"), "{out}");
        assert!(out.contains("new Nested(Greeting.decode(r.doc(\"greeting\")))"), "{out}");
        assert!(out.contains("  names: Array<string>\n  grid: Array<Array<u8>>\n"), "{out}");
        assert!(out.contains("w.array(\"grid\", __write_list_list_u8(this.grid))"), "{out}");
        assert!(out.contains("new Lists(__read_list_string(r.array(\"names\")), __read_list_list_u8(r.array(\"grid\")), __read_list_Greeting(r.array(\"people\")))"), "{out}");
        let write = "function __write_list_list_u8(v: Array<Array<u8>>): Writer {\n  const w = new Writer()\n  for (let i = 0; i < v.length; i++) w.array(i.toString(), __write_list_u8(v[i]))\n  return w\n}\n";
        assert!(out.contains(write), "{out}");
        let read = "function __read_list_list_u8(r: Reader): Array<Array<u8>> {\n  const keys = r.keys()\n  const out = new Array<Array<u8>>()\n  for (let i = 0; i < keys.length; i++) out.push(__read_list_u8(r.array(keys[i])))\n  return out\n}\n";
        assert!(out.contains(read), "{out}");
        assert!(out.contains("out.push(<u8>r.i64(keys[i]))") && out.contains("out.push(Greeting.decode(r.doc(keys[i])))"), "{out}");
        assert!(out.contains("  env: Map<string, string>\n  groups: Map<string, Array<Greeting>>\n"), "{out}");
        assert!(out.contains("w.map(\"groups\", __write_map_list_Greeting(this.groups))"), "{out}");
        assert!(out.contains("new Maps(__read_map_string(r.map(\"env\")), __read_map_list_Greeting(r.map(\"groups\")))"), "{out}");
        assert!(out.contains("  const keys = v.keys()\n  for (let i = 0; i < keys.length; i++) w.array(keys[i], __write_list_Greeting(v.get(keys[i])))\n"), "{out}");
        assert!(out.contains("  for (let i = 0; i < keys.length; i++) out.set(keys[i], r.string(keys[i]))\n"), "{out}");
        let only = assemblyscript(&schemas, &["Nested".into()], "othismo").code;
        assert!(only.find("class Nested") < only.find("class Greeting"), "the types a field names come with it: {only}");
        assert!(out.contains("export class Ping extends Message {\n  /** The `Ping` in `p`, or null if it holds none. */\n"), "{out}");
        assert!(out.contains("  typeName(): string { return \"Ping\" }\n  encode(w: Writer): void {}\n  static decode(r: Reader): Ping { return new Ping() }\n}"), "{out}");
    }
}

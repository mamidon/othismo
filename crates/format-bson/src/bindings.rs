//! The BSON format's AssemblyScript bindings: a class for each message
//! type, with its encoder and decoder, and an enum for each enum.

use crate::Messages;
use crate::messages::{EnumType, Field, FieldType, MessageType};
use format::Bindings;
use std::collections::BTreeMap;
use std::fmt::Write;

/// The classes for `types`, each of which `schemas` has, in that order,
/// and the enums `schemas` declares. `library` is the SDK, `"othismo"`;
/// `Message`, `Reader` and `Writer` are imported from its `bson`. Each
/// class has what every format's do, for the server bases and clients:
/// `fromPayload`, the message in a payload if it is one of this type, and
/// `toPayload`.
pub fn assemblyscript(schemas: &Messages, types: &[String], library: &str) -> Bindings {
    // And every type their fields name, which have to be declared too.
    let mut all: Vec<String> = vec![];
    for name in types.iter().chain(schemas.declared.iter().filter(|t| schemas.enums.contains_key(*t))) {
        for t in schemas.closure(name) {
            if !all.contains(&t) {
                all.push(t);
            }
        }
    }
    let mut g = Gen { schemas, helpers: BTreeMap::new() };
    let fields = || all.iter().filter_map(|t| schemas.types.get(t)).flat_map(|t| &t.fields);
    let boxed = fields().any(|f| f.optional && g.is_value(&f.ty));
    let uuids = fields().any(|f| has_uuid(&f.ty));
    let imports = format!(
        "import {{ Payload }} from \"{library}\"\nimport {{ {}Message, Reader, {}Writer, bson, registerMessage }} from \"{library}/bson\"\n",
        if boxed { "Box, " } else { "" },
        if uuids { "Uuid, " } else { "" },
    );
    let mut code = String::new();
    for name in &all {
        code.push('\n');
        match schemas.enums.get(name) {
            Some(e) => enumeration(&mut code, e),
            None => g.class(&mut code, &schemas.types[name]),
        }
    }
    for helper in g.helpers.values() {
        code.push('\n');
        code.push_str(helper);
    }
    Bindings { imports, code }
}

/// What the bindings are generated from, and the functions they need that
/// aren't a class's, by name: AssemblyScript has no closures, so each list
/// and map type gets its own to write and read it.
struct Gen<'a> {
    schemas: &'a Messages,
    helpers: BTreeMap<String, String>,
}

/// An AssemblyScript enum, and functions between its values and their names,
/// which are what the wire carries.
fn enumeration(out: &mut String, e: &EnumType) {
    let name = &e.name;
    doc(out, "", &e.doc);
    writeln!(out, "export enum {name} {{").unwrap();
    for (v, vdoc) in e.values.iter().zip(&e.value_docs) {
        doc(out, "  ", vdoc);
        writeln!(out, "  {v},").unwrap();
    }
    out.push_str("}\n");
    writeln!(out, "\n/** The name of a `{name}`, as the wire carries it. */").unwrap();
    writeln!(out, "export function nameOf{name}(v: {name}): string {{\n  switch (v) {{").unwrap();
    for v in &e.values {
        writeln!(out, "    case {name}.{v}: return \"{v}\"").unwrap();
    }
    writeln!(out, "  }}\n  abort(\"not a {name}\")\n  return \"\"\n}}").unwrap();
    writeln!(out, "\nfunction __parse_{name}(s: string): {name} {{").unwrap();
    for v in &e.values {
        writeln!(out, "  if (s == \"{v}\") return {name}.{v}").unwrap();
    }
    writeln!(out, "  abort(\"`\" + s + \"` is not a {name}\")\n  return 0\n}}").unwrap();
}

impl Gen<'_> {
    fn class(&mut self, out: &mut String, ty: &MessageType) {
        let name = &ty.name;
        doc(out, "", &ty.doc);
        writeln!(out, "export class {name} extends Message {{").unwrap();
        for (f, fdoc) in ty.fields.iter().zip(&ty.field_docs) {
            doc(out, "  ", fdoc);
            writeln!(out, "  {}: {}", f.name, self.field_type(f)).unwrap();
        }
        if !ty.fields.is_empty() {
            let params: Vec<String> = ty.fields.iter().map(|f| format!("{}: {}", f.name, self.field_type(f))).collect();
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
        let writes: Vec<String> = ty.fields.iter().map(|f| self.write_field(f)).collect();
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
        let reads: Vec<String> = ty.fields.iter().map(|f| self.read_field(f)).collect();
        writeln!(out, "  static decode(r: Reader): {name} {{ return new {name}({}) }}", reads.join(", ")).unwrap();
        out.push_str("}\n");
        writeln!(out, "registerMessage(\"{name}\", (r: Reader): Message => {name}.decode(r))").unwrap();
    }

    /// A field's type in AssemblyScript. An optional field may be null, so
    /// one of a type that can't be, such as `i32`, is boxed.
    fn field_type(&self, f: &Field) -> String {
        match (f.optional, self.is_value(&f.ty)) {
            (false, _) => as_type(&f.ty),
            (true, false) => format!("{} | null", as_type(&f.ty)),
            (true, true) => format!("Box<{}> | null", as_type(&f.ty)),
        }
    }

    /// Whether AssemblyScript keeps `t` by value, so that it can't be null.
    fn is_value(&self, t: &FieldType) -> bool {
        use FieldType::*;
        match t {
            Bool | I8 | I16 | I32 | I64 | U8 | U16 | U32 | U64 | F32 | F64 => true,
            Named(n) => self.schemas.enums.contains_key(n),
            _ => false,
        }
    }

    /// The statement that writes field `f` in `encode`: an optional one
    /// only if it isn't null.
    fn write_field(&mut self, f: &Field) -> String {
        let (key, this) = (format!("\"{}\"", f.name), format!("this.{}", f.name));
        match (f.optional, self.is_value(&f.ty)) {
            (false, _) => self.write(&key, &this, &f.ty),
            (true, false) => format!("if ({this} !== null) {}", self.write(&key, &format!("{this}!"), &f.ty)),
            (true, true) => format!("if ({this} !== null) {}", self.write(&key, &format!("{this}!.value"), &f.ty)),
        }
    }

    /// The expression that reads field `f` in `decode`: null for an
    /// optional one that is missing, or null on the wire.
    fn read_field(&mut self, f: &Field) -> String {
        let key = format!("\"{}\"", f.name);
        let read = self.read(&key, &f.ty);
        match (f.optional, self.is_value(&f.ty)) {
            (false, _) => read,
            (true, false) => format!("r.has({key}) ? <{}>{read} : null", self.field_type(f)),
            (true, true) => format!("r.has({key}) ? <{}>new Box<{}>({read}) : null", self.field_type(f), as_type(&f.ty)),
        }
    }

    /// The `Writer` call that writes `value`, of type `t`, as key `key` of
    /// `w`, following `wire.rs`'s mapping.
    fn write(&mut self, key: &str, value: &str, t: &FieldType) -> String {
        match t {
            FieldType::String => format!("w.string({key}, {value})"),
            FieldType::Bool => format!("w.bool({key}, {value})"),
            FieldType::Bytes => format!("w.bytes({key}, {value})"),
            FieldType::Time => format!("w.time({key}, {value})"),
            FieldType::Uuid => format!("w.uuid({key}, {value})"),
            FieldType::I32 => format!("w.i32({key}, {value})"),
            FieldType::I8 | FieldType::I16 | FieldType::U8 | FieldType::U16 => format!("w.i32({key}, <i32>{value})"),
            FieldType::I64 => format!("w.i64({key}, {value})"),
            FieldType::U32 | FieldType::U64 => format!("w.i64({key}, <i64>{value})"),
            FieldType::F64 => format!("w.f64({key}, {value})"),
            FieldType::F32 => format!("w.f64({key}, <f64>{value})"),
            FieldType::Named(n) if self.schemas.enums.contains_key(n) => format!("w.string({key}, nameOf{n}({value}))"),
            FieldType::Named(_) => format!("w.doc({key}, {value})"),
            FieldType::List(e) => {
                let f = format!("__write_{}", mangle(t));
                if !self.helpers.contains_key(&f) {
                    self.helpers.insert(f.clone(), String::new());
                    let each = self.write("i.toString()", "v[i]", e);
                    let body = format!(
                        "function {f}(v: {}): Writer {{\n  const w = new Writer()\n  for (let i = 0; i < v.length; i++) {each}\n  return w\n}}\n",
                        as_type(t)
                    );
                    self.helpers.insert(f.clone(), body);
                }
                format!("w.array({key}, {f}({value}))")
            }
            FieldType::Map(e) => {
                let f = format!("__write_{}", mangle(t));
                if !self.helpers.contains_key(&f) {
                    self.helpers.insert(f.clone(), String::new());
                    let each = self.write("keys[i]", "v.get(keys[i])", e);
                    let body = format!(
                        "function {f}(v: {}): Writer {{\n  const w = new Writer()\n  const keys = v.keys()\n  for (let i = 0; i < keys.length; i++) {each}\n  return w\n}}\n",
                        as_type(t)
                    );
                    self.helpers.insert(f.clone(), body);
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
            FieldType::Time => format!("r.time({key})"),
            FieldType::Uuid => format!("r.uuid({key})"),
            FieldType::I32 => format!("r.i32({key})"),
            FieldType::I64 => format!("r.i64({key})"),
            FieldType::F64 => format!("r.f64({key})"),
            FieldType::F32 => format!("<f32>r.f64({key})"),
            FieldType::I8 | FieldType::I16 | FieldType::U8 | FieldType::U16 | FieldType::U32 | FieldType::U64 => {
                format!("<{}>r.i64({key})", scalar(t))
            }
            FieldType::Named(n) if self.schemas.enums.contains_key(n) => format!("__parse_{n}(r.string({key}))"),
            FieldType::Named(n) => format!("{n}.decode(r.doc({key}))"),
            FieldType::List(e) => {
                let f = format!("__read_{}", mangle(t));
                if !self.helpers.contains_key(&f) {
                    self.helpers.insert(f.clone(), String::new());
                    let each = self.read("keys[i]", e);
                    let body = format!(
                        "function {f}(r: Reader): {ty} {{\n  const keys = r.keys()\n  const out = new {ty}()\n  for (let i = 0; i < keys.length; i++) out.push({each})\n  return out\n}}\n",
                        ty = as_type(t)
                    );
                    self.helpers.insert(f.clone(), body);
                }
                format!("{f}(r.array({key}))")
            }
            FieldType::Map(e) => {
                let f = format!("__read_{}", mangle(t));
                if !self.helpers.contains_key(&f) {
                    self.helpers.insert(f.clone(), String::new());
                    let each = self.read("keys[i]", e);
                    let body = format!(
                        "function {f}(r: Reader): {ty} {{\n  const keys = r.keys()\n  const out = new {ty}()\n  for (let i = 0; i < keys.length; i++) out.set(keys[i], {each})\n  return out\n}}\n",
                        ty = as_type(t)
                    );
                    self.helpers.insert(f.clone(), body);
                }
                format!("{f}(r.map({key}))")
            }
        }
    }
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

fn has_uuid(t: &FieldType) -> bool {
    match t {
        FieldType::Uuid => true,
        FieldType::List(e) | FieldType::Map(e) => has_uuid(e),
        _ => false,
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
        FieldType::Time => "Date",
        FieldType::Uuid => "Uuid",
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
            message Opt { nick?: string, n?: u8, g?: Greeting }
            message Moments { at: time, ids: [uuid], when?: time }
            ## A color.
            enum Color { red, green }
            message Paint { color: Color, fav?: Color, palette: [Color] }
        "#;
        let schemas = Bson::parse_messages(src, &mut |_| unreachable!()).unwrap();
        let Bindings { imports, code: out } = assemblyscript(&schemas, &schemas.declared, "othismo");
        let plain = assemblyscript(&schemas, &["Greeting".into()], "othismo").imports;
        assert_eq!(plain, "import { Payload } from \"othismo\"\nimport { Message, Reader, Writer, bson, registerMessage } from \"othismo/bson\"\n");
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
        assert!(imports.contains("import { Box, Message, Reader"), "{imports}");
        assert!(out.contains("  nick: string | null\n  n: Box<u8> | null\n  g: Greeting | null\n"), "{out}");
        assert!(out.contains("constructor(nick: string | null, n: Box<u8> | null, g: Greeting | null)"), "{out}");
        assert!(out.contains("    if (this.nick !== null) w.string(\"nick\", this.nick!)\n    if (this.n !== null) w.i32(\"n\", <i32>this.n!.value)\n"), "{out}");
        let read = "new Opt(r.has(\"nick\") ? <string | null>r.string(\"nick\") : null, r.has(\"n\") ? <Box<u8> | null>new Box<u8>(<u8>r.i64(\"n\")) : null, r.has(\"g\") ? <Greeting | null>Greeting.decode(r.doc(\"g\")) : null)";
        assert!(out.contains(read), "{out}");
        assert!(!assemblyscript(&schemas, &["Greeting".into()], "othismo").imports.contains("Box"), "only when it is used");
        assert!(imports.contains(", Reader, Uuid, Writer,"), "{imports}");
        assert!(out.contains("  at: Date\n  ids: Array<Uuid>\n  when: Date | null\n"), "{out}");
        assert!(out.contains("w.time(\"at\", this.at)") && out.contains("w.uuid(i.toString(), v[i])"), "{out}");
        assert!(out.contains("new Moments(r.time(\"at\"), __read_list_Uuid(r.array(\"ids\")), r.has(\"when\") ? <Date | null>r.time(\"when\") : null)"), "{out}");
        let color = "/** A color. */\nexport enum Color {\n  red,\n  green,\n}\n\n/** The name of a `Color`, as the wire carries it. */\nexport function nameOfColor(v: Color): string {\n  switch (v) {\n    case Color.red: return \"red\"\n    case Color.green: return \"green\"\n  }\n  abort(\"not a Color\")\n  return \"\"\n}\n\nfunction __parse_Color(s: string): Color {\n  if (s == \"red\") return Color.red\n  if (s == \"green\") return Color.green\n  abort(\"`\" + s + \"` is not a Color\")\n  return 0\n}\n";
        assert!(out.contains(color), "{out}");
        assert!(out.contains("  color: Color\n  fav: Box<Color> | null\n  palette: Array<Color>\n"), "{out}");
        assert!(out.contains("w.string(\"color\", nameOfColor(this.color))") && out.contains("if (this.fav !== null) w.string(\"fav\", nameOfColor(this.fav!.value))"), "{out}");
        assert!(out.contains("new Paint(__parse_Color(r.string(\"color\")), r.has(\"fav\") ? <Box<Color> | null>new Box<Color>(__parse_Color(r.string(\"fav\"))) : null, __read_list_Color(r.array(\"palette\")))"), "{out}");
        let alone = Bson::parse_messages("enum Color { red } message M {}", &mut |_| unreachable!()).unwrap();
        assert!(assemblyscript(&alone, &[], "othismo").code.contains("export enum Color"), "a file's enums are bound even if no message names them");
        let only = assemblyscript(&schemas, &["Nested".into()], "othismo").code;
        assert!(only.find("class Nested") < only.find("class Greeting"), "the types a field names come with it: {only}");
        assert!(out.contains("export class Ping extends Message {\n  /** The `Ping` in `p`, or null if it holds none. */\n"), "{out}");
        assert!(out.contains("  typeName(): string { return \"Ping\" }\n  encode(w: Writer): void {}\n  static decode(r: Reader): Ping { return new Ping() }\n}"), "{out}");
    }
}

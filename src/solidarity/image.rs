use core::panic;
use std::collections::HashMap;
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use bson::Document;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use wasmbin::builtins::{Blob, Lazy, UnparsedBytes};
use wasmbin::io::Decode;
use wasmbin::sections::{payload, CustomSection, Kind, RawCustomSection, Section};
use wasmbin::Module;
use wasmer::{Global, GlobalType, Store, Type};
use crate::solidarity::SolidarityError::{ImageAlreadyExists, ObjectAlreadyExists, ObjectDoesNotExist, ObjectNotFree};
use crate::solidarity::{Errors, Result,};

pub struct InstanceAtRest(wasmbin::Module);
pub struct ModuleAtRest(wasmbin::Module);

pub enum Object {
    Module(ModuleAtRest),
    Instance(InstanceAtRest)
}

#[derive(Serialize, Deserialize)]
pub enum GlobalAtRest {
    I32(i32),
    I64(i64),
    F32(f32),
    F64(f64)
}

impl From<(&wasmer::Global, &mut Store)> for GlobalAtRest {
    fn from(tuple: (&wasmer::Global, &mut Store)) -> Self {
        let (global, store) = tuple;

        match global.get(store) {
            wasmer::Value::I32(i) => GlobalAtRest::I32(i),
            wasmer::Value::I64(i) => GlobalAtRest::I32(i),
            wasmer::Value::F32(f) => GlobalAtRest::F32(f),
            wasmer::Value::F64(f) => GlobalAtRest::F64(f),
            wasmer::Value::ExternRef(extern_ref) => todo!(),
            wasmer::Value::FuncRef(function) => todo!(),
            wasmer::Value::V128(_) => todo!(),
        }
    }
}

#[derive(Serialize, Deserialize)]
pub struct StateAtRest{
    globals: HashMap<String, GlobalAtRest>
}

// TODO InstanceAtRest must provide ways to read & write dehydrated state to consumers
impl InstanceAtRest {
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut buffer = Vec::new();
        self.0.encode_into(BufWriter::new(&mut buffer));

        buffer
    }

    pub fn persist_state_storage_section(&mut self) -> Result<()> {

        let buffer = {
            // todo, accept state at rest as a parameter
            let state = StateAtRest("Hello, world".to_owned());
            let mut document = Document::new();
            let mut buffer = Vec::new();
            
            document.insert("instance_state", bson::to_bson(&state)?);
            document.to_writer(BufWriter::new(&mut buffer));
    
            buffer
        };


        let payload = RawCustomSection {
            name: "mamidon".to_string(),
            data: UnparsedBytes {
                bytes: buffer
            }
        };

        for section in self.0.sections.iter_mut() {
            if let Section::Custom(custom_blob) = section {
                let decoded_blob = custom_blob.try_contents_mut()?;
                if let CustomSection::Other(raw) = decoded_blob {
                    if (raw.name == payload.name) {
                        raw.data = payload.data.clone();
                        
                        println!("mutated existing section: {}", self.0.sections.len());
                        return Ok(());
                    }
                }
            }
        }

        self.0.sections.push(Section::Custom(Blob { contents: Lazy::from(payload::Custom::Other(payload))}));

        Ok(())
    }
}

impl ModuleAtRest {
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut buffer = Vec::new();
        self.0.encode_into(BufWriter::new(&mut buffer));

        buffer
    }
}


impl From<wasmbin::Module> for InstanceAtRest {
    fn from(value: wasmbin::Module) -> Self {
        InstanceAtRest(value)
    }
}

impl From<ModuleAtRest> for InstanceAtRest {
    fn from(value: ModuleAtRest) -> Self {
        let mut instance = InstanceAtRest(value.0);
        instance.persist_state_storage_section().expect("Error dehydrating instance");

        instance
    }
}

impl From<wasmbin::Module> for ModuleAtRest {
    fn from(value: wasmbin::Module) -> Self {
        ModuleAtRest(value)
    }
}

impl Object {
    pub fn as_kind_str(&self) -> &'static str {
        match self {
            Object::Module(_) => "MODULE",
            Object::Instance(_) => "INSTANCE"
        }
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        fn to_vec(module: &wasmbin::Module) -> Vec<u8> {
            let mut buffer = Vec::new();
            module.encode_into(BufWriter::new(&mut buffer));

            buffer
        }


        match self {
            Object::Module(module) => module.to_bytes(),
            Object::Instance(instance) => instance.to_bytes()
        }
    }

    pub fn from_tuple(kind: &str, bytes: Vec<u8>) -> Result<Object> {
        match (kind) {
            "MODULE" => Ok(Object::Module(Module::decode_from(bytes.as_slice())?.into())),
            "INSTANCE" => Ok(Object::Instance(Module::decode_from(bytes.as_slice())?.into())),
            _ => panic!()
        }
    }

    pub fn new_module(bytes: &Vec<u8>) -> Result<Object> {
        Ok(Object::Module(Module::decode_from(bytes.as_slice())?.into()))
    }

    pub fn new_instance(object: &Object) -> Result<Object> {

        let module = match object {
            Object::Module(inner_module) => inner_module,
            _ => panic!("Should only pass in a module here")
        };

        let instance = module.0.clone();

        Ok(Object::Instance(instance.into()))
    }
}

pub enum LinkKind {
    InstanceOf
}

pub struct Link {
    kind: LinkKind,
    from: Object
}

pub struct Image {
    path_name: PathBuf,
    file: Connection,
}

impl Image {
    pub fn create<P: AsRef<Path>>(path: P) -> Result<Image> {
        if let Ok(_) = std::fs::metadata(&path) {
            Err(ImageAlreadyExists)?
        }

        let connection = Connection::open(path.as_ref())?;

        connection.execute_batch(include_str!("../sql_scripts/create_image_schema.sql"))?;

        Ok(Image {
            path_name: path.as_ref().to_path_buf(),
            file: connection
        })
    }

    fn create_in_memory() -> Result<Image> {
        let connection = Connection::open_in_memory()?;

        connection.execute_batch(include_str!("../sql_scripts/create_image_schema.sql"))?;

        Ok(Image {
            path_name: PathBuf::from("/in_memory"),
            file: connection
        })
    }

    pub fn open<P: AsRef<Path>>(path: P) -> Result<Image> {
        Ok(Image {
            path_name: path.as_ref().to_path_buf(),
            file: Connection::open(path)?
        })
    }

    pub fn import_object(&mut self, name: &str, object: Object) -> Result<()> {
        if self.object_exists(name)? {
            return Err(Errors::Solidarity(ObjectAlreadyExists));
        }

        self.insert_object(name, object.as_kind_str(), &object.to_bytes())?;

        Ok(())
    }

    pub fn get_object(&self, name: &str) -> Result<Object> {
        self.file.query_row(
            "select 
                kind, bytes
            from object o
            inner join namespace n on n.object_key = o.object_key
            where n.path = ?",
            params![name],
            |row|  Ok(Object::from_tuple(
                row.get::<usize, String>(0).map(|name| name)?.as_str(), 
                row.get(1)?
            )) 
        )?
    }

    pub fn remove_object(&mut self, name: &str) -> Result<()> {
        let object_key = self.get_object_key(name)?;

        let references: Option<i64> = self.file.query_row(r#"
            select
                count(*)
            from link L
            inner join object O on O.object_key = L.to_object_key
            inner join namespace NS on NS.object_key = O.object_key
            where NS.path = ?"#,
            params![name],
            |row| row.get(0)
        ).optional()?;

        if references.unwrap_or(0) > 0 {
            return Err(Errors::Solidarity(ObjectNotFree));
        }

        self.file.execute(r#"
        DELETE FROM namespace
        WHERE object_key = ?"#, params![object_key])?;

        self.file.execute(r#"
        DELETE FROM object where object_key = ?
        "#, params![object_key])?;

        Ok(())
    }

    pub fn object_exists(&self, name: &str) -> Result<bool> {
        let namespace_key: Option<i64> = self.file.query_row(
            "select count(*) from namespace where path = ?",
            params![name],
            |row| row.get(0)
        ).optional()?;

        return match namespace_key {
            Some(count) => Ok(count > 0),
            None => Ok(false)
        };
    }

    pub fn list_objects(&self, prefix: &str) -> Result<Vec<String>> {
        let mut statement = self.file.prepare(r#"
            SELECT
                NS.path
            FROM object O
            INNER JOIN namespace NS on NS.object_key = O.object_key
            WHERE path LIKE ? || '%'"#)?;
        let mut rows = statement.query(params![prefix])?;

        let mut names: Vec<String> = Vec::new();

        while let Some(row) = rows.next()? {
            names.push(row.get(0)?)
        }

        Ok(names)
    }

    fn get_object_key(&self, name: &str) -> Result<i64> {
        let object_key: Option<i64> = self.file.query_row(
            "select object_key from namespace where path = ?",
            params![name],
            |row| row.get(0)
        ).optional()?;

        return match object_key {
            Some(key) => Ok(key),
            None => Err(Errors::Solidarity(ObjectDoesNotExist))
        };
    }

    fn insert_object(&mut self, name: &str, kind: &str, bytes: &Vec<u8>) -> Result<()> {
        self.file.execute(
            "INSERT INTO object (kind, bytes) VALUES (?, ?)",
            params![
                kind,
                bytes
            ])?;
        let row_id = self.file.last_insert_rowid();

        self.upsert_name(name, row_id)?;

        Ok(())
    }

    fn upsert_name(&mut self, name: &str, object_key: i64) -> Result<()> {
        self.file.execute(
            "INSERT OR REPLACE INTO namespace (path, object_key) VALUES (?,?)",
            params![
            name,
            object_key
        ])?;

        Ok(())
    }
}

#[cfg(test)]
mod tests;

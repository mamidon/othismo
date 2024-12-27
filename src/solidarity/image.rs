use core::panic;
use std::any::Any;
use std::collections::HashMap;
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use bson::Document;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use wasmbin::builtins::{Blob, FloatConst, Lazy, UnparsedBytes};
use wasmbin::indices::{GlobalId, TypeId};
use wasmbin::instructions::Instruction;
use wasmbin::io::Decode;
use wasmbin::sections::{payload, CustomSection, Export, Global, Import, ImportDesc, Kind, RawCustomSection, Section};
use wasmbin::Module;
use wasmer::{GlobalType, Store, Type};
use crate::solidarity::SolidarityError::{ImageAlreadyExists, ObjectAlreadyExists, ObjectDoesNotExist, ObjectNotFree};
use crate::solidarity::{Errors, Result,};

use super::SolidarityError;

pub struct InstanceAtRest(wasmbin::Module);
pub struct ModuleAtRest(wasmbin::Module);

pub enum Object {
    Module(ModuleAtRest),
    Instance(InstanceAtRest)
}

#[derive(Serialize, Deserialize)]
pub enum GlobalAtRest {
    I32(i32, GlobalMutability),
    I64(i64, GlobalMutability),
    F32(f32, GlobalMutability),
    F64(f64, GlobalMutability)
}

impl GlobalAtRest {
    pub fn mutability(&self) -> &GlobalMutability {
        match self {
            GlobalAtRest::I32(_, global_mutability) => global_mutability,
            GlobalAtRest::I64(_, global_mutability) => global_mutability,
            GlobalAtRest::F32(_, global_mutability) => global_mutability,
            GlobalAtRest::F64(_, global_mutability) => global_mutability,
        }
    }
}

#[derive(Serialize, Deserialize)]
pub enum GlobalMutability {
    Const,
    Var
}

impl From<wasmer::Mutability> for GlobalMutability {
    fn from(mutability: wasmer::Mutability) -> Self {
        match mutability {
            wasmer::Mutability::Const => GlobalMutability::Const,
            wasmer::Mutability::Var => GlobalMutability::Var,
        }
    }
}

impl From<GlobalMutability> for wasmer::Mutability {
    fn from(mutability: GlobalMutability) -> Self {
        match mutability {
            GlobalMutability::Const => wasmer::Mutability::Const,
            GlobalMutability::Var => wasmer::Mutability::Var,
        }
    }
}

impl From<(&wasmer::Global, &mut Store)> for GlobalAtRest {
    fn from(tuple: (&wasmer::Global, &mut Store)) -> Self {
        let (global, store) = tuple;
        let mutability = global.ty(store).mutability.into();

        match global.get(store) {
            wasmer::Value::I32(i) => GlobalAtRest::I32(i, mutability),
            wasmer::Value::I64(i) => GlobalAtRest::I64(i, mutability),
            wasmer::Value::F32(f) => GlobalAtRest::F32(f, mutability),
            wasmer::Value::F64(f) => GlobalAtRest::F64(f, mutability),
            wasmer::Value::ExternRef(extern_ref) => todo!(),
            wasmer::Value::FuncRef(function) => todo!(),
            wasmer::Value::V128(_) => todo!(),
        }
    }
}

impl From<&GlobalAtRest> for wasmer::Value {
    fn from(value: &GlobalAtRest) -> Self {
        match value {
            GlobalAtRest::I32(v, _) => wasmer::Value::I32(*v),
            GlobalAtRest::I64(v, _) => wasmer::Value::I64(*v),
            GlobalAtRest::F32(v, _) => wasmer::Value::F32(*v),
            GlobalAtRest::F64(v, _) => wasmer::Value::F64(*v),
        }
    }
}

#[derive(Serialize, Deserialize)]
pub struct StateAtRest{
    pub globals: HashMap<String, GlobalAtRest>
}

// TODO InstanceAtRest must provide ways to read & write dehydrated state to consumers
impl InstanceAtRest {
    pub fn find_or_create_state(&self) -> Result<StateAtRest> {
        // Find a pre-existing state blob
        for section in self.0.sections.iter() {
            if let Section::Custom(custom_blob) = section {
                let decoded_blob = custom_blob.try_contents()?;
                if let CustomSection::Other(raw) = decoded_blob {
                    if (raw.name == "othismo") {
                        let state: StateAtRest = bson::from_slice(&raw.data)?;
                        return Ok(state);
                    }
                }
            }
        }

        // create a new state blob
        let mut state = StateAtRest {
            globals: HashMap::new()
        };

        if let Some(import_section) = self.0.find_std_section::<wasmbin::sections::payload::Import>() {
            for import in import_section.contents.try_contents()? {
                match &import.desc {
                    ImportDesc::Func(func) => todo!(),
                    ImportDesc::Global(global) => {
                        let mutability = match global.mutable {
                            true => GlobalMutability::Var,
                            false => GlobalMutability::Const
                        };

                        let value = match &global.value_type {
                            wasmbin::types::ValueType::V128 => todo!(),
                            wasmbin::types::ValueType::F64 => GlobalAtRest::F64(0.0, mutability),
                            wasmbin::types::ValueType::F32 => GlobalAtRest::F32(0.0, mutability),
                            wasmbin::types::ValueType::I32 => GlobalAtRest::I32(0, mutability),
                            wasmbin::types::ValueType::I64 => GlobalAtRest::I64(0, mutability),
                            wasmbin::types::ValueType::Ref(ref_type) => todo!(),
                        };
                        state.globals.insert(format!("{}.{}", import.path.module, import.path.name), value);
                    },
                    ImportDesc::Mem(mem) => todo!(),
                    ImportDesc::Table(table) => todo!(),
                }
            }
        }

        Ok(state)
     }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut buffer = Vec::new();
        self.0.encode_into(BufWriter::new(&mut buffer));

        buffer
    }

    pub fn persist_state_storage_section(&mut self) -> Result<()> {

        let buffer = {
            // todo, accept state at rest as a parameter
            let state = StateAtRest {
                globals: HashMap::new()
            };
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

    fn export_imported_globals(mut module: wasmbin::Module) -> Result<wasmbin::Module, Errors> {
        let extracted_globals = ModuleAtRest::extract_imported_globals(&mut module)?;
        ModuleAtRest::export_extracted_globals(&mut module, extracted_globals)?;

        Ok(module)
     }

     fn export_extracted_globals(module: &mut wasmbin::Module, extracted_globals: Vec<Global>) -> Result<(), Errors> {
        let global_section = module.find_or_insert_std_section(|| payload::Global::default());
        let mut existing_globals = global_section.contents.try_contents_mut()?;

        for global in extracted_globals.iter() {
            existing_globals.push(global.clone());
        }

        let exports_section = module.find_or_insert_std_section(|| payload::Export::default());
        let mut existing_exports = exports_section.contents.try_contents_mut()?;

        for (index, global) in extracted_globals.iter().enumerate().rev() {
            let name = format!("othismo_global_{}", index);
            let id: GlobalId = (index as u32).into();

            existing_exports.insert(0, Export {
                name,
                desc: wasmbin::sections::ExportDesc::Global(id)
            });
        }

        Ok(())
     }

     fn extract_imported_globals(module: &mut wasmbin::Module) -> Result<Vec<wasmbin::sections::Global>, Errors> {
        let import_section = module.find_or_insert_std_section(|| payload::Import::default());
        let mut imports = import_section.contents.try_contents_mut()?;
        let mut globals: Vec<wasmbin::sections::Global> = Vec::new();

        while let Some(index) = imports.iter().position(|import| matches!(import.desc, ImportDesc::Global(_))) {
            let Import { path, desc } = &imports[index];

            globals.push(match desc {
                ImportDesc::Global(ty) => {
                    Global {
                        ty: ty.clone(),
                        init: match &ty.value_type {
                            wasmbin::types::ValueType::F64 => vec![Instruction::F64Const(FloatConst { value: 0f64 })],
                            wasmbin::types::ValueType::F32 => vec![Instruction::F32Const(FloatConst { value: 0f32 })],
                            wasmbin::types::ValueType::I64 => vec![Instruction::I64Const(0)],
                            wasmbin::types::ValueType::I32 => vec![Instruction::I32Const(0)],
                            wasmbin::types::ValueType::Ref(_) => Err(SolidarityError::UnsupportedModuleDefinition("reference_type_global".to_string()))?,
                            wasmbin::types::ValueType::V128 => Err(SolidarityError::UnsupportedModuleDefinition("simd_global".to_string()))?,
                        }
                    }
                },
                _ => panic!()
            });

            imports.remove(index);
        }

        Ok(globals)
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

impl TryFrom<wasmbin::Module> for ModuleAtRest {
    type Error=Errors;

    fn try_from(module: wasmbin::Module) -> std::result::Result<Self, Self::Error> {
        Ok(ModuleAtRest(ModuleAtRest::export_imported_globals(module)?))
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
            "MODULE" => Ok(Object::Module(Module::decode_from(bytes.as_slice())?.try_into()?)),
            "INSTANCE" => Ok(Object::Instance(Module::decode_from(bytes.as_slice())?.into())),
            _ => panic!()
        }
    }

    pub fn new_module(bytes: &Vec<u8>) -> Result<Object> {
        Ok(Object::Module(Module::decode_from(bytes.as_slice())?.try_into()?))
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

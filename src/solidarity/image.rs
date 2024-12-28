use crate::solidarity::SolidarityError::{
    ImageAlreadyExists, ObjectAlreadyExists, ObjectDoesNotExist, ObjectNotFree,
};
use crate::solidarity::{Errors, Result};
use bson::Document;
use wasmbin::types::{Limits, MemType};
use core::panic;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::any::Any;
use std::collections::HashMap;
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use wasmbin::builtins::{Blob, FloatConst, Lazy, UnparsedBytes};
use wasmbin::indices::{GlobalId, MemId, TypeId};
use wasmbin::instructions::Instruction;
use wasmbin::io::Decode;
use wasmbin::sections::{
    payload, CustomSection, Export, ExportDesc, Global, Import, ImportDesc, Kind, RawCustomSection,
    Section,
};
use wasmbin::Module;
use wasmer::{GlobalType, Store, Type};

use super::SolidarityError;

pub struct InstanceAtRest(wasmbin::Module);
pub struct ModuleAtRest(wasmbin::Module);

pub enum Object {
    Module(ModuleAtRest),
    Instance(InstanceAtRest),
}

impl InstanceAtRest {
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut buffer = Vec::new();
        self.0.encode_into(BufWriter::new(&mut buffer));

        buffer
    }

    pub fn set_exported_global(&mut self, name: &str, value: wasmer::Value) -> Result<()> {
        let global_index = {
            let export = self
                .0
                .find_or_insert_std_section(|| payload::Export::default())
                .try_contents_mut()?
                .iter_mut()
                .find(|e| e.name == name)
                .expect("we should never set an non-extant export");

            match export.desc {
                wasmbin::sections::ExportDesc::Global(index) => index.index as usize,
                _ => unimplemented!("Only global exports supported"),
            }
        };

        let global = self
            .0
            .find_or_insert_std_section(|| payload::Global::default())
            .try_contents_mut()?
            .get_mut(global_index)
            .unwrap();

        match global.ty.value_type {
            wasmbin::types::ValueType::F64 => {
                let float = match value {
                    wasmer::Value::F64(f) => f,
                    _ => panic!(),
                };

                global.init = vec![Instruction::F64Const(FloatConst { value: float })]
            }
            wasmbin::types::ValueType::F32 => {
                let float = match value {
                    wasmer::Value::F32(f) => f,
                    _ => panic!(),
                };

                global.init = vec![Instruction::F32Const(FloatConst { value: float })]
            }
            wasmbin::types::ValueType::I64 => {
                let int = match value {
                    wasmer::Value::I64(i) => i,
                    _ => panic!(),
                };

                global.init = vec![Instruction::I64Const(int)]
            }
            wasmbin::types::ValueType::I32 => {
                let int = match value {
                    wasmer::Value::I32(i) => i,
                    _ => panic!(),
                };

                global.init = vec![Instruction::I32Const(int)]
            }
            _ => unimplemented!("Only global int & float exports supported"),
        };

        Ok(())
    }
}

impl ModuleAtRest {
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut buffer = Vec::new();
        self.0.encode_into(BufWriter::new(&mut buffer));

        buffer
    }

    fn remove_memory_imports(module: &mut wasmbin::Module) -> Result<Vec<Limits>, Errors> {
        let mut limits = Vec::new();

        let mut imports = module
            .find_or_insert_std_section(|| payload::Import::default())
            .try_contents_mut()?;

        let mut index = 0;
        while index < imports.len() {
            if let ImportDesc::Mem(memory_type) = &imports[index].desc {
                limits.push(memory_type.limits.clone());
                imports.remove(index);
            } else {
                index += 1;
            }
        }

        return Ok(limits);
    }

    fn add_memory_segments(module: &mut wasmbin::Module, limits: &Vec<Limits>) -> Result<usize, Errors> {
        if (limits.len() == 0) {
            return Ok(0);
        }

        let mut memories = module.find_or_insert_std_section(|| payload::Memory::default()).try_contents_mut()?;
        for limit in limits {
            memories.push(MemType { 
                limits: limit.clone()
            });
        }

        return Ok(limits.len());
    }

    fn add_memory_exports(module: &mut wasmbin::Module, imports_to_replace: usize) -> Result<usize, Errors> {
        let memory_count = {
            if let Some(memory_section) = module.find_std_section::<payload::Memory>() {
                memory_section.contents.try_contents()?.len()
            } else {
                0
            }
        };

        let exports = module
            .find_or_insert_std_section(|| payload::Export::default())
            .try_contents_mut()?;
        let exports_already_existing = exports.iter().map(|e| &e.desc).filter(|e| matches!(e, ExportDesc::Mem(_))).count();
        
        assert!(memory_count <= 1, "WASM only supports up to 1 memory, for now");
        
        if imports_to_replace > 0 || (memory_count - exports_already_existing) > 0 {
            exports.push(Export {
                desc: ExportDesc::Mem(MemId {
                    index: 0,
                }),
                name: format!("othismo_memory_{}", 0),
            });
            Ok(1)
        } else {
            Ok(0)
        }
    }

    fn export_imported_globals(mut module: wasmbin::Module) -> Result<wasmbin::Module, Errors> {
        let extracted_globals = ModuleAtRest::extract_imported_globals(&mut module)?;
        ModuleAtRest::export_extracted_globals(&mut module, extracted_globals)?;

        Ok(module)
    }

    fn export_extracted_globals(
        module: &mut wasmbin::Module,
        extracted_globals: Vec<Global>,
    ) -> Result<(), Errors> {
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

            existing_exports.insert(
                0,
                Export {
                    name,
                    desc: wasmbin::sections::ExportDesc::Global(id),
                },
            );
        }

        Ok(())
    }

    fn extract_imported_globals(
        module: &mut wasmbin::Module,
    ) -> Result<Vec<wasmbin::sections::Global>, Errors> {
        let import_section = module.find_or_insert_std_section(|| payload::Import::default());
        let mut imports = import_section.contents.try_contents_mut()?;
        let mut globals: Vec<wasmbin::sections::Global> = Vec::new();

        while let Some(index) = imports
            .iter()
            .position(|import| matches!(import.desc, ImportDesc::Global(_)))
        {
            let Import { path, desc } = &imports[index];

            globals.push(match desc {
                ImportDesc::Global(ty) => Global {
                    ty: ty.clone(),
                    init: match &ty.value_type {
                        wasmbin::types::ValueType::F64 => {
                            vec![Instruction::F64Const(FloatConst { value: 0f64 })]
                        }
                        wasmbin::types::ValueType::F32 => {
                            vec![Instruction::F32Const(FloatConst { value: 0f32 })]
                        }
                        wasmbin::types::ValueType::I64 => vec![Instruction::I64Const(0)],
                        wasmbin::types::ValueType::I32 => vec![Instruction::I32Const(0)],
                        wasmbin::types::ValueType::Ref(_) => {
                            Err(SolidarityError::UnsupportedModuleDefinition(
                                "reference_type_global".to_string(),
                            ))?
                        }
                        wasmbin::types::ValueType::V128 => Err(
                            SolidarityError::UnsupportedModuleDefinition("simd_global".to_string()),
                        )?,
                    },
                },
                _ => panic!(),
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
        InstanceAtRest(value.0)
    }
}

impl TryFrom<wasmbin::Module> for ModuleAtRest {
    type Error = Errors;

    fn try_from(mut module: wasmbin::Module) -> std::result::Result<Self, Self::Error> {
        module = ModuleAtRest::export_imported_globals(module)?;
        let limits = ModuleAtRest::remove_memory_imports(&mut module)?;
        ModuleAtRest::add_memory_segments(&mut module, &limits);
        ModuleAtRest::add_memory_exports(&mut module, limits.len())?;

        Ok(ModuleAtRest(module))
    }
}

impl Object {
    pub fn as_kind_str(&self) -> &'static str {
        match self {
            Object::Module(_) => "MODULE",
            Object::Instance(_) => "INSTANCE",
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
            Object::Instance(instance) => instance.to_bytes(),
        }
    }

    pub fn from_tuple(kind: &str, bytes: Vec<u8>) -> Result<Object> {
        match (kind) {
            "MODULE" => Ok(Object::Module(
                Module::decode_from(bytes.as_slice())?.try_into()?,
            )),
            "INSTANCE" => Ok(Object::Instance(
                Module::decode_from(bytes.as_slice())?.into(),
            )),
            _ => panic!(),
        }
    }

    pub fn new_module(bytes: &Vec<u8>) -> Result<Object> {
        Ok(Object::Module(
            Module::decode_from(bytes.as_slice())?.try_into()?,
        ))
    }

    pub fn new_instance(object: &Object) -> Result<Object> {
        let module = match object {
            Object::Module(inner_module) => inner_module,
            _ => panic!("Should only pass in a module here"),
        };

        let instance = module.0.clone();

        Ok(Object::Instance(instance.into()))
    }
}

pub enum LinkKind {
    InstanceOf,
}

pub struct Link {
    kind: LinkKind,
    from: Object,
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
            file: connection,
        })
    }

    fn create_in_memory() -> Result<Image> {
        let connection = Connection::open_in_memory()?;

        connection.execute_batch(include_str!("../sql_scripts/create_image_schema.sql"))?;

        Ok(Image {
            path_name: PathBuf::from("/in_memory"),
            file: connection,
        })
    }

    pub fn open<P: AsRef<Path>>(path: P) -> Result<Image> {
        Ok(Image {
            path_name: path.as_ref().to_path_buf(),
            file: Connection::open(path)?,
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
            |row| {
                Ok(Object::from_tuple(
                    row.get::<usize, String>(0).map(|name| name)?.as_str(),
                    row.get(1)?,
                ))
            },
        )?
    }

    pub fn remove_object(&mut self, name: &str) -> Result<()> {
        let object_key = self.get_object_key(name)?;

        let references: Option<i64> = self
            .file
            .query_row(
                r#"
            select
                count(*)
            from link L
            inner join object O on O.object_key = L.to_object_key
            inner join namespace NS on NS.object_key = O.object_key
            where NS.path = ?"#,
                params![name],
                |row| row.get(0),
            )
            .optional()?;

        if references.unwrap_or(0) > 0 {
            return Err(Errors::Solidarity(ObjectNotFree));
        }

        self.file.execute(
            r#"
        DELETE FROM namespace
        WHERE object_key = ?"#,
            params![object_key],
        )?;

        self.file.execute(
            r#"
        DELETE FROM object where object_key = ?
        "#,
            params![object_key],
        )?;

        Ok(())
    }

    pub fn object_exists(&self, name: &str) -> Result<bool> {
        let namespace_key: Option<i64> = self
            .file
            .query_row(
                "select count(*) from namespace where path = ?",
                params![name],
                |row| row.get(0),
            )
            .optional()?;

        return match namespace_key {
            Some(count) => Ok(count > 0),
            None => Ok(false),
        };
    }

    pub fn list_objects(&self, prefix: &str) -> Result<Vec<String>> {
        let mut statement = self.file.prepare(
            r#"
            SELECT
                NS.path
            FROM object O
            INNER JOIN namespace NS on NS.object_key = O.object_key
            WHERE path LIKE ? || '%'"#,
        )?;
        let mut rows = statement.query(params![prefix])?;

        let mut names: Vec<String> = Vec::new();

        while let Some(row) = rows.next()? {
            names.push(row.get(0)?)
        }

        Ok(names)
    }

    fn get_object_key(&self, name: &str) -> Result<i64> {
        let object_key: Option<i64> = self
            .file
            .query_row(
                "select object_key from namespace where path = ?",
                params![name],
                |row| row.get(0),
            )
            .optional()?;

        return match object_key {
            Some(key) => Ok(key),
            None => Err(Errors::Solidarity(ObjectDoesNotExist)),
        };
    }

    fn insert_object(&mut self, name: &str, kind: &str, bytes: &Vec<u8>) -> Result<()> {
        self.file.execute(
            "INSERT INTO object (kind, bytes) VALUES (?, ?)",
            params![kind, bytes],
        )?;
        let row_id = self.file.last_insert_rowid();

        self.upsert_name(name, row_id)?;

        Ok(())
    }

    fn upsert_name(&mut self, name: &str, object_key: i64) -> Result<()> {
        self.file.execute(
            "INSERT OR REPLACE INTO namespace (path, object_key) VALUES (?,?)",
            params![name, object_key],
        )?;

        Ok(())
    }
}

#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use rusqlite::{Connection, OptionalExtension, params};
use wasmer::{MemoryView, Store};
use crate::solidarity::SolidarityError::{ImageAlreadyExists, ObjectAlreadyExists};
use crate::solidarity::{Errors, Result};

pub enum Object {
    Module(wasmer::Module),
    Instance(wasmer::Instance)
}

impl Object {
    pub fn as_kind_str(&self) -> &'static str {
        match self {
            Object::Module(_) => "MODULE",
            Object::Instance(_) => "INSTANCE"
        }
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        match self {
            Object::Module(module) => module.serialize()
                .expect("Only valid modules should exist")
                .to_vec(),
            Object::Instance(instance) => instance.exports.get_memory("memory")
                .map(|memory| memory.view(&Store::default()).read())
                .or(Vec::new())
        }
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
    file: ImageFile,
    name_space: HashMap<String, Object>
}

impl Image {
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Image> {
        Ok(Image {
            file: ImageFile::open(path)?,
            name_space: HashMap::new(),
        })
    }
}

pub struct ImageFile {
    path_name: PathBuf,
    file: Connection,
}

impl ImageFile {
    pub fn create<P: AsRef<Path>>(path: P) -> Result<ImageFile> {
        if let Ok(_) = std::fs::metadata(&path) {
            Err(ImageAlreadyExists)?
        }

        let connection = Connection::open(path.as_ref())?;

        connection.execute_batch(include_str!("../sql_scripts/create_image_schema.sql"))?;

        Ok(ImageFile {
            path_name: path.as_ref().to_path_buf(),
            file: connection
        })
    }

    fn create_in_memory() -> Result<ImageFile> {
        let connection = Connection::open_in_memory()?;

        connection.execute_batch(include_str!("../sql_scripts/create_image_schema.sql"))?;

        Ok(ImageFile {
            path_name: PathBuf::from("/in_memory"),
            file: connection
        })
    }

    pub fn open<P: AsRef<Path>>(path: P) -> Result<ImageFile> {
        Ok(ImageFile {
            path_name: path.as_ref().to_path_buf(),
            file: Connection::open(path)?
        })
    }

    pub fn import_object(&mut self, name: &str, object: Object) -> Result<()> {
        if self.object_exists(name)? {
            return Err(Errors::Solidarity(ObjectAlreadyExists));
        }

        self.insert_object(name, "", ob)
        Ok(())
    }

    pub fn remove_object(&mut self, name: &str) -> Result<()> {
        // basically garbage collect
        self.file.execute(r#"
        DELETE FROM namespace
        WHERE path = ?"#, params![name])?;

        self.file.execute(r#"
        DELETE FROM instance
        WHERE module_key NOT IN (
            SELECT M.module_key
            FROM module M
            INNER JOIN namespace N ON N.module_key = M.module_key
            WHERE path = ?
        ) or instance_key NOT IN (
            SELECT M.module_key
            FROM module M
            INNER JOIN namespace N ON N.module_key = M.module_key
            WHERE path = ?
        )"#, params![name, name])?;

        self.file.execute(r#"
        DELETE FROM module
        WHERE module_key NOT IN (
            SELECT M.module_key
            FROM module M
            INNER JOIN namespace N ON N.module_key = M.module_key
            WHERE path = ?
        )"#, params![name])?;

        Ok(())
    }

    pub fn object_exists(&self, name: &str) -> Result<bool> {
        let namespace_key: Option<i64> = self.file.query_row(
            "select count(*) from namespace where path = ?",
            params![name],
            |row| row.get(0)
        ).optional()?;

        return Ok(namespace_key.is_some());
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

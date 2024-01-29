use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use rusqlite::{Connection, params};
use wasmer::{Module, Store};
use crate::solidarity::SolidarityError::{ImageAlreadyExists, ModuleAlreadyExists};
use crate::solidarity::{Errors, Result};

#[derive(Debug)]
pub struct Name(String);

impl Name {
    pub fn new(name: &str) -> Name {
        Name(name.to_string())
    }
}

impl PartialEq<Self> for Name {
    fn eq(&self, other: &Self) -> bool {
        self.0.eq(&other.0)
    }
}

impl Eq for Name {

}

impl Hash for Name {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.hash(state)
    }
}

pub enum Object {
    Module,
    Instance(wasmer::Instance)
}
pub struct Image {
    file: ImageFile,
    name_space: HashMap<Name, Object>
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

    pub fn import_module<P: AsRef<Path>>(&mut self, file_path: P, namespace_path: &str) -> Result<()> {
        let wasm_bytes = std::fs::read(file_path.as_ref().canonicalize()?)?;

        self.import_module_bytes(namespace_path, &wasm_bytes)
    }

    fn import_module_bytes(&mut self, namespace_path: &str, wasm_bytes: &Vec<u8>) -> Result<()> {
        if self.list_modules()?.contains(&Name(namespace_path.to_string())) {
            return Err(Errors::Solidarity(ModuleAlreadyExists))
        }

        Module::new(&Store::default(), &wasm_bytes)?;

        self.file.execute("INSERT INTO module (wasm) VALUES (?)", params![wasm_bytes])?;
        let row_id = self.file.last_insert_rowid();

        self.upsert_name(&Name::new(namespace_path), Some(row_id), None)?;

        Ok(())
    }

    pub fn remove_module<P: AsRef<Path>>(&mut self, name: Name) -> Result<()> {
        self.file.execute(r#"
        DELETE FROM module M
        inner join namespace N on N.module_key = M.module_key
        WHERE path = ?"#, params![name.0])?;

        self.file.execute(r#"
        DELETE FROM namespace
        WHERE path = ?"#, params![name.0])?;

        Ok(())
    }

    pub fn list_modules(&self) -> Result<Vec<Name>> {
        let mut statement = self.file.prepare("SELECT path FROM namespace WHERE module_key IS NOT NULL")?;
        let module_names = statement.query_map([], |row| {
            Ok(Name(row.get(0)?))
        })?;

        let mut rows = Vec::new();
        for row in module_names {
            rows.push(row?);
        }

        return Ok(rows)
    }

    fn upsert_name(&mut self, name: &Name, module_key: Option<i64>, instance_key: Option<i64>) -> Result<()> {
        self.file.execute(
            "INSERT OR REPLACE INTO namespace (path, module_key, instance_key) VALUES (?,?,?)",
            params![
            name.0,
            module_key,
            instance_key,
        ])?;

        return Ok(());
    }
}

#[cfg(test)]
mod tests;

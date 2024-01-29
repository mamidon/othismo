use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use rusqlite::{Connection, OptionalExtension, params};
use wasmer::{Module, Store};
use crate::solidarity::SolidarityError::{ImageAlreadyExists, ModuleAlreadyExists};
use crate::solidarity::{Errors, Result};

pub enum Object {
    Module,
    Instance(wasmer::Instance)
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

    pub fn import_module<P: AsRef<Path>>(&mut self, file_path: P, namespace_path: &str) -> Result<()> {
        let wasm_bytes = std::fs::read(file_path.as_ref().canonicalize()?)?;

        self.import_module_bytes(namespace_path, &wasm_bytes)
    }

    fn import_module_bytes(&mut self, namespace_path: &str, wasm_bytes: &Vec<u8>) -> Result<()> {
        if self.list_objects(Some(namespace_path))?.len() > 0 {
            return Err(Errors::Solidarity(ModuleAlreadyExists))
        }

        Module::new(&Store::default(), &wasm_bytes)?;

        self.file.execute("INSERT INTO module (wasm) VALUES (?)", params![wasm_bytes])?;
        let row_id = self.file.last_insert_rowid();

        self.upsert_name(namespace_path, Some(row_id), None)?;

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

    pub fn list_objects(&self, prefix: Option<&str>) -> Result<Vec<String>> {
        match prefix {
            None => {
                let mut statement = self.file.prepare("SELECT path FROM namespace")?;
                let module_names = statement.query_map([], |row| {
                    Ok(row.get(0)?)
                })?;

                let mut rows = Vec::new();
                for row in module_names {
                    rows.push(row?);
                }

                Ok(rows)
            },
            Some(prefix) => {
                let mut statement = self.file.prepare("SELECT path FROM namespace WHERE path LIKE '?%'")?;
                let module_names = statement.query_map([], |row| {
                    Ok(row.get(0)?)
                })?;

                let mut rows = Vec::new();
                for row in module_names {
                    rows.push(row?);
                }

                Ok(rows)
            }
        }
    }

    fn upsert_name(&mut self, name: &str, module_key: Option<i64>, instance_key: Option<i64>) -> Result<()> {
        self.file.execute(
            "INSERT OR REPLACE INTO namespace (path, module_key, instance_key) VALUES (?,?,?)",
            params![
            name,
            module_key,
            instance_key,
        ])?;

        return Ok(());
    }
}

#[cfg(test)]
mod tests;

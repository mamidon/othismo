use clap::{Parser, Subcommand};
use crate::solidarity::image::ImageFile;

mod solidarity {
    use std::result;
    use wasmer::CompileError;
    use crate::solidarity;

    #[derive(Debug)]
    pub enum Error {
        ImageAlreadyExists
    }

    pub type Result<T, E=Errors> = result::Result<T,E>;
    #[derive(Debug)]
    pub enum Errors {
        Solidarity(Error),
        Rusqlite(rusqlite::Error),
        Io(std::io::Error),
        Wasmer(wasmer::CompileError)
    }

    impl From<rusqlite::Error> for Errors {
        fn from(value: rusqlite::Error) -> Self {
            Errors::Rusqlite(value)
        }
    }

    impl From<std::io::Error> for Errors {
        fn from(value: std::io::Error) -> Self {
            Errors::Io(value)
        }
    }

    impl From<Error> for Errors {
        fn from(value: solidarity::Error) -> Self {
            Errors::Solidarity(value)
        }
    }

    impl From<CompileError> for Errors {
        fn from(value: CompileError) -> Self {
            Errors::Wasmer(value)
        }
    }


    pub mod image {
        use std::path::{Path, PathBuf};
        use rusqlite::{Connection, params};
        use wasmer::{Module, Store};
        use crate::solidarity::Error::ImageAlreadyExists;
        use crate::solidarity::{Result, solidarity};

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

                connection.execute(include_str!("./sql_scripts/create_image_schema.sql"), ())?;

                Ok(ImageFile {
                    path_name: path.as_ref().to_path_buf(),
                    file: connection
                })
            }

            pub fn open<P: AsRef<Path>>(path: P) -> Result<ImageFile> {
                Ok(ImageFile {
                    path_name: path.as_ref().to_path_buf(),
                    file: Connection::open(path)?
                })
            }

            pub fn import_module<P: AsRef<Path>>(mut self, file_path: P, namespace_path: &str) -> Result<()> {
                let wasm_bytes = std::fs::read(file_path.as_ref().canonicalize()?)?;

                Module::new(&Store::default(), &wasm_bytes)?;

                self.file.execute("INSERT INTO module (path, wasm) VALUES (?,?)", params![
                    namespace_path,
                    wasm_bytes
                ])?;

                Ok(())
            }
        }
    }
}

#[derive(Parser)]
struct CliArguments {
    #[arg()]
    image_name: Option<String>,
    #[command(subcommand)]
    sub_command: Option<SubCommands>,
}

#[derive(Subcommand)]
enum SubCommands {
    NewImage {
        #[arg()]
        image_name: String
    },
    ImportModule {
        #[arg()]
        module_name: String,
    },
    RemoveModule {
        #[arg()]
        module_name: String
    },
    InstantiateInstance {
        #[arg()]
        module_name: String,
        #[arg()]
        instance_name: String,
    },
    DeleteInstance {
        #[arg()]
        module_name: String,
        #[arg()]
        instance_name: String,
    },
    SendMessage {},
}

fn main() -> solidarity::Result<()> {
    let command = CliArguments::parse();

    if let Some(image_name) = command.image_name {
        let mut image = ImageFile::open(image_name.clone() + ".simg")?;

        match command.sub_command {
            Some(SubCommands::ImportModule {
                module_name
            }) => {
                image.import_module(module_name, "foo")?;
            }
            Some(SubCommands::RemoveModule {
                module_name
            }) => {
                unimplemented!()
            }
            Some(SubCommands::InstantiateInstance {
                module_name,
                instance_name
            }) => {
                unimplemented!()
            }
            Some(SubCommands::DeleteInstance {
                module_name,
                instance_name
            }) => {
                unimplemented!()
            }
            Some(SubCommands::SendMessage {}) => {
                unimplemented!()
            },
            Some(SubCommands::NewImage {
                image_name
            }) => {
                eprintln!("Specify the image name _after_ the new-image command");
            },
            None => {eprintln!("No sub command specified");}
        }
    } else {
        match command.sub_command {
            Some(SubCommands::NewImage {
                image_name
                 }) => {
                let image_path = image_name.clone() + ".simg";
                let image = solidarity::image::ImageFile::create(image_path)?;

                println!("Image created");
            },
            _ => {
                eprintln!("This sub-command needs the relevant image name specified before it");
            }
        }
    };

    Ok(())
}

// super WIP & exploratory.. lots of unused stuff
#![allow(unused)]

use clap::{Parser, Subcommand};
use crate::solidarity::image::{ImageFile, Object};

mod solidarity;
mod prototype;

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
        instance_name: String,
    },
    SendMessage {},
    ListObjects {},
    ParseModule {
        #[arg()]
        module_name: String
    }
}

fn main() -> solidarity::Result<()> {
    let command = CliArguments::parse();

    if let Some(image_name) = command.image_name {
        let mut image = ImageFile::open(image_name.clone() + ".simg")?;

        match command.sub_command {
            Some(SubCommands::ImportModule {
                module_name
            }) => {
                let module = std::fs::read(&module_name)?;
                let module_namespace_name = &std::path::Path::new(&module_name).file_stem().unwrap().to_str().unwrap();
                image.import_object(&module_namespace_name, Object::new_module(&module)?)?;
            }
            Some(SubCommands::RemoveModule {
                module_name: _
            }) => {
                unimplemented!()
            }
            Some(SubCommands::InstantiateInstance {
                module_name,
                instance_name
            }) => {
                let object = image.get_object(&module_name)?;

                let module = match object {
                    Object::Instance(obj) => panic!("Please specify a module"),
                    Object::Module(module) => module
                };

                image.import_object(&instance_name, Object::Instance(module))?;
            }
            Some(SubCommands::DeleteInstance {
                instance_name: _
            }) => {
                unimplemented!()
            }
            Some(SubCommands::SendMessage {}) => {
                unimplemented!()
            },
            Some(SubCommands::NewImage {
                image_name: _
            }) => {
                eprintln!("Specify the image name _after_ the new-image command");
            },
            Some(SubCommands::ListObjects {  }) => {
                for name in image.list_objects("")? {
                    println!("{}", name);
                }
            }
            Some(SubCommands::ParseModule { module_name }) => eprintln!("Not a sub command"),
            None => {eprintln!("No sub command specified");}
        }
    } else {
        match command.sub_command {
            Some(SubCommands::NewImage {
                image_name
                 }) => {
                let image_path = image_name.clone() + ".simg";
                solidarity::image::ImageFile::create(image_path)?;

                println!("Image created");
            },
            Some(SubCommands::ParseModule { module_name }) => {
                prototype::parse_module_2(module_name)?
            },
            _ => {
                eprintln!("This sub-command needs the relevant image name specified before it");
            }
        }
    };

    Ok(())
}

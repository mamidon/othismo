// super WIP & exploratory.. lots of unused stuff
#![allow(unused)]

use bson::bson;
use std::time::Duration;

use crate::othismo::image::{Image, Object};
use crate::othismo::Message;
use bson::doc;
use clap::{Parser, Subcommand};
use othismo::executors::{ConsoleExecutor, EchoExecutor};
use othismo::namespace::Namespace;
use tokio::time::sleep;

mod othismo;

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
        image_name: String,
    },
    ImportModule {
        #[arg()]
        module_name: String,
    },
    RemoveModule {
        #[arg()]
        module_name: String,
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
    SendMessage {
        #[arg()]
        instance_name: String,
    },
    ListObjects {},
}

#[tokio::main]
async fn main() -> othismo::Result<()> {
    let command = CliArguments::parse();

    if let Some(image_name) = command.image_name {
        let mut image = Image::open(image_name.clone() + ".simg")?;

        match command.sub_command {
            Some(SubCommands::ImportModule { module_name }) => {
                let module = std::fs::read(&module_name)?;
                let module_namespace_name = &std::path::Path::new(&module_name)
                    .file_stem()
                    .unwrap()
                    .to_str()
                    .unwrap();
                image.import_object(&module_namespace_name, Object::new_module(&module)?)?;
            }
            Some(SubCommands::RemoveModule { module_name }) => {
                image.remove_object(&module_name)?;
            }
            Some(SubCommands::InstantiateInstance {
                module_name,
                instance_name,
            }) => {
                let object = image.get_object(&module_name)?;

                let module = match object {
                    Object::Instance(obj) => panic!("Please specify a module"),
                    Object::Module(module) => module,
                };

                image.import_object(&instance_name, Object::Instance(module.into()))?;
            }
            Some(SubCommands::DeleteInstance { instance_name }) => {
                image.remove_object(&instance_name)?;
            }
            Some(SubCommands::SendMessage { instance_name }) => {
                let mut namespace: Namespace = image.into();
                let document = doc! { "othismo": { "send_to": "foobar" } };
                let mut bytes: Vec<u8> = Vec::new();
                document.to_writer(&mut bytes).unwrap();
                namespace.send_message(&instance_name, Message::new(bytes));
                namespace.wait_for_idleness(Duration::from_secs(30)).await;
            }
            Some(SubCommands::NewImage { image_name: _ }) => {
                eprintln!("Specify the image name _after_ the new-image command");
            }
            Some(SubCommands::ListObjects {}) => {
                for name in image.list_objects("")? {
                    println!("{}", name);
                }
            }
            None => {
                eprintln!("No sub command specified");
            }
        }
    } else {
        match command.sub_command {
            Some(SubCommands::NewImage { image_name }) => {
                let image_path = image_name.clone() + ".simg";
                othismo::image::Image::create(image_path)?;

                println!("Image created");
            }
            _ => {
                eprintln!("This sub-command needs the relevant image name specified before it");
            }
        }
    };

    Ok(())
}

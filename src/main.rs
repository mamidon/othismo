use clap::{Parser, Subcommand};
use rusqlite::{Connection, OpenFlags, Result};

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

fn main() -> Result<()> {
    let command = CliArguments::parse();

    if let Some(image_name) = command.image_name {
        match command.sub_command {
            Some(SubCommands::ImportModule {
                module_name
            }) => { unimplemented!() }
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
                if let Ok(_) = std::fs::metadata(&image_path) {
                    eprintln!("There is already an image file at {}", image_path);
                    return Ok(());
                }

                let connection = Connection::open(image_name.clone() + ".simg")?;

                connection.execute(include_str!("./sql_scripts/create_image_schema.sql"), ())?;
                println!("Image {} created", image_name);
            },
            _ => {
                eprintln!("This sub-command needs the relevant image name specified before it");
            }
        }
    };

    Ok(())
}

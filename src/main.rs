use clap::{Parser, Subcommand};

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

fn main() {
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
            None => eprintln!("No sub command specified")
        }
    } else {
        match command.sub_command {
            Some(SubCommands::NewImage {
                image_name
                 }) => {
                println!("new-image {}", image_name);
            },
            _ => {
                eprintln!("This sub-command needs the relevant image name specified before it");
            }
        }
    }

}

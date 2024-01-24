use clap::{Parser, Subcommand};

#[derive(Parser)]
struct CliArguments {
    #[command(subcommand)]
    sub_command: Option<SubCommands>
}
#[derive(Subcommand)]
enum SubCommands {
    NewImage {
        #[arg()]
        image_name: String
    }
}

fn main() {
    let command = CliArguments::parse();

    if let Some(sub_command) = command.sub_command {
        match sub_command {
            SubCommands::NewImage { image_name } => println!("new-image {}", image_name)
        }
    }
}

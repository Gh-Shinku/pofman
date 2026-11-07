mod config;
mod display;
mod manager;
mod ui;

use anyhow::Result;
use clap::{Parser, Subcommand};
use config::Paths;
use manager::Manager;

#[derive(Parser)]
#[command(version, about = "A lightweight SSH port forwarding manager")]
struct Cli {
    #[command(subcommand)]
    command: Option<Action>,
}

#[derive(Subcommand)]
enum Action {
    /// List configured profiles and running processes
    List,
    /// Start a profile in the background
    Start { name: String },
    /// Stop a managed profile
    Stop { name: String },
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    let manager = Manager::new(Paths::discover()?)?;
    match cli.command {
        Some(Action::Start { name }) => println!("{}", manager.start(&name)?),
        Some(Action::Stop { name }) => println!("{}", manager.stop(&name)?),
        Some(Action::List) => display::list(&manager.rows()?)?,
        None => ui::run(&manager)?,
    }
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("pofman: {error:#}");
        std::process::exit(1);
    }
}

mod agent;
mod config;
mod llm;
mod prompt;
mod skills;
mod storage;
mod telegram;
mod tools;

use anyhow::Result;
use clap::Parser;
use std::io::{self, Write};

#[derive(Parser, Debug)]
#[command(name = "hermes-rs")]
#[command(about = "Hermes Agent implementation in Rust — Nous Parity", long_about = None)]
struct Args {
    /// Hermes profile to load (e.g. redmess or syadagentic)
    #[arg(short, long, default_value = "redmess")]
    profile: String,

    /// Override model to use
    #[arg(short, long)]
    model: Option<String>,

    /// Run as Telegram Gateway long-polling service
    #[arg(short, long)]
    gateway: bool,

    /// Interactive chat / REPL mode
    #[arg(short, long)]
    chat: bool,

    /// Initial task / prompt to execute
    #[arg(trailing_var_arg = true)]
    prompt: Vec<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    println!("==================================================");
    println!("🦀 Hermes Agent (Rust Edition) v0.2.0 [Nous Parity]");
    println!("Loading profile: '{}'...", args.profile);

    let profile = match config::Profile::load(&args.profile) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("❌ Failed to load profile '{}': {}", args.profile, e);
            std::process::exit(1);
        }
    };

    println!("Profile Base URL : {}", profile.base_url);
    println!("Profile Model    : {}", profile.model);
    println!("Profile Path     : {:?}", profile.path);
    println!("Proxy            : {:?}", profile.proxy);
    println!("==================================================");

    // If running as Telegram Gateway
    if args.gateway {
        let gateway = telegram::TelegramGateway::new(profile)?;
        gateway.run().await?;
        return Ok(());
    }

    let mut agent = agent::Agent::new(profile, args.model, None)?;

    let initial_prompt = if !args.prompt.is_empty() {
        Some(args.prompt.join(" "))
    } else {
        None
    };

    if let Some(prompt) = initial_prompt {
        agent.run_turn(&prompt, 15).await?;
        if !args.chat {
            return Ok(());
        }
    }

    if args.chat {
        println!("\n💬 Entering interactive chat mode. Type 'exit' to quit.\n");
        loop {
            print!("hermes-rs> ");
            io::stdout().flush()?;
            let mut input = String::new();
            if io::stdin().read_line(&mut input)? == 0 {
                break;
            }
            let input = input.trim();
            if input.is_empty() {
                continue;
            }
            if input.eq_ignore_ascii_case("exit") || input.eq_ignore_ascii_case("quit") {
                println!("Bye!");
                break;
            }

            if let Err(e) = agent.run_turn(input, 15).await {
                eprintln!("❌ Error during turn: {}", e);
            }
        }
    } else if args.prompt.is_empty() {
        println!("No prompt provided. Run with --chat, --gateway, or provide a prompt.");
        println!("Example: hermes-rs --profile redmess --gateway");
    }

    Ok(())
}

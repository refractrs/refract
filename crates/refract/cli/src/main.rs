mod dev;
mod process;
mod project;
mod scaffold;
mod server;
mod watch;

use clap::{Parser, Subcommand, ValueEnum};
use std::path::PathBuf;
use std::sync::{atomic::AtomicBool, Arc};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[derive(Parser)]
#[command(
    name = "refract",
    version,
    about = "Create and develop Refract applications"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Create a counter app with shared web and desktop views.
    New(scaffold::NewOptions),
    /// Watch, rebuild, and run an application (reloads reset application state).
    Dev(DevOptions),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum Platform {
    Web,
    Desktop,
}

#[derive(clap::Args)]
struct DevOptions {
    #[arg(long, value_enum, default_value = "web")]
    platform: Platform,
    /// Select an application in a Cargo workspace.
    #[arg(short, long)]
    package: Option<String>,
    #[arg(long)]
    manifest_path: Option<PathBuf>,
    /// Desktop binary; inferred when the app has one binary.
    #[arg(long)]
    bin: Option<String>,
    /// Additional Cargo features (comma-separated).
    #[arg(long, value_delimiter = ',')]
    features: Vec<String>,
    #[arg(long)]
    no_default_features: bool,
    /// Serve on localhost at this port (web only).
    #[arg(long, default_value_t = 4173)]
    port: u16,
    /// Open the browser when the development server starts.
    #[arg(long)]
    open: bool,
    /// Refuse network access, including tool installation.
    #[arg(long)]
    offline: bool,
    /// Require the existing Cargo.lock to remain unchanged.
    #[arg(long)]
    locked: bool,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    let stopped = Arc::new(AtomicBool::new(false));
    let signal = stopped.clone();
    ctrlc::set_handler(move || signal.store(true, std::sync::atomic::Ordering::Relaxed))?;
    match cli.command {
        Commands::New(options) => scaffold::create(options),
        Commands::Dev(options) => dev::run(options, stopped),
    }
}

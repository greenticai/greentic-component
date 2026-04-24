use anyhow::Result;
use clap::Args;
use std::path::{Path, PathBuf};

use super::human;
use super::reader;

#[derive(Debug, Args)]
pub struct InfoArgs {
    /// Path to a compiled component .wasm file.
    #[arg(value_name = "PATH")]
    pub path: PathBuf,

    /// Emit the info report as JSON.
    #[arg(long, default_value_t = false)]
    pub json: bool,
}

pub fn run(args: &InfoArgs) -> Result<()> {
    validate_path(&args.path)?;
    let report = match reader::read(&args.path) {
        Ok(r) => r,
        Err(e) => {
            eprintln!(
                "{}: not a valid wasm32-wasip2 component: {e}",
                args.path.display()
            );
            std::process::exit(5);
        }
    };
    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print!("{}", human::render(&report));
    }
    Ok(())
}

fn validate_path(path: &Path) -> Result<()> {
    if !path.exists() {
        eprintln!("{}: not a .wasm file (file not found)", path.display());
        std::process::exit(2);
    }
    if path.is_dir() {
        eprintln!("{}: not a .wasm file (is a directory)", path.display());
        std::process::exit(2);
    }
    if path.extension().and_then(|s| s.to_str()) != Some("wasm") {
        eprintln!("{}: not a .wasm file (wrong extension)", path.display());
        std::process::exit(2);
    }
    Ok(())
}

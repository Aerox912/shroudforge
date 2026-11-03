use anyhow::{ensure, Context, Result};
use hdmap_prepared_generator::{prepare, Request};
use std::{fs, path::Path};

fn main() {
    if let Err(error) = run() {
        eprintln!("{error:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    ensure!(args.len() == 2 && args[0] == "prepare", "usage: prepare REQUEST");
    let path = Path::new(&args[1]);
    hdmap_prepared_generator::ordinary(path, false)?;
    ensure!(fs::metadata(path)?.len() <= 8 * 1024 * 1024, "oversized request");
    let request: Request = serde_json::from_slice(&fs::read(path)?).context("prepared request")?;
    // No file logger, publish, recovery, cleanup, or game/native launch path.
    println!("{}", serde_json::to_string(&prepare(&request)?)?);
    Ok(())
}

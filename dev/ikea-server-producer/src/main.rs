use anyhow::{Result, ensure};
use std::{fs, path::Path};

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    ensure!(args.len() == 3 && args[0] == "check-schema", "usage: check-schema EVIDENCE_ROOT INPUT.json");
    let raw = fs::read(&args[2])?;
    ensure!(raw.len() <= 65536, "schema input bound");
    let input = serde_json::from_slice(&raw)?;
    let registry = shroudforge_api::admit_ikea_server_registry(Path::new(&args[1]), &input)?;
    println!("{}", serde_json::to_string(registry.provenance())?);
    Ok(())
}

use anyhow::{Result, ensure};
use std::{fs, path::Path};
mod approval;
mod plan;
mod prepare;

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.as_slice() == ["plan-staged"] {
        println!("{}", serde_json::to_string(&plan::staged()?)?);
        return Ok(());
    }
    if args.first().is_some_and(|s| s == "prepare-once") {
        ensure!(
            args.len() == 3,
            "usage: prepare-once SELECTION_BYTES SELECTION_SHA256"
        );
        let approved = approval::admit(args[1].parse()?, &args[2])?;
        match prepare::prepare(&approved.request) {
            Ok(result) => println!("{}", serde_json::to_string(&result)?),
            Err(error) => {
                prepare::write_new(
                    &Path::new(prepare::PRIVATE).join("operation-r1/FAILED.json"),
                    &serde_json::json!({"status":"SERVER_ASSET_PREPARATION_FAILED_SPENT","selectionSha256":approved.selection_sha256,"error":format!("{error:#}"),"replayAllowed":false}),
                )?;
                return Err(error);
            }
        }
        return Ok(());
    }
    ensure!(
        args.len() == 3 && args[0] == "check-schema",
        "usage: check-schema EVIDENCE_ROOT INPUT.json"
    );
    let raw = fs::read(&args[2])?;
    ensure!(raw.len() <= 65536, "schema input bound");
    let input = serde_json::from_slice(&raw)?;
    let registry = shroudforge_api::admit_ikea_server_registry(Path::new(&args[1]), &input)?;
    println!("{}", serde_json::to_string(registry.provenance())?);
    Ok(())
}

use anyhow::{Result, bail};
use std::{env, path::PathBuf};

fn hash_arg(value: &str, label: &str) -> Result<()> {
    anyhow::ensure!(
        value.len() == 64
            && value
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
        "{label} must be a lowercase SHA-256 digest"
    );
    Ok(())
}
fn read_e(path: &PathBuf, limit: u64) -> Result<Vec<u8>> {
    sf_full_candidate_assembler::read_input_file(path, limit)
}
fn run() -> Result<()> {
    let args: Vec<String> = env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("prepare-template") if args.len() == 4 => {
            let path = PathBuf::from(&args[2]);
            hash_arg(&args[3], "manifest digest")?;
            let bytes = read_e(&path, 64 * 1024 * 1024)?;
            let template = sf_full_candidate_assembler::prepare_template(&bytes, &args[3])?;
            println!("{} {}", template.status, template.candidate_id);
            Ok(())
        }
        Some("finalize-policy") if args.len() == 7 => {
            let template = PathBuf::from(&args[2]);
            let plan = PathBuf::from(&args[4]);
            let receipt = PathBuf::from(&args[5]);
            hash_arg(&args[3], "template digest")?;
            hash_arg(&args[6], "XHL receipt digest")?;
            let finalized = sf_full_candidate_assembler::finalize_policy(
                &template, &args[3], &plan, &receipt, &args[6],
            )?;
            println!("{} {}", finalized.status, finalized.policy_sha256);
            Ok(())
        }
        _ => {
            bail!(
                "usage: sf-full-candidate-assembler prepare-template <E-manifest.json> <manifest-sha256> | finalize-policy <E-template.json> <template-sha256> <E-approved-plan.json> <E-prepared-receipt.json> <receipt-sha256>"
            )
        }
    }
}
fn main() {
    if let Err(error) = run() {
        eprintln!("candidate assembly refused: {error:#}");
        std::process::exit(2);
    }
}

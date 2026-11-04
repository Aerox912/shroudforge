use anyhow::{Context, Result, ensure};
use hdmap_prepared_generator::{Request, prepare};
use std::{fs, path::Path};

fn main() {
    if let Err(error) = run() {
        eprintln!("{error:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.first().is_some_and(|arg| arg == "prepare-effective") {
        ensure!(args.len() == 3, "usage: prepare-effective REQUEST APPROVAL");
        let request = hdmap_prepared_generator::effective::read_protocol_json(Path::new(&args[1]))
            .context("effective request")?;
        let approval = hdmap_prepared_generator::effective::read_protocol_json(Path::new(&args[2]))
            .context("effective use approval")?;
        let admitted =
            hdmap_prepared_generator::effective::admit_effective_use(&request, approval)?;
        let output = hdmap_prepared_generator::effective::prepare_effective(&request, admitted)?;
        println!("{}", serde_json::to_string(&output.write_receipt()?)?);
        return Ok(());
    }
    ensure!(
        args.len() == 2 && args[0] == "prepare",
        "usage: prepare REQUEST"
    );
    let path = Path::new(&args[1]);
    hdmap_prepared_generator::ordinary(path, false)?;
    ensure!(
        fs::metadata(path)?.len() <= 8 * 1024 * 1024,
        "oversized request"
    );
    let request: Request = serde_json::from_slice(&fs::read(path)?).context("prepared request")?;
    // No file logger, publish, recovery, cleanup, or game/native launch path.
    println!("{}", serde_json::to_string(&prepare(&request)?)?);
    Ok(())
}

fn main() {
    std::panic::set_hook(Box::new(|_| {
        eprintln!(
            "{{\"status\":\"refused\",\"kind\":\"internal_fault\",\"rawErrorIncluded\":false}}"
        );
        std::process::exit(2);
    }));
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let result = (|| -> anyhow::Result<serde_json::Value> {
        if args == [std::ffi::OsString::from("--check-containment")] {
            ikea_server_schema::diagnostics::set(1);
            ikea_server_schema::containment::install()?;
            return Ok(
                serde_json::json!({"status":"containment_installed","sourceFilesOpened":0,"actualAcquisition":false}),
            );
        }
        anyhow::ensure!(
            args == [
                std::ffi::OsString::from("--qualify-ikea-server-schema"),
                std::ffi::OsString::from("--authorize-fixed-metadata")
            ],
            "arguments"
        );
        ikea_server_schema::diagnostics::set(1);
        ikea_server_schema::containment::install()?;
        ikea_server_schema::acquire::run()
    })();
    match result {
        Ok(v) => println!("{v}"),
        Err(_) => {
            eprintln!("{}", ikea_server_schema::diagnostics::refusal());
            std::process::exit(2);
        }
    }
}

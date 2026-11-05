use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

fn main() -> std::io::Result<()> {
    println!("cargo:rerun-if-env-changed=HD_PREPARED_SOURCE_CLOSURE");
    println!("cargo:rerun-if-env-changed=SF_FULL_PREPARED_SOURCE_CLOSURE");
    validate_prepared_source_pair(
        env::var("HD_PREPARED_SOURCE_CLOSURE").ok().as_deref(),
        env::var("SF_FULL_PREPARED_SOURCE_CLOSURE").ok().as_deref(),
    )?;
    generate_definitions_file()?;

    // does not change unless definitions change, so this may need to be changed in the future
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();
    println!("cargo:rustc-env=BUILD_ID={now}");

    Ok(())
}

fn validate_prepared_source_pair(legacy: Option<&str>, full: Option<&str>) -> std::io::Result<()> {
    if full.is_some() && full == legacy {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "HD_PREPARED_SOURCE_CLOSURE and SF_FULL_PREPARED_SOURCE_CLOSURE must identify distinct producers; omit the unused historical HD binding for a full-runtime-only build",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::validate_prepared_source_pair;

    #[test]
    fn rejects_a_full_producer_registered_as_legacy() {
        assert!(validate_prepared_source_pair(Some("producer"), Some("producer")).is_err());
    }

    #[test]
    fn accepts_distinct_or_unselected_prepared_modes() {
        for (legacy, full) in [(None, None), (Some("legacy"), None), (None, Some("full")), (Some("legacy"), Some("full"))] {
            validate_prepared_source_pair(legacy, full).unwrap();
        }
    }
}

fn generate_definitions_file() -> std::io::Result<()> {
    let definitions_dir = Path::new("lua");

    println!("cargo:rerun-if-changed={}", definitions_dir.display());

    let mut lua_files = Vec::new();

    collect_lua_files(definitions_dir, &mut lua_files)?;

    lua_files.sort();

    for file in &lua_files {
        println!("cargo:rerun-if-changed={}", file.display());
    }

    let mut combined = String::new();

    combined.push_str("--- @meta\n");

    for file in lua_files {
        let content = fs::read_to_string(&file)?;

        if let Some(pos) = content.find('\n') {
            if content[..pos].trim().contains("@meta") {
                combined.push_str(&content[pos + 1..]);
            } else {
                combined.push_str(&content);
            }
        } else {
            combined.push_str(&content);
        }
    }

    let out_dir = env::var("OUT_DIR").unwrap();
    let out_path = Path::new(&out_dir).join("definitions.lua");

    let need_write = match fs::read_to_string(&out_path) {
        Ok(existing) => existing != combined,
        Err(_) => true,
    };

    if need_write {
        std::fs::write(&out_path, combined.as_bytes())?;
    }

    Ok(())
}

fn collect_lua_files(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let p = entry?.path();

        if p.is_dir() {
            collect_lua_files(&p, out)?;
        } else if p.extension().and_then(|s| s.to_str()) == Some("lua")
            && p.components().any(|part| part.as_os_str() == "definitions")
        {
            out.push(p);
        }
    }
    Ok(())
}

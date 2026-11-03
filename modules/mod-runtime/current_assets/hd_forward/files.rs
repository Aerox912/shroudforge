use super::contract::*;
use anyhow::{Context, Result, ensure};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::Read,
    path::{Path, PathBuf},
};

pub fn ordinary(path: &Path, directory: bool) -> Result<()> {
    let m = fs::symlink_metadata(path)?;
    ensure!(
        !m.file_type().is_symlink() && if directory { m.is_dir() } else { m.is_file() },
        "unexpected file kind: {}",
        path.display()
    );
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        ensure!(m.file_attributes() & 0x400 == 0, "reparse path refused");
    }
    Ok(())
}
pub fn absolute(path: &Path) -> Result<PathBuf> {
    ensure!(path.is_absolute(), "absolute path required");
    ensure!(
        !path.components().any(|c| matches!(
            c,
            std::path::Component::ParentDir | std::path::Component::CurDir
        )),
        "non-canonical path"
    );
    for parent in path.ancestors().skip(1) {
        ordinary(parent, true)?;
    }
    ordinary(path, path.is_dir())?;
    Ok(fs::canonicalize(path)?)
}
pub fn checked(root: &Path, name: &str) -> Result<PathBuf> {
    ensure!(relative(name), "unsafe relative path");
    let path = root.join(name);
    for parent in path.ancestors().skip(1) {
        match fs::symlink_metadata(parent) {
            Ok(_) => ordinary(parent, true)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    match fs::symlink_metadata(&path) {
        Ok(_) => ordinary(&path, path.is_dir())?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    Ok(path)
}
pub fn stamp(path: &Path) -> Result<Stamp> {
    ordinary(path, false)?;
    let mut f = File::open(path)?;
    let mut h = Sha256::new();
    let mut bytes = 0;
    let mut buffer = [0u8; 65536];
    loop {
        let n = f.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        h.update(&buffer[..n]);
        bytes += n as u64;
    }
    Ok(Stamp {
        bytes,
        sha256: format!("{:x}", h.finalize()),
    })
}
pub fn exact(root: &Path, name: &str, want: &Stamp) -> Result<()> {
    ensure!(stamp(&checked(root, name)?)? == *want, "file drift: {name}");
    Ok(())
}
pub fn text(path: &Path) -> Result<String> {
    ordinary(path, false)?;
    ensure!(
        fs::metadata(path)?.len() <= 1024 * 1024,
        "oversized protocol file"
    );
    Ok(fs::read_to_string(path)?)
}
pub fn hold(root: &Path, r: &Request, raw: &str) -> Result<()> {
    let v: serde_json::Value = serde_json::from_str(&text(&checked(root, HOLD)?)?)?;
    ensure!(
        v["schemaVersion"] == 1
            && v["kind"] == "hdmap-forward"
            && v["transactionId"] == r.transaction_id
            && v["helperRequestSha256"] == digest(raw.as_bytes()),
        "outer hold does not own this request"
    );
    Ok(())
}
pub fn lease(root: &Path) -> Result<File> {
    // Qualified installed schema3 layout. Do not use alternate configured roots.
    let p = checked(root, "shroudforge/runtime/startup-assets.lock")?;
    ensure!(
        p.parent().is_some_and(Path::is_dir),
        "startup lock directory absent"
    );
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        Ok(OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .share_mode(0)
            .open(p)?)
    }
    #[cfg(not(windows))]
    {
        let _ = p;
        anyhow::bail!("Windows native lease required")
    }
}
pub fn membership(root: &Path) -> Result<BTreeMap<String, u64>> {
    let mut result = BTreeMap::new();
    for entry in fs::read_dir(root)? {
        let e = entry?;
        let n = e
            .file_name()
            .into_string()
            .map_err(|_| anyhow::anyhow!("nonUTF8 root member"))?;
        if asset(&n.to_ascii_lowercase()) {
            ensure!(n == n.to_ascii_lowercase(), "asset alias");
            ordinary(&e.path(), false)?;
            result.insert(n, e.metadata()?.len());
        }
    }
    Ok(result)
}
pub fn hd_members(root: &Path) -> Result<BTreeMap<String, Stamp>> {
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<String, Stamp>) -> Result<()> {
        ordinary(dir, true)?;
        for entry in fs::read_dir(dir)? {
            let e = entry?;
            let p = e.path();
            if e.file_type()?.is_dir() {
                walk(root, &p, out)?;
            } else {
                let n = p
                    .strip_prefix(root)?
                    .to_str()
                    .context("HD path encoding")?
                    .replace('\\', "/");
                out.insert(n, stamp(&p)?);
            }
        }
        Ok(())
    }
    let mut out = BTreeMap::new();
    walk(root, &checked(root, "mods/esm-real-map")?, &mut out)?;
    Ok(out)
}

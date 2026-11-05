use crate::read_budget::{Phase, SourceReads};
use anyhow::{Result, ensure};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Seek, SeekFrom},
    path::{Component, Path, PathBuf},
};
fn ordinary(m: &fs::Metadata, dir: bool) -> Result<()> {
    ensure!(!m.file_type().is_symlink(), "symlink");
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        ensure!(m.file_attributes() & 0x400 == 0, "reparse");
    }
    ensure!(if dir { m.is_dir() } else { m.is_file() }, "path kind");
    Ok(())
}

#[cfg(windows)]
fn file_id(f: &File) -> Result<Value> {
    use std::os::windows::io::AsRawHandle;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetFileInformationByHandle(h: *mut std::ffi::c_void, out: *mut std::ffi::c_void) -> i32;
    }
    let mut info = [0u32; 13];
    ensure!(
        unsafe { GetFileInformationByHandle(f.as_raw_handle(), info.as_mut_ptr() as *mut _) } != 0,
        "file identity"
    );
    Ok(json!({"volumeSerial":info[7],"fileIndexHigh":info[11],"fileIndexLow":info[12]}))
}
#[cfg(not(windows))]
fn file_id(_: &File) -> Result<Value> {
    anyhow::bail!("Windows identity required")
}
fn metadata_identity(path: &Path) -> Result<Value> {
    let mut o = OpenOptions::new();
    o.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        o.access_mode(0x80).share_mode(7).custom_flags(0x00200000);
    }
    let f = o.open(path)?;
    ordinary(&f.metadata()?, false)?;
    file_id(&f)
}

fn authorized_output(path: &Path) -> bool {
    let Some(text) = path.to_str() else { return false; };
    let path = Path::new(text.strip_prefix(r"\\?\").unwrap_or(text));
    if path.starts_with(Path::new(crate::acquire::OUTPUT)) { return true; }
    #[cfg(test)]
    if path.starts_with(Path::new("E:/Build/ikea-server-schema-tests-20261105-r1")) { return true; }
    false
}

pub struct DirectoryGuard {
    pub path: PathBuf,
    handles: Vec<File>,
}
impl DirectoryGuard {
    pub fn open(path: &Path, e_only: bool) -> Result<Self> {
        ensure!(
            path.is_absolute()
                && !path
                    .components()
                    .any(|c| matches!(c, Component::ParentDir | Component::CurDir)),
            "path syntax"
        );
        if e_only {
            ensure!(authorized_output(path), "fixed server output root");
        }
        let mut handles = Vec::new();
        for at in path.ancestors().collect::<Vec<_>>().into_iter().rev() {
            ordinary(&fs::symlink_metadata(at)?, true)?;
            let mut opts = OpenOptions::new();
            opts.read(true);
            #[cfg(windows)]
            {
                use std::os::windows::fs::OpenOptionsExt;
                opts.access_mode(0x80)
                    .share_mode(1)
                    .custom_flags(0x02000000 | 0x00200000);
            }
            ensure!(handles.len() < 64, "ancestor handle cap");
            let f = opts.open(at)?;
            ordinary(&f.metadata()?, true)?;
            handles.push(f);
        }
        Ok(Self {
            path: fs::canonicalize(path)?,
            handles,
        })
    }
    pub fn verify(&self) -> Result<()> {
        for f in &self.handles {
            ordinary(&f.metadata()?, true)?;
        }
        ensure!(
            fs::canonicalize(&self.path)? == self.path,
            "directory identity"
        );
        Ok(())
    }
}
pub struct Guard {
    file: File,
    bytes: u64,
    hash: String,
    path: PathBuf,
    id: Value,
    slot: usize,
}
impl Guard {
    pub(crate) fn open(
        dir: &DirectoryGuard,
        name: &str,
        size: u64,
        hash: &str,
        slot: usize,
        reads: &mut SourceReads,
    ) -> Result<Self> {
        ensure!(!name.contains(['\\', '/']) && name != "..", "filename");
        let path = dir.path.join(name);
        ordinary(&fs::symlink_metadata(&path)?, false)?;
        let mut o = OpenOptions::new();
        o.read(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            o.share_mode(1).custom_flags(0x00200000);
        }
        let file = o.open(&path)?;
        ordinary(&file.metadata()?, false)?;
        ensure!(file.metadata()?.len() == size, "size");
        let id = file_id(&file)?;
        let mut g = Self {
            file,
            bytes: size,
            hash: hash.into(),
            path,
            id,
            slot,
        };
        ensure!(g.digest(reads, Phase::BeforeHash)? == hash, "hash");
        Ok(g)
    }
    fn verify_metadata(&self) -> Result<()> {
        let held = self.file.metadata()?;
        let path = fs::symlink_metadata(&self.path)?;
        ordinary(&held, false)?;
        ordinary(&path, false)?;
        ensure!(
            held.len() == self.bytes && path.len() == self.bytes,
            "content size drift"
        );
        ensure!(
            file_id(&self.file)? == self.id && metadata_identity(&self.path)? == self.id,
            "content identity drift"
        );
        Ok(())
    }
    fn digest(&mut self, reads: &mut SourceReads, phase: Phase) -> Result<String> {
        let result = (|| {
            self.verify_metadata()?;
            self.file.seek(SeekFrom::Start(0))?;
            let mut hash = Sha256::new();
            reads.transfer(&mut self.file, self.slot, phase, |b| hash.update(b))?;
            self.verify_metadata()?;
            Ok(format!("{:x}", hash.finalize()))
        })();
        if result.is_err() {
            reads.fail();
        }
        result
    }
    pub(crate) fn bytes(&mut self, max: u64, reads: &mut SourceReads) -> Result<Vec<u8>> {
        let result = (|| {
            ensure!(self.bytes <= max, "content allocation");
            self.verify_metadata()?;
            self.file.seek(SeekFrom::Start(0))?;
            let mut out = Vec::with_capacity(usize::try_from(self.bytes)?);
            reads.transfer(&mut self.file, self.slot, Phase::Parse, |b| {
                out.extend_from_slice(b)
            })?;
            self.verify_metadata()?;
            ensure!(
                crate::schema::sha(&out) == self.hash,
                "content parse identity"
            );
            Ok(out)
        })();
        if result.is_err() {
            reads.fail();
        }
        result
    }
    pub fn observation(&self) -> Value {
        json!({"bytes":self.bytes,"sha256":self.hash,"fileIdentity":self.id,"heldReadLease":"FILE_SHARE_READ only; no write/delete sharing"})
    }
    pub(crate) fn verify(&mut self, reads: &mut SourceReads) -> Result<()> {
        let result = (|| {
            self.verify_metadata()?;
            ensure!(
                self.digest(reads, Phase::AfterHash)? == self.hash,
                "content changed"
            );
            self.verify_metadata()
        })();
        if result.is_err() {
            reads.fail();
        }
        result
    }
}
pub fn namespace(dir: &DirectoryGuard) -> Result<Value> {
    let mut rows = Vec::new();
    let mut total = 0;
    for e in fs::read_dir(&dir.path)? {
        total += 1;
        ensure!(total <= 10000, "namespace entry cap");
        let e = e?;
        let name = e
            .file_name()
            .into_string()
            .map_err(|_| anyhow::anyhow!("namespace UTF8"))?;
        let low = name.to_ascii_lowercase();
        let Some(n) = low
            .strip_prefix("enshrouded_server_")
            .and_then(|n| n.strip_suffix(".dat"))
        else {
            continue;
        };
        ensure!(
            name == low && !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()),
            "namespace syntax"
        );
        let index: u32 = n.parse()?;
        ensure!(
            index <= 255 && name == format!("enshrouded_server_{index:03}.dat"),
            "namespace slot"
        );
        let m = fs::symlink_metadata(e.path())?;
        ordinary(&m, false)?;
        #[cfg(windows)]
        let identity = {
            use std::os::windows::fs::MetadataExt;
            json!({"attributes":m.file_attributes(),"creationTime":m.creation_time(),"lastWriteTime":m.last_write_time()})
        };
        #[cfg(not(windows))]
        let identity = json!({});
        rows.push(json!({"name":name,"index":index,"bytes":m.len(),"observedMetadata":identity,"fileIdentity":metadata_identity(&e.path())?,"metadataOnlyHandleOpened":true,"contentSha256":"UNOBSERVED","DATContentOpened":false}));
        ensure!(rows.len() <= 256, "namespace cap");
    }
    rows.sort_by_key(|v| v["index"].as_u64().unwrap());
    Ok(
        json!({"members":rows,"enumeratedDirectoryEntries":total,"DATContentReads":false,"metadataIdentityNotFullFileFreshness":true}),
    )
}
pub fn output(parent: &Path, name: &str) -> Result<DirectoryGuard> {
    let p = DirectoryGuard::open(parent, true)?;
    ensure!(name == "current-acquisition-v1", "output name");
    fs::create_dir(p.path.join(name))?;
    DirectoryGuard::open(&p.path.join(name), true)
}
pub fn write_new(dir: &DirectoryGuard, name: &str, data: &[u8], max: usize) -> Result<()> {
    ensure!(
        data.len() <= max && !name.contains(['\\', '/']),
        "output cap/path"
    );
    let mut f = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(dir.path.join(name))?;
    use std::io::Write;
    f.write_all(data)?;
    f.sync_all()?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn phase_a_namespace_never_reads_dat_body_and_guards_deny_write() {
        let base = std::path::PathBuf::from(
            "E:/Build/ikea-server-schema-tests-20261105-r1/synthetic-guards",
        );
        fs::create_dir_all(&base).unwrap();
        let f = base.join("enshrouded_server_007.dat");
        if !f.exists() {
            fs::write(&f, b"DO_NOT_READ_DAT_SENTINEL").unwrap();
        }
        let dir = DirectoryGuard::open(&base, true).unwrap();
        let n = namespace(&dir).unwrap();
        assert_eq!(n["members"][0]["contentSha256"], "UNOBSERVED");
        assert!(!n.to_string().contains("DO_NOT_READ_DAT_SENTINEL"));
        let b = base.join("fixed.bin");
        if !b.exists() {
            fs::write(&b, b"fixed").unwrap();
        }
        let mut reads = SourceReads::new([5, 0, 0]);
        let mut g = Guard::open(
            &dir,
            "fixed.bin",
            5,
            &crate::schema::sha(b"fixed"),
            0,
            &mut reads,
        )
        .unwrap();
        assert!(OpenOptions::new().write(true).open(&b).is_err());
        assert!(DirectoryGuard::open(&base.join(".."), true).is_err());
        assert!(
            Guard::open(
                &dir,
                "../fixed.bin",
                5,
                "bad",
                0,
                &mut SourceReads::new([5, 0, 0])
            )
            .is_err()
        );
        g.verify(&mut reads).unwrap();
        assert!(g.bytes(4, &mut reads).is_err());
        dir.verify().unwrap();
        assert!(
            Guard::open(
                &dir,
                "fixed.bin",
                5,
                "bad",
                0,
                &mut SourceReads::new([5, 0, 0])
            )
            .is_err()
        );
    }
}

#[cfg(test)]
mod output_tests {
    use super::*;
    #[test]
    fn phase_a_output_createnew_bounded_no_overwrite_or_path_escape() {
        let base = std::path::PathBuf::from(format!(
            "E:/Build/ikea-server-schema-tests-20261105-r1/synthetic-output-{}",
            std::process::id()
        ));
        fs::create_dir(&base).unwrap();
        let out = output(&base, "current-acquisition-v1").unwrap();
        write_new(&out, "bounded.json", b"{}", 2).unwrap();
        assert!(write_new(&out, "bounded.json", b"BAD", 3).is_err());
        assert!(write_new(&out, "too-large.json", b"long", 3).is_err());
        assert!(!out.path.join("too-large.json").exists());
        assert!(write_new(&out, "../escape.json", b"{}", 2).is_err());
        assert!(output(&base, "current-acquisition-v1").is_err());
        out.verify().unwrap();
    }
}

#[cfg(test)]
mod accounting_guard_tests {
    use super::*;
    #[test]
    fn read_a1_held_metadata_drift_refuses_before_additional_content_request() {
        let base = PathBuf::from(format!(
            "E:/Build/ikea-server-schema-tests-20261105-r1/synthetic-accounting-{}",
            std::process::id()
        ));
        fs::create_dir(&base).unwrap();
        fs::write(base.join("enshrouded_server.exe"), b"fixed").unwrap();
        let dir = DirectoryGuard::open(&base, true).unwrap();
        let mut reads = SourceReads::new([5, 0, 0]);
        let mut held = Guard::open(
            &dir,
            "enshrouded_server.exe",
            5,
            &crate::schema::sha(b"fixed"),
            0,
            &mut reads,
        )
        .unwrap();
        assert!(
            OpenOptions::new()
                .write(true)
                .open(base.join("enshrouded_server.exe"))
                .is_err()
        );
        let before = reads.observation();
        // Exact held/path metadata validation is exercised independently of EOF reads.
        held.bytes = 4;
        assert!(held.bytes(5, &mut reads).is_err());
        let after = reads.observation();
        assert_eq!(before["requestedBytes"], after["requestedBytes"]);
        assert_eq!(after["terminal"], true);
        assert!(held.verify(&mut reads).is_err());
        assert_eq!(reads.observation()["readCalls"], 1);
    }
}

#[cfg(test)]
mod server_output_tests {
    use super::*;
    #[test]
    fn rejects_client_live_and_sibling_output_roots() {
        assert!(authorized_output(Path::new(crate::acquire::OUTPUT)));
        assert!(authorized_output(Path::new(r"\\?\D:\GameServers\staging\ikea-server-schema-observation-20261105-r1\current-acquisition-v1")));
        assert!(authorized_output(&Path::new(crate::acquire::OUTPUT).join("current-acquisition-v1")));
        assert!(!authorized_output(Path::new("D:/GameServers/IKEA")));
        assert!(!authorized_output(Path::new("D:/GameServers/staging/ikea-server-schema-observation-20261105-r1-extra")));
        assert!(!authorized_output(Path::new("D:/Games/SteamLibrary/steamapps/common/Enshrouded")));
        assert!(!authorized_output(Path::new("E:/Build/unrelated")));
    }
}

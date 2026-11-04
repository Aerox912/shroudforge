//! Read-only Windows identity pins. No ACL, permission or content changes.
use anyhow::{Context, Result, ensure};
use std::{
    fs::{File, OpenOptions},
    path::{Component, Path, PathBuf},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub volume: u64,
    pub id: [u8; 16],
    pub bytes: u64,
    write: u64,
    attributes: u32,
}
impl Identity {
    pub fn key(&self) -> (u64, [u8; 16]) {
        (self.volume, self.id)
    }
}

pub struct PinnedDirectory {
    file: File,
    path: PathBuf,
    identity: Identity,
}

pub fn pin_chain(path: &Path) -> Result<Vec<PinnedDirectory>> {
    ensure!(
        path.is_absolute()
            && !path
                .components()
                .any(|c| matches!(c, Component::ParentDir | Component::CurDir)),
        "plain absolute source root"
    );
    let text = path.to_str().context("nonUTF8 source root")?;
    ensure!(
        text.len() >= 3
            && text.as_bytes()[0].eq_ignore_ascii_case(&b'e')
            && text.as_bytes()[1] == b':'
            && !text.starts_with("\\\\"),
        "E-only source root"
    );
    path.ancestors().map(PinnedDirectory::open).collect()
}

impl PinnedDirectory {
    pub fn open(path: &Path) -> Result<Self> {
        let mut options = OpenOptions::new();
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options
                .access_mode(0x80)
                .share_mode(3)
                .custom_flags(0x02200000);
        }
        #[cfg(not(windows))]
        {
            options.read(true);
        }
        let file = options.open(path)?;
        let identity = snapshot(&file, path, true)?;
        Ok(Self {
            file,
            path: path.to_owned(),
            identity,
        })
    }
    pub fn verify(&self) -> Result<()> {
        ensure!(
            snapshot(&self.file, &self.path, true)? == self.identity,
            "directory identity drift"
        );
        Ok(())
    }
}

#[cfg(windows)]
pub fn snapshot(file: &File, path: &Path, directory: bool) -> Result<Identity> {
    use std::{
        ffi::c_void,
        os::windows::{fs::MetadataExt, io::AsRawHandle},
    };
    #[repr(C)]
    struct FileId {
        volume: u64,
        id: [u8; 16],
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetFileInformationByHandleEx(
            handle: *mut c_void,
            class: u32,
            information: *mut c_void,
            bytes: u32,
        ) -> i32;
        fn GetFinalPathNameByHandleW(
            handle: *mut c_void,
            name: *mut u16,
            length: u32,
            flags: u32,
        ) -> u32;
    }
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_dir() == directory
            && (directory || metadata.is_file())
            && metadata.file_attributes() & 0x400 == 0,
        "held kind/reparse"
    );
    let mut id = FileId {
        volume: 0,
        id: [0; 16],
    };
    ensure!(
        unsafe {
            GetFileInformationByHandleEx(
                file.as_raw_handle(),
                18,
                (&mut id as *mut FileId).cast(),
                24,
            )
        } != 0,
        "file identity lookup: {}",
        std::io::Error::last_os_error()
    );
    let mut name = vec![0u16; 32768];
    let length = unsafe {
        GetFinalPathNameByHandleW(
            file.as_raw_handle(),
            name.as_mut_ptr(),
            name.len() as u32,
            0,
        )
    } as usize;
    ensure!(length > 0 && length < name.len(), "held final path");
    let actual = String::from_utf16(&name[..length])?;
    let actual = actual
        .strip_prefix("\\\\?\\")
        .unwrap_or(&actual)
        .trim_end_matches('\\');
    let wanted = path
        .to_str()
        .context("nonUTF8 held path")?
        .replace('/', "\\");
    ensure!(
        actual.eq_ignore_ascii_case(wanted.trim_end_matches('\\')),
        "held final path differs"
    );
    Ok(Identity {
        volume: id.volume,
        id: id.id,
        bytes: if directory { 0 } else { metadata.file_size() },
        write: if directory {
            0
        } else {
            metadata.last_write_time()
        },
        attributes: metadata.file_attributes(),
    })
}

#[cfg(not(windows))]
pub fn snapshot(_file: &File, _path: &Path, _directory: bool) -> Result<Identity> {
    anyhow::bail!("this private source lease requires Windows file identities")
}

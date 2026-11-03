//! Primary DLL identity and held path for the explicit host-preload policy.
//! This is not the diagnostic probe selector or executable publication proof.
use sha2::{Digest, Sha256};
use std::{
    fs::{File, OpenOptions},
    io::Read,
    path::{Path, PathBuf},
};

pub(crate) struct PreloadArtifact {
    pub(crate) path: PathBuf,
    pub(crate) sha256: [u8; 32],
    pub(crate) bytes: u64,
    identity: (u64, [u8; 16]),
    _file: File,
    _directories: Vec<File>,
}
const MAX_BYTES: u64 = 64 * 1024 * 1024;
#[cfg(windows)]
fn identity(file: &File) -> Result<(u64, [u8; 16]), String> {
    use std::{ffi::c_void, os::windows::io::AsRawHandle};
    #[repr(C)]
    struct FileIdInfo {
        volume: u64,
        id: [u8; 16],
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetFileInformationByHandleEx(
            handle: *mut c_void,
            class: i32,
            output: *mut c_void,
            size: u32,
        ) -> i32;
    }
    let mut info = std::mem::MaybeUninit::<FileIdInfo>::uninit();
    if unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            18,
            info.as_mut_ptr().cast(),
            std::mem::size_of::<FileIdInfo>() as u32,
        )
    } == 0
    {
        return Err(format!(
            "preload file identity unavailable: {}",
            std::io::Error::last_os_error()
        ));
    }
    let info = unsafe { info.assume_init() };
    Ok((info.volume, info.id))
}
#[cfg(not(windows))]
fn identity(_: &File) -> Result<(u64, [u8; 16]), String> {
    Err("host preload is Windows-only".into())
}
fn read_locked(path: &Path) -> Result<(File, (u64, [u8; 16]), u64, [u8; 32]), String> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(1).custom_flags(0x0020_0000);
    }
    let mut file = options.open(path).map_err(|e| e.to_string())?;
    let meta = file.metadata().map_err(|e| e.to_string())?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 {
            return Err("preload artifact is a reparse point".into());
        }
    }
    if !meta.is_file() || meta.len() > MAX_BYTES {
        return Err("preload primary DLL must be a regular file at most64MiB".into());
    }
    let id = identity(&file)?;
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 32768];
    let mut count = 0u64;
    loop {
        let n = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        count = count.checked_add(n as u64).ok_or("preload size overflow")?;
        if count > MAX_BYTES {
            return Err("preload DLL grew beyond its bound".into());
        }
        digest.update(&buffer[..n]);
    }
    if count != meta.len() || identity(&file)? != id {
        return Err("preload DLL changed while hashing".into());
    }
    Ok((file, id, count, digest.finalize().into()))
}
impl PreloadArtifact {
    pub(crate) fn open(path: &Path) -> Result<Self, String> {
        #[cfg(not(windows))]
        {
            let _ = path;
            return Err("host preload is Windows-only".into());
        }
        #[cfg(windows)]
        {
            use std::{
                os::windows::fs::{MetadataExt, OpenOptionsExt},
                path::{Component, Prefix},
            };
            if !path.is_absolute()
                || !matches!(path.components().next(),Some(Component::Prefix(p)) if matches!(p.kind(),Prefix::Disk(_)|Prefix::VerbatimDisk(_)))
            {
                return Err("preload artifact needs an absolute local-drive path".into());
            }
            let mut directories = Vec::new();
            for parent in path.ancestors().skip(1) {
                let handle = OpenOptions::new()
                    .read(true)
                    .share_mode(1)
                    .custom_flags(0x0220_0000)
                    .open(parent)
                    .map_err(|e| format!("cannot hold preload path {}: {e}", parent.display()))?;
                let meta = handle.metadata().map_err(|e| e.to_string())?;
                if !meta.is_dir() || meta.file_attributes() & 0x400 != 0 {
                    return Err("preload path contains redirection".into());
                }
                directories.push(handle);
            }
            let canonical = path.canonicalize().map_err(|e| e.to_string())?;
            let (file, id, bytes, sha256) = read_locked(&canonical)?;
            Ok(Self {
                path: canonical,
                sha256,
                bytes,
                identity: id,
                _file: file,
                _directories: directories,
            })
        }
    }
    pub(crate) fn ensure_unmapped(&self) -> Result<(), String> {
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            let wide: Vec<u16> = self.path.as_os_str().encode_wide().chain(Some(0)).collect();
            if !unsafe {
                windows_sys::Win32::System::LibraryLoader::GetModuleHandleW(wide.as_ptr())
            }
            .is_null()
            {
                return Err("preload DLL is already mapped outside this owner".into());
            }
        }
        Ok(())
    }
    pub(crate) fn verify_mapping(&self, module: usize) -> Result<(), String> {
        #[cfg(windows)]
        {
            let mut path = vec![0u16; 32768];
            let n = unsafe {
                windows_sys::Win32::System::LibraryLoader::GetModuleFileNameW(
                    module as _,
                    path.as_mut_ptr(),
                    path.len() as u32,
                )
            } as usize;
            if n == 0 || n >= path.len() {
                return Err("preload mapped module path unavailable".into());
            }
            let path = PathBuf::from(
                String::from_utf16(&path[..n]).map_err(|_| "preload mapped path is not UTF16")?,
            )
            .canonicalize()
            .map_err(|e| e.to_string())?;
            if path != self.path {
                return Err("preload mapped path differs from selected artifact".into());
            }
            let (_, id, bytes, sha) = read_locked(&path)?;
            if id != self.identity || bytes != self.bytes || sha != self.sha256 {
                return Err("preload mapped file identity/content mismatch".into());
            }
            Ok(())
        }
        #[cfg(not(windows))]
        {
            let _ = module;
            Err("host preload is Windows-only".into())
        }
    }
}
pub(crate) fn address_in_module(address: usize, module: usize) -> bool {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::LibraryLoader::{
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
            GetModuleHandleExW,
        };
        let mut actual = std::ptr::null_mut();
        unsafe {
            GetModuleHandleExW(
                GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS
                    | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
                address as *const u16,
                &mut actual,
            ) != 0
                && actual as usize == module
        }
    }
    #[cfg(not(windows))]
    {
        let _ = (address, module);
        false
    }
}

//! One explicitly selected diagnostic instance, prepared while PE entry is
//! genuinely held; later drained/revoked on that same bootstrap/runtime thread.
//! No ShroudForgePluginEntry is negotiated and no second DLL mapping is made.
use super::{NativeLibrary, load_native_library, process_lifetime};
use std::{
    ffi::c_void,
    mem::size_of,
    path::PathBuf,
    sync::{Arc, Mutex},
    thread::ThreadId,
};
const GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS: u32 = 4;
const GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT: u32 = 2;

#[path = "early_probe_admission.rs"]
mod early_probe_admission;
#[path = "early_probe_contract.rs"]
mod early_probe_contract;
use early_probe_contract::{Attach, Journal, validate_attachment};

#[repr(C)]
struct ProbeHost {
    size: u32,
    version: u32,
    module: u64,
    image: u64,
    generation: u64,
    retained_context: *mut c_void,
    copy_context: *const c_void,
    reserved: [u64; 2],
}
type ValidateOwner = unsafe extern "C" fn(usize, u64, *const c_void) -> bool;
#[repr(C)]
struct OwnerProof {
    size: u32,
    version: u32,
    module: u64,
    generation: u64,
    runtime_image: u64,
    controller_identity: *const c_void,
    validator: ValidateOwner,
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Publication {
    size: u32,
    version: u32,
    status: u32,
    ranges_claimed: u32,
    pair_published: u32,
    reserved: u32,
    relays: [u64; 2],
    relay_bytes: [u32; 2],
}
type Publish =
    unsafe extern "C" fn(*const c_void, *const OwnerProof, *const Attach, *mut Publication) -> u32;
#[repr(C)]
struct BootstrapHost {
    size: u32,
    version: u32,
    runtime_image: u64,
    executable_image: u64,
    held_entry: *const c_void,
    publish: Option<Publish>,
}
type Prepare = unsafe extern "C" fn(*const ProbeHost, *mut Attach) -> u32;
type Drain = unsafe extern "C" fn(u32, *mut Record, *mut u32, *mut u64) -> u32;
type Revoke = unsafe extern "C" fn();
#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct Stats {
    size: u32,
    transfer_capability: u32,
    hits: [u64; 4],
    dropped: u64,
    capture_enabled: u32,
    reserved: u32,
}
type ReadStats = unsafe extern "C" fn(*mut Stats) -> u32;
#[repr(C, align(8))]
#[derive(Clone, Copy)]
struct Record {
    bytes: [u8; 248],
}
struct Identity {
    admission: Arc<process_lifetime::Admission>,
    module: usize,
    generation: u64,
    runtime_image: usize,
    owner: ThreadId,
}
struct Controller {
    identity: Box<Identity>,
    _library: NativeLibrary,
    _files: early_probe_admission::ValidatedFiles,
    prepare: Prepare,
    drain: Drain,
    revoke: Revoke,
    stats: ReadStats,
    last_stats: Option<Stats>,
    journal: Journal,
    publication: Publication,
    records: [Record; 32],
    dropped: u64,
}
static CONTROLLER: Mutex<Option<Box<Controller>>> = Mutex::new(None);

#[repr(C)]
struct MemoryInfo {
    base: *mut c_void,
    allocation: *mut c_void,
    allocation_protect: u32,
    partition: u16,
    padding: u16,
    size: usize,
    state: u32,
    protect: u32,
    kind: u32,
}
#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetModuleHandleW(name: *const u16) -> *mut c_void;
    fn GetModuleHandleExW(flags: u32, address: *const u16, module: *mut *mut c_void) -> i32;
    fn GetModuleFileNameW(module: *mut c_void, output: *mut u16, capacity: u32) -> u32;
    fn GetProcAddress(
        module: *mut c_void,
        name: *const u8,
    ) -> Option<unsafe extern "system" fn() -> isize>;
    fn VirtualQuery(address: *const c_void, output: *mut MemoryInfo, size: usize) -> usize;
    fn ReadProcessMemory(
        process: *mut c_void,
        address: *const c_void,
        output: *mut c_void,
        size: usize,
        read: *mut usize,
    ) -> i32;
    fn GetCurrentProcess() -> *mut c_void;
}
fn module_at(address: usize) -> Option<usize> {
    let mut module = std::ptr::null_mut();
    let ok = unsafe {
        GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
            address as *const u16,
            &mut module,
        )
    };
    (ok != 0).then_some(module as usize)
}
fn executable_in(module: usize, address: usize) -> bool {
    let mut info = std::mem::MaybeUninit::<MemoryInfo>::uninit();
    if unsafe {
        VirtualQuery(
            address as *const c_void,
            info.as_mut_ptr(),
            size_of::<MemoryInfo>(),
        )
    } != size_of::<MemoryInfo>()
    {
        return false;
    }
    let info = unsafe { info.assume_init() };
    const MEM_COMMIT: u32 = 0x1000;
    const MEM_IMAGE: u32 = 0x1000000;
    info.allocation as usize == module
        && info.state == MEM_COMMIT
        && info.kind == MEM_IMAGE
        && info.protect & 0x101 == 0
        && matches!(info.protect & 0xff, 0x20 | 0x40 | 0x80)
}
fn module_path(module: usize) -> Result<PathBuf, String> {
    let mut buffer = vec![0u16; 32768];
    let count = unsafe { GetModuleFileNameW(module as _, buffer.as_mut_ptr(), buffer.len() as u32) }
        as usize;
    if count == 0 || count >= buffer.len() {
        return Err("mapped diagnostic module path unavailable".into());
    }
    PathBuf::from(String::from_utf16(&buffer[..count]).map_err(|_| "invalid module path")?)
        .canonicalize()
        .map_err(|e| e.to_string())
}
fn read_memory(address: usize, bytes: &mut [u8]) -> bool {
    let mut read = 0;
    unsafe {
        ReadProcessMemory(
            GetCurrentProcess(),
            address as _,
            bytes.as_mut_ptr().cast(),
            bytes.len(),
            &mut read,
        ) != 0
            && read == bytes.len()
    }
}
fn image_size(module: usize) -> Result<usize, String> {
    let mut dos = [0u8; 64];
    if !read_memory(module, &mut dos) || &dos[..2] != b"MZ" {
        return Err("diagnostic DOS header refused".into());
    }
    let offset = u32::from_le_bytes(dos[60..64].try_into().unwrap()) as usize;
    if offset > 1024 * 1024 {
        return Err("diagnostic PE offset refused".into());
    }
    let mut pe = [0u8; 88];
    if !read_memory(module.checked_add(offset).ok_or("PE overflow")?, &mut pe)
        || &pe[..4] != b"PE\0\0"
        || u16::from_le_bytes(pe[4..6].try_into().unwrap()) != 0x8664
        || u16::from_le_bytes(pe[24..26].try_into().unwrap()) != 0x20b
    {
        return Err("diagnostic PE image refused".into());
    }
    let size = u32::from_le_bytes(pe[80..84].try_into().unwrap()) as usize;
    if size == 0 || size > 64 * 1024 * 1024 {
        return Err("diagnostic mapped image size refused".into());
    }
    Ok(size)
}
fn symbol(module: usize, name: &[u8]) -> Result<usize, String> {
    let address = unsafe { GetProcAddress(module as _, name.as_ptr()) }
        .map(|f| f as usize)
        .ok_or("required diagnostic export missing")?;
    if !executable_in(module, address) {
        return Err("diagnostic export is not code in admitted mapping".into());
    }
    Ok(address)
}

unsafe extern "C" fn validate_owner(
    module: usize,
    generation: u64,
    identity: *const c_void,
) -> bool {
    if identity.is_null() {
        return false;
    }
    // Issued only to synchronous trusted bootstrap; never a native sampler.
    // Identity lives in the permanent controller and contains no game pointer.
    let owner = unsafe { &*identity.cast::<Identity>() };
    owner.owner == std::thread::current().id()
        && owner.module == module
        && owner.generation == generation
        && owner.admission.is_bound_host(identity as usize)
        && module_at(validate_owner as *const () as usize) == Some(owner.runtime_image)
        && owner.admission.is_retained()
        && owner
            .admission
            .validate(early_probe_contract::PACKAGE_ID, generation)
            .is_ok()
}

pub(crate) unsafe fn prepare(host: *const c_void) -> u32 {
    match unsafe { prepare_selected(host) } {
        Ok(value) => value,
        Err(error) => {
            tracing::error!(target:"shroudforge::early-probe",%error,"Early observation preparation refused; entry release remains blocked");
            2
        }
    }
}
unsafe fn prepare_selected(host: *const c_void) -> Result<u32, String> {
    let Some(path) = std::env::var_os(early_probe_admission::ENVIRONMENT) else {
        return Ok(0);
    };
    let Some(files) = early_probe_admission::read_selected(std::path::Path::new(&path))? else {
        return Ok(0);
    };
    let selected_admission_sha256 = files.admission_sha256.clone();
    let host =
        unsafe { host.cast::<BootstrapHost>().as_ref() }.ok_or("missing private bootstrap host")?;
    if host.size != size_of::<BootstrapHost>() as u32
        || host.version != 1
        || host.held_entry.is_null()
        || host.publish.is_none()
        || host.executable_image != unsafe { GetModuleHandleW(std::ptr::null()) } as u64
        || module_at(validate_owner as *const () as usize) != Some(host.runtime_image as usize)
    {
        return Err("actual held-entry runtime binding unavailable".into());
    }
    let mut slot = CONTROLLER
        .lock()
        .map_err(|_| "early controller unavailable")?;
    if slot.is_some() {
        return Err("early diagnostic is one-shot and already retained; restart required".into());
    }
    let executable = module_path(host.executable_image as usize)?;
    let executable_file = early_probe_admission::hold_file(&executable, 64 * 1024 * 1024)?;
    if early_probe_admission::hex_digest(&executable_file.bytes) != early_probe_contract::EXE_SHA256
    {
        return Err("unsupported dedicated-server executable identity".into());
    }
    let generation = process_lifetime::next_generation()?;
    let admission = process_lifetime::reserve(early_probe_contract::PACKAGE_ID, generation)?;
    use std::os::windows::ffi::OsStrExt;
    let path_wide = files
        .dll
        .path
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    if !unsafe { GetModuleHandleW(path_wide.as_ptr()) }.is_null() {
        return Err("selected diagnostic is already mapped outside this controller".into());
    }
    let mut library = load_native_library(early_probe_contract::PACKAGE_ID, &files.dll.path, None)?;
    let module = library.handle as usize;
    admission.bind_module(module)?;
    library.admission = Some(admission.clone());
    if module_path(module)? != files.dll.path {
        return Err("mapped diagnostic path differs from held file".into());
    }
    let mapped_file = early_probe_admission::hold_file(&module_path(module)?, 16 * 1024 * 1024)?;
    if !files.dll.same_identity(&mapped_file)
        || early_probe_admission::hex_digest(&mapped_file.bytes) != early_probe_contract::DLL_SHA256
    {
        return Err("mapped diagnostic file identity/content changed".into());
    }
    let module_size = image_size(module)?;
    if unsafe { GetProcAddress(module as _, b"ShroudForgePluginEntry\0".as_ptr()) }.is_some() {
        return Err("Stage1 diagnostic must not be adopted as an SDK plugin".into());
    }
    let prepare = unsafe {
        std::mem::transmute::<usize, Prepare>(symbol(module, b"SFALProbePrepareAttachV1\0")?)
    };
    let drain =
        unsafe { std::mem::transmute::<usize, Drain>(symbol(module, b"SFALProbeDrainV1\0")?) };
    let revoke = unsafe {
        std::mem::transmute::<usize, Revoke>(symbol(module, b"SFALProbeRevokeCaptureV1\0")?)
    };
    let stats = unsafe {
        std::mem::transmute::<usize, ReadStats>(symbol(module, b"SFALProbeReadStatsV1\0")?)
    };
    let controller = Box::new(Controller {
        identity: Box::new(Identity {
            admission: admission.clone(),
            module,
            generation,
            runtime_image: host.runtime_image as usize,
            owner: std::thread::current().id(),
        }),
        _library: library,
        _files: files,
        prepare,
        drain,
        revoke,
        stats,
        last_stats: None,
        journal: Journal::default(),
        publication: Publication::default(),
        records: [Record { bytes: [0; 248] }; 32],
        dropped: 0,
    });
    let identity = (&*controller.identity as *const Identity).cast::<c_void>();
    admission.retain_early_controller(identity as usize)?;
    // No fallible step between retention commit and publishing full ownership.
    *slot = Some(controller);
    let controller = slot.as_mut().unwrap();
    controller.journal.retained = true;
    let probe_host = ProbeHost {
        size: size_of::<ProbeHost>() as u32,
        version: 1,
        module: module as u64,
        image: host.executable_image,
        generation,
        retained_context: std::ptr::null_mut(),
        copy_context: std::ptr::null(),
        reserved: [0; 2],
    };
    let mut attach = Attach::default();
    controller.journal.begin_prepare().map_err(str::to_owned)?;
    let status = unsafe { (controller.prepare)(&probe_host, &mut attach) };
    let outcome = (|| -> Result<(), String> {
        controller
            .journal
            .finish_prepare(status == 0)
            .map_err(|reason| format!("{reason}: {status}"))?;
        validate_attachment(
            &attach,
            host.executable_image,
            module as u64,
            module_size as u64,
            generation,
        )
        .map_err(str::to_owned)?;
        if attach
            .calls
            .iter()
            .any(|call| !executable_in(module, call.probe_entry as usize))
        {
            return Err("returned probe entry outside admitted executable image".into());
        }
        let owner = OwnerProof {
            size: size_of::<OwnerProof>() as u32,
            version: 1,
            module: module as u64,
            generation,
            runtime_image: host.runtime_image,
            controller_identity: identity,
            validator: validate_owner,
        };
        let status = unsafe {
            host.publish.unwrap()(
                host.held_entry,
                &owner,
                &attach,
                &mut controller.publication,
            )
        };
        let result = controller.publication;
        controller
            .journal
            .record_publication(
                result.ranges_claimed == 1,
                result.pair_published == 1,
                status,
            )
            .map_err(str::to_owned)?;
        if status != 0
            || result.size != size_of::<Publication>() as u32
            || result.version != 1
            || result.status != 0
            || result.ranges_claimed != 1
            || result.pair_published != 1
            || result.reserved != 0
            || result.relays.iter().any(|entry| *entry == 0)
            || result.relay_bytes != [16; 2]
        {
            return Err(format!(
                "retained pair preparation/publication refused: {status}; claim={}, published={}",
                result.ranges_claimed, result.pair_published
            ));
        }
        Ok(())
    })();
    if let Err(error) = outcome {
        controller.revoke_capture();
        return Err(error);
    }
    tracing::info!(target:"shroudforge::early-probe",admission_sha256=%selected_admission_sha256,
        generation,"Stage1 retained CALL pair prepared; runtime observation remains independently qualified");
    Ok(1)
}

impl Controller {
    fn revoke_capture(&mut self) {
        if self.journal.revoke() {
            unsafe {
                (self.revoke)();
            }
        }
    }
}
pub(crate) fn tick() -> u32 {
    let Ok(mut slot) = CONTROLLER.lock() else {
        return 2;
    };
    let Some(controller) = slot.as_mut() else {
        return 0;
    };
    if controller.identity.owner != std::thread::current().id() {
        return 2;
    }
    if !controller.journal.can_drain() {
        return 0;
    }
    let mut copied = 0;
    let mut dropped = 0;
    if unsafe {
        (controller.drain)(
            controller.records.len() as u32,
            controller.records.as_mut_ptr(),
            &mut copied,
            &mut dropped,
        )
    } != 0
        || copied as usize > controller.records.len()
    {
        controller.revoke_capture();
        return 2;
    }
    for index in 0..copied as usize {
        let record = controller.records[index];
        let u32_at =
            |offset| u32::from_le_bytes(record.bytes[offset..offset + 4].try_into().unwrap());
        let u64_at =
            |offset| u64::from_le_bytes(record.bytes[offset..offset + 8].try_into().unwrap());
        if u32_at(0) != 248
            || !matches!(u32_at(4), 1 | 2)
            || u32_at(8) > 1
            || u32_at(12) & !0x3ff != 0
        {
            controller.revoke_capture();
            return 2;
        }
        tracing::info!(target:"shroudforge::early-probe",site=u32_at(4),phase=u32_at(8),flags=u32_at(12),
            sequence=u64_at(16),diagnostic_token=u64_at(24),native_caller_rva=u32_at(32),callback_caller_rva=u32_at(36),
            "Copied Stage1 observation; transfer capability remains zero");
    }
    if dropped != controller.dropped {
        tracing::warn!(target:"shroudforge::early-probe",dropped,"Observation ring loss");
        controller.dropped = dropped;
    }
    let mut stats = Stats {
        size: size_of::<Stats>() as u32,
        ..Stats::default()
    };
    if unsafe { (controller.stats)(&mut stats) } != 0
        || stats.size != size_of::<Stats>() as u32
        || stats.transfer_capability != 0
        || stats.capture_enabled > 1
        || stats.reserved != 0
    {
        controller.revoke_capture();
        return 2;
    }
    if controller.last_stats != Some(stats) {
        tracing::info!(target:"shroudforge::early-probe",builder_pre=stats.hits[0],builder_post=stats.hits[1],
            stock_pre=stats.hits[2],stock_post=stats.hits[3],dropped=stats.dropped,capture_enabled=stats.capture_enabled,
            "Stage1 counters; zero hits are an explicit observation, not a transfer result");
        controller.last_stats = Some(stats);
    }
    0
}
pub(crate) fn revoke() {
    // A panic in owner-thread diagnostic formatting must not prevent the
    // already retained controller's no-allocation capture revocation.
    let mut slot = match CONTROLLER.lock() {
        Ok(slot) => slot,
        Err(poisoned) => poisoned.into_inner(),
    };
    if let Some(controller) = slot.as_mut() {
        if controller.identity.owner == std::thread::current().id() {
            controller.revoke_capture();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn private_bootstrap_and_imported_probe_layouts_match_native() {
        assert_eq!(size_of::<BootstrapHost>(), 40);
        assert_eq!(size_of::<OwnerProof>(), 48);
        assert_eq!(size_of::<Publication>(), 48);
        assert_eq!(size_of::<ProbeHost>(), 64);
        assert_eq!(size_of::<Stats>(), 56);
        assert_eq!(size_of::<Record>(), 248);
        assert_eq!(size_of::<MemoryInfo>(), 48);
    }
}

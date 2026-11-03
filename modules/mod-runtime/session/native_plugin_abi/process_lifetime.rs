//! Explicit host lifetime only. This does not own patch ranges or arbitrary
//! plugin allocations. The registry is shared by AppStates in this SF image.
use super::*;
use crate::session::native_plugins::preload_artifact::{PreloadArtifact, address_in_module};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{LazyLock, Mutex, Weak};
use std::thread::ThreadId;

const PREPARED: usize = 0;
const HOST_PINNED: usize = 1;
const LOADING: usize = 2;
const MODULE_BOUND: usize = 3;
const ADOPTED: usize = 4;
const CANCELLED: usize = 5;
const FAILED: usize = 6;

pub(crate) struct PreloadIntent {
    pub(crate) artifact: PreloadArtifact,
    pub(crate) package_binding: mod_loader::package_lookup::ModPackageBinding,
    pub(crate) relative_path: String,
    pub(crate) process_target: u32,
}
struct PreloadOwner {
    intent: PreloadIntent,
    creator: ThreadId,
    phase: AtomicUsize,
    host_image: AtomicUsize,
    // This owns the one returned LoadLibrary reference, including quarantine.
    // Committed roots have no release path. NativeLibrary then only borrows it.
    mapping: AtomicUsize,
}

pub(crate) const SERVICE: &str = "shroudforge.process-lifetime";
pub(crate) const RESTART_REQUIRED: i32 = 12;
static NEXT_GENERATION: AtomicU64 = AtomicU64::new(1);
static ADMISSIONS: LazyLock<Mutex<HashMap<String, Record>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

struct Record {
    owner: Weak<Admission>,
    // A strong identity is permanent only after explicit retention. While a
    // worker/instance is alive, its ordinary Arc also excludes another load.
    retained: Option<Arc<Admission>>,
}

pub(crate) struct Admission {
    pub(super) owner_adoption: super::owner_adoption::Slot,
    package: String,
    generation: u64,
    module: AtomicUsize,
    host: AtomicUsize,
    retained: AtomicBool,
    exposed: AtomicBool,
    preload: Option<PreloadOwner>,
}

pub(crate) fn next_generation() -> Result<u64, String> {
    allocate_generation(&NEXT_GENERATION)
}

fn allocate_generation(counter: &AtomicU64) -> Result<u64, String> {
    counter
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
            if value == 0 {
                None
            } else {
                value.checked_add(1)
            }
        })
        .map_err(|_| "native package generations exhausted; restart required".into())
}

pub(crate) fn reserve(package: &str, generation: u64) -> Result<Arc<Admission>, String> {
    reserve_inner(package, generation, None)
}

fn reserve_inner(
    package: &str,
    generation: u64,
    preload: Option<PreloadOwner>,
) -> Result<Arc<Admission>, String> {
    if package.is_empty() || generation == 0 {
        return Err("invalid native package admission identity".into());
    }
    let mut records = ADMISSIONS
        .lock()
        .map_err(|_| "native admission registry unavailable")?;
    if let Some(new) = &preload {
        if records.values().filter_map(|r| r.owner.upgrade()).any(|a| {
            a.preload
                .as_ref()
                .is_some_and(|p| p.intent.artifact.sha256 == new.intent.artifact.sha256)
        }) {
            return Err(
                "preload primary artifact already has another package/owner; restart required"
                    .into(),
            );
        }
    }
    if let Some(record) = records.get(package) {
        if record.retained.is_some() {
            return Err("native package is process-retained; restart required".into());
        }
        if record.owner.upgrade().is_some() {
            return Err("native package already has a pending or active lifecycle load".into());
        }
    }
    let owner = Arc::new(Admission {
        owner_adoption: super::owner_adoption::Slot::default(),
        package: package.to_owned(),
        generation,
        module: AtomicUsize::new(0),
        host: AtomicUsize::new(0),
        retained: AtomicBool::new(false),
        exposed: AtomicBool::new(false),
        preload,
    });
    records.insert(
        package.to_owned(),
        Record {
            owner: Arc::downgrade(&owner),
            retained: owner.preload.as_ref().map(|_| Arc::clone(&owner)),
        },
    );
    Ok(owner)
}

pub(crate) fn reserve_preload(
    package: &str,
    generation: u64,
    intent: PreloadIntent,
) -> Result<Arc<Admission>, String> {
    intent.artifact.ensure_unmapped()?;
    let owner = reserve_inner(
        package,
        generation,
        Some(PreloadOwner {
            intent,
            creator: std::thread::current().id(),
            phase: AtomicUsize::new(PREPARED),
            host_image: AtomicUsize::new(0),
            mapping: AtomicUsize::new(0),
        }),
    )?;
    // The full strong owner is already in the existing registry. No registry
    // lock crosses the OS call, and no fallible operation follows successful PIN.
    let preload = owner.preload.as_ref().expect("constructed preload owner");
    let Some(image) = pin_module_identity(reserve_preload as *const () as usize) else {
        if let Ok(mut records) = ADMISSIONS.lock() {
            if records
                .get(package)
                .and_then(|r| r.owner.upgrade())
                .is_some_and(|a| Arc::ptr_eq(&a, &owner))
            {
                records.remove(package);
            }
        }
        return Err("preload host image PIN failed before target load".into());
    };
    preload.host_image.store(image, Ordering::Release);
    preload.phase.store(HOST_PINNED, Ordering::Release);
    owner.retained.store(true, Ordering::Release);
    Ok(owner)
}

pub(crate) fn preload_is_sticky(package: &str) -> bool {
    ADMISSIONS
        .lock()
        .ok()
        .and_then(|r| r.get(package).and_then(|r| r.owner.upgrade()))
        .is_some_and(|o| o.is_preload() && o.is_retained())
}

pub(crate) fn owner(package: &str, generation: u64) -> Option<Arc<Admission>> {
    let records = ADMISSIONS.lock().ok()?;
    let admission = records.get(package)?.owner.upgrade()?;
    (admission.generation == generation).then_some(admission)
}

impl Admission {
    pub(super) fn owner_adoption_module(
        self: &Arc<Self>,
        host_identity: usize,
        generation: u64,
        target: u32,
    ) -> Result<usize, i32> {
        self.validate(&self.package, generation)
            .map_err(|_| NATIVE_STATUS_UNAVAILABLE)?;
        let p = self.preload.as_ref().ok_or(NATIVE_STATUS_UNAVAILABLE)?;
        if p.creator != std::thread::current().id()
            || p.phase.load(Ordering::Acquire) != ADOPTED
            || p.intent.process_target != target
            || !self.preload_host_pinned()
            || !self.exposed.load(Ordering::Acquire)
            || !self.is_bound_host(host_identity)
        {
            return Err(NATIVE_STATUS_UNAVAILABLE);
        }
        Ok(self.module.load(Ordering::Acquire))
    }
    #[cfg(test)]
    pub(super) fn preload_test_snapshot(&self) -> (usize, usize, usize, bool) {
        (
            self.preload
                .as_ref()
                .map_or(0, |p| p.phase.load(Ordering::Acquire)),
            self.host.load(Ordering::Acquire),
            self.preload
                .as_ref()
                .map_or(0, |p| p.mapping.load(Ordering::Acquire)),
            self.exposed.load(Ordering::Acquire),
        )
    }
    pub(crate) fn is_preload(&self) -> bool {
        self.preload.is_some()
    }
    pub(crate) fn root_cookie(&self) -> (u64, usize) {
        (self.generation, self as *const Self as usize)
    }
    pub(crate) fn preload_target(&self) -> Option<u32> {
        self.preload.as_ref().map(|p| p.intent.process_target)
    }
    pub(crate) fn preload_package_matches(
        &self,
        binding: &mod_loader::package_lookup::ModPackageBinding,
        relative: &str,
    ) -> bool {
        self.preload.as_ref().is_some_and(|p| {
            p.intent.package_binding == *binding && p.intent.relative_path == relative
        })
    }
    pub(crate) fn claim_preload_worker(&self) -> Result<(), String> {
        if !self.preload_host_pinned() {
            return Err("preload host has not committed its PIN".into());
        }
        let p = self.preload.as_ref().ok_or("not a preload owner")?;
        p.phase
            .compare_exchange(HOST_PINNED, LOADING, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| "preload worker cancelled, duplicated or stale")?;
        Ok(())
    }
    pub(crate) fn preload_host_pinned(&self) -> bool {
        self.is_retained()
            && self
                .preload
                .as_ref()
                .is_some_and(|p| p.host_image.load(Ordering::Acquire) != 0)
    }
    pub(crate) fn record_preload_mapping(&self, module: usize) -> Result<(), String> {
        let p = self.preload.as_ref().ok_or("not a preload owner")?;
        if module == 0 {
            return Err("absent preload mapping".into());
        }
        p.mapping
            .compare_exchange(0, module, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| "preload mapping already owned")?;
        Ok(())
    }
    pub(crate) fn bind_preload_mapping(&self, module: usize) -> Result<(), String> {
        let p = self.preload.as_ref().ok_or("not a preload owner")?;
        if p.mapping.load(Ordering::Acquire) != module {
            return Err("preload mapping reference is not owned by this root".into());
        }
        p.intent.artifact.verify_mapping(module)?;
        self.bind_module(module)?;
        if !pin_module(module) {
            return Err("preload target PIN failed; mapping retained".into());
        }
        p.phase
            .compare_exchange(LOADING, MODULE_BOUND, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| "preload mapping completed after cancellation")?;
        Ok(())
    }
    pub(crate) fn begin_preload_adoption(
        self: &Arc<Self>,
        package: &str,
        generation: u64,
        target: u32,
        module: usize,
        entry: usize,
    ) -> Result<(), String> {
        self.validate(package, generation)?;
        let p = self.preload.as_ref().ok_or("not a preload owner")?;
        if p.creator != std::thread::current().id()
            || p.intent.process_target != target
            || self.module.load(Ordering::Acquire) != module
            || !address_in_module(entry, module)
        {
            return Err("preload adoption thread, target or mapped entry mismatch".into());
        }
        p.phase
            .compare_exchange(MODULE_BOUND, ADOPTED, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| "preload adoption is one-time and was cancelled or consumed")?;
        Ok(())
    }
    pub(crate) fn cancel_preload(&self) {
        if let Some(p) = &self.preload {
            let _ = p
                .phase
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |phase| {
                    matches!(phase, HOST_PINNED | LOADING | MODULE_BOUND).then_some(CANCELLED)
                });
        }
    }
    pub(crate) fn fail_preload(&self) {
        if let Some(p) = &self.preload {
            let _ = p
                .phase
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |phase| {
                    (phase != CANCELLED).then_some(FAILED)
                });
            if !self.exposed.load(Ordering::Acquire) {
                self.host.store(0, Ordering::Release);
            }
        }
    }
    pub(super) fn mark_host_exposed(&self) {
        self.exposed.store(true, Ordering::Release);
    }
    pub(super) fn retain_host_boxes_on_drop(&self) -> bool {
        self.is_retained() && self.exposed.load(Ordering::Acquire)
    }
    pub(crate) fn is_bound_host(&self, identity: usize) -> bool {
        identity != 0 && self.host.load(Ordering::Acquire) == identity
    }
    /// Private bootstrap diagnostic controller, not SDK adoption. The caller
    /// has already selected/hashed the exact external diagnostic and owns its
    /// real NativeLibrary plus a stable, process-retained controller identity.
    pub(crate) fn retain_early_controller(self: &Arc<Self>, identity: usize) -> Result<(), String> {
        self.bind_host(identity)?;
        let status = self.retain(identity);
        if status != NATIVE_STATUS_OK {
            return Err(format!(
                "early diagnostic process retention refused: {status}"
            ));
        }
        Ok(())
    }
    pub(super) fn bind_host(&self, identity: usize) -> Result<(), String> {
        if identity == 0
            || self
                .host
                .compare_exchange(0, identity, Ordering::AcqRel, Ordering::Acquire)
                .is_err()
        {
            return Err("native lifecycle generation already has a host instance".into());
        }
        Ok(())
    }
    pub(crate) fn bind_module(&self, module: usize) -> Result<(), String> {
        let records = ADMISSIONS
            .lock()
            .map_err(|_| "native admission registry unavailable")?;
        if records
            .values()
            .filter_map(|r| r.owner.upgrade())
            .any(|other| {
                !std::ptr::eq(&*other, self)
                    && (self.is_preload() || other.is_preload())
                    && module != 0
                    && other.module.load(Ordering::Acquire) == module
            })
        {
            return Err("mapped module already belongs to another preload/lifecycle owner".into());
        }
        if module == 0
            || self
                .module
                .compare_exchange(0, module, Ordering::AcqRel, Ordering::Acquire)
                .is_err()
        {
            return Err("native admission DLL identity is absent or already bound".into());
        }
        Ok(())
    }

    pub(crate) fn validate(self: &Arc<Self>, package: &str, generation: u64) -> Result<(), String> {
        let records = ADMISSIONS
            .lock()
            .map_err(|_| "native admission registry unavailable")?;
        let current = records
            .get(package)
            .and_then(|record| record.owner.upgrade());
        if self.package != package
            || self.generation != generation
            || self.module.load(Ordering::Acquire) == 0
            || self
                .preload
                .as_ref()
                .is_some_and(|p| !matches!(p.phase.load(Ordering::Acquire), MODULE_BOUND | ADOPTED))
            || !current
                .as_ref()
                .is_some_and(|owner| Arc::ptr_eq(owner, self))
        {
            return Err("stale native DLL completion or lifecycle owner".into());
        }
        Ok(())
    }

    fn retain(self: &Arc<Self>, host_identity: usize) -> i32 {
        let Ok(mut records) = ADMISSIONS.lock() else {
            return NATIVE_STATUS_FAILED;
        };
        let Some(record) = records.get_mut(&self.package) else {
            return NATIVE_STATUS_CONFLICT;
        };
        let Some(current) = record.owner.upgrade() else {
            return NATIVE_STATUS_CONFLICT;
        };
        if !Arc::ptr_eq(&current, self)
            || host_identity == 0
            || self.host.load(Ordering::Acquire) != host_identity
        {
            return NATIVE_STATUS_CONFLICT;
        }
        if self.retained.load(Ordering::Acquire) {
            if self.preload.as_ref().is_some_and(|p| {
                p.phase.load(Ordering::Acquire) != ADOPTED
                    || self.module.load(Ordering::Acquire) == 0
                    || !self.exposed.load(Ordering::Acquire)
            }) {
                return NATIVE_STATUS_UNAVAILABLE;
            }
            return NATIVE_STATUS_OK;
        }
        let module = self.module.load(Ordering::Acquire);
        if module == 0
            || !pin_module(module)
            || !pin_module(retain_until_process_exit as *const () as usize)
        {
            return NATIVE_STATUS_UNAVAILABLE;
        }
        // No fallible operation after commit. The library/instance owner sees
        // this latch even on initialization failure. Pins keep this registry's
        // code/image and the admitted plugin mapped across host FreeLibrary.
        record.retained = Some(Arc::clone(self));
        self.retained.store(true, Ordering::Release);
        NATIVE_STATUS_OK
    }

    pub(crate) fn is_retained(&self) -> bool {
        self.retained.load(Ordering::Acquire)
    }
}

fn pin_module(address: usize) -> bool {
    pin_module_identity(address).is_some()
}
fn pin_module_identity(address: usize) -> Option<usize> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::LibraryLoader::{
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_PIN,
            GetModuleHandleExW,
        };
        let mut module = std::ptr::null_mut();
        unsafe {
            (GetModuleHandleExW(
                GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_PIN,
                address as *const u16,
                &mut module,
            ) != 0)
                .then_some(module as usize)
        }
    }
    #[cfg(not(windows))]
    {
        let _ = address;
        None
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct RetentionResult {
    pub(super) size: u32,
    pub(super) version: u32,
    pub(super) package_generation: u64,
    pub(super) retained: u32,
    pub(super) restart_required: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct Service {
    pub(super) size: u32,
    pub(super) version: u32,
    pub(super) context: *mut c_void,
    pub(super) retain_until_process_exit:
        Option<unsafe extern "C" fn(*mut c_void, u32, *mut RetentionResult) -> i32>,
}

pub(super) unsafe fn query(context: *mut c_void, capacity: u32, output: *mut c_void) -> i32 {
    let Some(owner) = (unsafe { host_context(context) }) else {
        return NATIVE_STATUS_INVALID_ARGUMENT;
    };
    if !owner.initializing.get() || owner.process_admission.is_none() {
        return NATIVE_STATUS_UNAVAILABLE;
    }
    if capacity < size_of::<Service>() as u32 {
        return NATIVE_STATUS_BUFFER_TOO_SMALL;
    }
    if output.is_null() {
        return NATIVE_STATUS_INVALID_ARGUMENT;
    }
    unsafe {
        output.cast::<Service>().write(Service {
            size: size_of::<Service>() as u32,
            version: 1,
            context,
            retain_until_process_exit: Some(retain_until_process_exit),
        });
    }
    NATIVE_STATUS_OK
}

unsafe extern "C" fn retain_until_process_exit(
    context: *mut c_void,
    out_size: u32,
    output: *mut RetentionResult,
) -> i32 {
    let Some(owner) = (unsafe { host_context(context) }) else {
        return NATIVE_STATUS_INVALID_ARGUMENT;
    };
    if !owner.is_owner_thread() {
        return NATIVE_STATUS_WRONG_THREAD;
    }
    if owner.services_revoked.get() || !owner.initializing.get() {
        return NATIVE_STATUS_UNAVAILABLE;
    }
    if out_size < size_of::<RetentionResult>() as u32 {
        return NATIVE_STATUS_BUFFER_TOO_SMALL;
    }
    if output.is_null() {
        return NATIVE_STATUS_INVALID_ARGUMENT;
    }
    let Some(admission) = &owner.process_admission else {
        return NATIVE_STATUS_UNAVAILABLE;
    };
    let status = admission.retain(context as usize);
    if status != NATIVE_STATUS_OK {
        return status;
    }
    unsafe {
        output.write(RetentionResult {
            size: size_of::<RetentionResult>() as u32,
            version: 1,
            package_generation: owner.package_generation,
            retained: 1,
            restart_required: 1,
        });
    }
    NATIVE_STATUS_OK
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity() -> (String, u64) {
        let generation = next_generation().unwrap();
        (format!("process-lifetime-test-{generation}"), generation)
    }
    #[test]
    fn generations_never_wrap_or_reuse_zero() {
        let counter = AtomicU64::new(u64::MAX - 1);
        assert_eq!(allocate_generation(&counter).unwrap(), u64::MAX - 1);
        assert!(allocate_generation(&counter).is_err());
        assert!(allocate_generation(&counter).is_err());
        assert_eq!(counter.load(Ordering::Relaxed), u64::MAX);
        assert!(allocate_generation(&AtomicU64::new(0)).is_err());
    }
    #[test]
    fn pending_worker_lease_excludes_load_and_survives_receiver_cancellation() {
        let (package, generation) = identity();
        let pending = reserve(&package, generation).unwrap();
        let worker = Arc::clone(&pending);
        drop(pending); // AppState receiver destroyed while LoadLibrary is pending.
        assert!(reserve(&package, generation + 1).is_err());
        drop(worker);
        assert!(reserve(&package, generation + 1).is_ok());
    }
    #[test]
    fn stale_async_completion_cannot_enter_another_owner() {
        let (package, generation) = identity();
        let pending = reserve(&package, generation).unwrap();
        assert!(pending.validate(&package, generation).is_err()); // no loaded image
        pending.bind_module(1).unwrap();
        assert!(pending.validate(&package, generation).is_ok());
        assert!(pending.validate(&package, generation + 1).is_err());
        assert!(pending.validate("another-package", generation).is_err());
        assert!(pending.bind_module(2).is_err());
    }
    #[test]
    fn failed_retention_has_no_committed_tombstone() {
        let (package, generation) = identity();
        let pending = reserve(&package, generation).unwrap();
        pending.bind_host(1).unwrap();
        assert_eq!(pending.retain(1), NATIVE_STATUS_UNAVAILABLE); // no loaded module
        assert!(!pending.is_retained());
        drop(pending);
        assert!(reserve(&package, generation + 1).is_ok());
    }
    #[test]
    #[cfg(windows)]
    fn retained_identity_blocks_new_session_before_load_and_is_idempotent() {
        let (package, generation) = identity();
        let first = reserve(&package, generation).unwrap();
        first
            .bind_module(retain_until_process_exit as *const () as usize)
            .unwrap();
        first.bind_host(1).unwrap();
        assert!(!first.is_retained());
        assert_eq!(first.retain(2), NATIVE_STATUS_CONFLICT);
        assert!(!first.is_retained());
        assert_eq!(first.retain(1), NATIVE_STATUS_OK);
        assert_eq!(first.retain(1), NATIVE_STATUS_OK);
        assert_eq!(first.retain(2), NATIVE_STATUS_CONFLICT);
        assert!(first.is_retained());
        drop(first); // The AppState and every non-registry identity owner are gone.
        assert!(owner(&package, generation).unwrap().is_retained());
        let mut loads = 0;
        if reserve(&package, generation + 1).is_ok() {
            loads += 1;
        }
        assert_eq!(loads, 0);
    }

    #[test]
    #[cfg(windows)]
    fn early_controller_uses_real_registry_and_exact_nonzero_host_binding() {
        let (package, generation) = identity();
        let admission = reserve(&package, generation).unwrap();
        assert!(!admission.is_bound_host(0));
        assert!(admission.retain_early_controller(0).is_err());
        admission
            .bind_module(retain_until_process_exit as *const () as usize)
            .unwrap();
        let retained_controller = Box::into_raw(Box::new(17u64));
        admission
            .retain_early_controller(retained_controller as usize)
            .unwrap();
        assert!(admission.is_bound_host(retained_controller as usize));
        assert!(!admission.is_bound_host((retained_controller as usize) + 1));
        assert!(admission.is_retained());
        assert!(
            admission
                .retain_early_controller((retained_controller as usize) + 1)
                .is_err()
        );
        drop(admission);
        assert!(reserve(&package, generation + 1).is_err());
        // Owned fixture metadata remains alive until this test process exits.
    }
}

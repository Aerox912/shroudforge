//! Owner-only strong consumer lease. No native CALL/content/support operations.
use super::*;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{
    Mutex, MutexGuard,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
pub(super) const SERVICE: &str = "shroudforge.owner-adoption";
const VACANT: u32 = 0;
const ACQUIRING: u32 = 1;
const ADOPTED: u32 = 2;
const INDETERMINATE: u32 = 3;
const REVOKED: u32 = 4;
const CLOSE_NONE: u32 = 0;
const CLOSE_ACTIVE: u32 = 1;
const CLOSE_DONE: u32 = 2;
const CLOSE_UNKNOWN: u32 = 3;
static NEXT_HANDLE: AtomicU64 = AtomicU64::new(1);
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub(super) struct Binding {
    pub size: u32,
    pub version: u32,
    pub package_generation: u64,
    pub owner_handle: u64,
    pub state_identity: *const c_void,
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub(super) struct StrongLease {
    pub size: u32,
    pub version: u32,
    pub package_generation: u64,
    pub owner_handle: u64,
    pub state_identity: *const c_void,
    pub owned_lease: *mut c_void,
}
type Acquire = unsafe extern "C" fn(*mut c_void, *const Binding, u32, *mut StrongLease) -> i32;
type Close = unsafe extern "C" fn(*mut c_void, *const Binding) -> i32;
#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct Request {
    pub size: u32,
    pub version: u32,
    pub state_identity: *const c_void,
    pub source_context: *mut c_void,
    pub acquire_strong_lease: Option<Acquire>,
    pub close_content: Option<Close>,
}
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct State {
    pub size: u32,
    pub version: u32,
    pub package_generation: u64,
    pub owner_handle: u64,
    pub state_identity: *const c_void,
    pub phase: u32,
    pub host_fenced: u32,
    pub lease_retained: u32,
    pub close_state: u32,
    pub native_operations_available: u32,
    pub last_callback_status: i32,
}
#[repr(C)]
pub(super) struct Service {
    pub size: u32,
    pub version: u32,
    pub context: *mut c_void,
    pub adopt:
        Option<unsafe extern "C" fn(*mut c_void, u32, *const Request, u32, *mut State) -> i32>,
    pub read_status: Option<unsafe extern "C" fn(*mut c_void, u64, u32, *mut State) -> i32>,
    pub revoke: Option<unsafe extern "C" fn(*mut c_void, u64) -> i32>,
}

// Thread-safe host record contains opaque values, not C++ objects. The owned
// unit's strong-reference semantics are acquired through the module-bound thunk.
// Its representation is never dereferenced by Rust and has no release path.
#[derive(Default, Clone, Copy)]
struct RawLease {
    size: u32,
    version: u32,
    generation: u64,
    handle: u64,
    state: usize,
    owned: usize,
}
impl From<StrongLease> for RawLease {
    fn from(v: StrongLease) -> Self {
        Self {
            size: v.size,
            version: v.version,
            generation: v.package_generation,
            handle: v.owner_handle,
            state: v.state_identity as usize,
            owned: v.owned_lease as usize,
        }
    }
}
#[derive(Default)]
struct Record {
    generation: u64,
    handle: u64,
    state: usize,
    phase: u32,
    lease: RawLease,
    valid: bool,
    close: Option<Close>,
    close_state: u32,
    last: i32,
}
impl Record {
    fn binding(&self) -> Binding {
        Binding {
            size: size_of::<Binding>() as u32,
            version: 1,
            package_generation: self.generation,
            owner_handle: self.handle,
            state_identity: self.state as *const c_void,
        }
    }
    fn valid_lease(&self, lease: RawLease) -> bool {
        lease.size == size_of::<StrongLease>() as u32
            && lease.version == 1
            && lease.generation == self.generation
            && lease.handle == self.handle
            && lease.state == self.state
            && lease.owned != 0
    }
}
#[derive(Default)]
pub(super) struct Slot {
    record: Mutex<Record>,
    active: AtomicBool,
    fenced: AtomicBool,
    #[cfg(test)]
    pub(super) panic_after_acquire: AtomicBool,
    #[cfg(test)]
    pub(super) panic_after_close: AtomicBool,
}
impl Slot {
    fn lock(&self) -> MutexGuard<'_, Record> {
        match self.record.lock() {
            Ok(r) => r,
            Err(p) => {
                self.fenced.store(true, Ordering::Release);
                let mut r = p.into_inner();
                if r.phase != VACANT {
                    r.phase = INDETERMINATE;
                }
                if r.close_state == CLOSE_ACTIVE {
                    r.close_state = CLOSE_UNKNOWN;
                }
                r
            }
        }
    }
    pub(super) fn is_active(&self) -> bool {
        self.active.load(Ordering::Acquire)
    }
    fn snapshot(&self) -> State {
        let r = self.lock();
        State {
            size: size_of::<State>() as u32,
            version: 1,
            package_generation: r.generation,
            owner_handle: r.handle,
            state_identity: r.state as *const c_void,
            phase: r.phase,
            host_fenced: u32::from(self.fenced.load(Ordering::Acquire)),
            lease_retained: u32::from(r.valid),
            close_state: r.close_state,
            native_operations_available: 0,
            last_callback_status: r.last,
        }
    }
    fn indeterminate(&self) {
        self.fenced.store(true, Ordering::Release);
        let mut r = self.lock();
        if r.phase != VACANT {
            r.phase = INDETERMINATE;
        }
        if r.close_state == CLOSE_ACTIVE {
            r.close_state = CLOSE_UNKNOWN;
        }
    }
    fn known(&self, handle: u64) -> Result<(), i32> {
        if handle == 0 {
            return Err(NATIVE_STATUS_INVALID_ARGUMENT);
        }
        if self.lock().handle != handle {
            return Err(NATIVE_STATUS_CONFLICT);
        }
        Ok(())
    }
}
struct Active<'a>(&'a Slot);
impl<'a> Active<'a> {
    fn acquire(slot: &'a Slot) -> Result<Self, i32> {
        slot.active
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| NATIVE_STATUS_BUSY)?;
        Ok(Self(slot))
    }
}
impl Drop for Active<'_> {
    fn drop(&mut self) {
        self.0.active.store(false, Ordering::Release);
    }
}
struct Attempt<'a> {
    slot: &'a Slot,
    scratch: StrongLease,
    last: i32,
    finished: bool,
}
impl Drop for Attempt<'_> {
    fn drop(&mut self) {
        if !self.finished {
            self.slot.fenced.store(true, Ordering::Release);
            let mut r = self.slot.lock();
            let raw = RawLease::from(self.scratch);
            r.valid = r.valid_lease(raw);
            r.lease = raw;
            if r.close_state == CLOSE_NONE {
                r.last = self.last;
            }
            r.phase = INDETERMINATE;
            if r.close_state == CLOSE_ACTIVE {
                r.close_state = CLOSE_UNKNOWN;
            }
        }
    }
}
struct CloseAttempt<'a> {
    slot: &'a Slot,
    last: Option<i32>,
    finished: bool,
}
impl Drop for CloseAttempt<'_> {
    fn drop(&mut self) {
        if !self.finished {
            self.slot.indeterminate();
            if let Some(status) = self.last {
                self.slot.lock().last = status;
            }
        }
    }
}
#[derive(Default)]
struct Track {
    owner: Option<Arc<process_lifetime::Admission>>,
    started: bool,
}

fn allocate_handle(counter: &AtomicU64) -> Result<u64, i32> {
    counter
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |v| {
            if v == 0 { None } else { v.checked_add(1) }
        })
        .map_err(|_| NATIVE_STATUS_UNAVAILABLE)
}
unsafe fn caller<'a>(pointer: *mut c_void) -> Result<&'a NativePluginHostContext, i32> {
    let context = unsafe { host_context(pointer) }.ok_or(NATIVE_STATUS_INVALID_ARGUMENT)?;
    // Do not inspect Cell/RefCell or any mutable context state before this check.
    if !context.is_owner_thread() {
        return Err(NATIVE_STATUS_WRONG_THREAD);
    }
    Ok(context)
}
fn ready(
    context: &NativePluginHostContext,
    initialize: bool,
) -> Result<(Arc<process_lifetime::Admission>, usize), i32> {
    if initialize && (!context.initializing.get() || context.services_revoked.get()) {
        return Err(NATIVE_STATUS_UNAVAILABLE);
    }
    if !context
        .package
        .info()
        .capabilities
        .contains(&mod_loader::Capability::RuntimeRegisterDll)
    {
        return Err(NATIVE_STATUS_UNAVAILABLE);
    }
    let owner = context
        .process_admission
        .as_ref()
        .ok_or(NATIVE_STATUS_UNAVAILABLE)?
        .clone();
    let module = owner.owner_adoption_module(
        context as *const _ as usize,
        context.package_generation,
        context.process_target,
    )?;
    Ok((owner, module))
}

fn executable_primary(address: usize, module: usize) -> bool {
    if !crate::session::native_plugins::preload_artifact::address_in_module(address, module) {
        return false;
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
        #[repr(C)]
        struct Memory {
            base: usize,
            allocation: usize,
            allocation_protect: u32,
            partition: u16,
            padding: u16,
            region: usize,
            state: u32,
            protect: u32,
            kind: u32,
            padding2: u32,
        }
        type Query = unsafe extern "system" fn(*const c_void, *mut Memory, usize) -> usize;
        const KERNEL: [u16; 13] = [107, 101, 114, 110, 101, 108, 51, 50, 46, 100, 108, 108, 0];
        let library = unsafe { GetModuleHandleW(KERNEL.as_ptr()) };
        if library.is_null() {
            return false;
        }
        let Some(symbol) = (unsafe { GetProcAddress(library, b"VirtualQuery\0".as_ptr()) }) else {
            return false;
        };
        let query: Query = unsafe { std::mem::transmute(symbol) };
        let mut m = std::mem::MaybeUninit::<Memory>::uninit();
        if unsafe {
            query(
                address as *const c_void,
                m.as_mut_ptr(),
                size_of::<Memory>(),
            )
        } != size_of::<Memory>()
        {
            return false;
        }
        let m = unsafe { m.assume_init() };
        m.allocation == module
            && m.state == 0x1000
            && m.kind == 0x1000000
            && m.protect & 0x100 == 0
            && matches!(m.protect & 0xff, 0x10 | 0x20 | 0x40 | 0x80)
            && address >= m.base
            && m.base
                .checked_add(m.region)
                .is_some_and(|end| address < end)
    }
    #[cfg(not(windows))]
    {
        false
    }
}

pub(super) unsafe fn query(pointer: *mut c_void, capacity: u32, output: *mut c_void) -> i32 {
    catch_unwind(AssertUnwindSafe(|| {
        let context = match unsafe { caller(pointer) } {
            Ok(c) => c,
            Err(e) => return e,
        };
        if let Err(e) = ready(context, true) {
            return e;
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
                context: pointer,
                adopt: Some(adopt),
                read_status: Some(read_status),
                revoke: Some(revoke),
            })
        };
        NATIVE_STATUS_OK
    }))
    .unwrap_or(NATIVE_STATUS_FAILED)
}

unsafe extern "C" fn adopt(
    pointer: *mut c_void,
    request_size: u32,
    request: *const Request,
    out_size: u32,
    output: *mut State,
) -> i32 {
    let mut track = Track::default();
    let result = catch_unwind(AssertUnwindSafe(|| {
        adopt_inner(pointer, request_size, request, out_size, output, &mut track)
    }));
    match result {
        Ok(Ok((status, state))) => {
            unsafe { output.write(state) };
            status
        }
        result => {
            if track.started {
                let owner = track.owner.as_ref().expect("attempt owns root");
                owner.owner_adoption.indeterminate();
                unsafe { output.write(owner.owner_adoption.snapshot()) };
                process_lifetime::RESTART_REQUIRED
            } else {
                match result {
                    Ok(Err(status)) => status,
                    _ => NATIVE_STATUS_FAILED,
                }
            }
        }
    }
}
fn adopt_inner(
    pointer: *mut c_void,
    request_size: u32,
    request: *const Request,
    out_size: u32,
    output: *mut State,
    track: &mut Track,
) -> Result<(i32, State), i32> {
    let context = unsafe { caller(pointer) }?;
    let (owner, module) = ready(context, true)?;
    if request_size != size_of::<Request>() as u32 || request.is_null() || output.is_null() {
        return Err(NATIVE_STATUS_INVALID_ARGUMENT);
    }
    if out_size < size_of::<State>() as u32 {
        return Err(NATIVE_STATUS_BUFFER_TOO_SMALL);
    }
    let request = unsafe { request.read() };
    if request.size != size_of::<Request>() as u32 {
        return Err(NATIVE_STATUS_INVALID_ARGUMENT);
    }
    if request.version != 1 {
        return Err(NATIVE_STATUS_ABI_MISMATCH);
    }
    let acquire = request
        .acquire_strong_lease
        .ok_or(NATIVE_STATUS_INVALID_ARGUMENT)?;
    let close = request
        .close_content
        .ok_or(NATIVE_STATUS_INVALID_ARGUMENT)?;
    if request.state_identity.is_null() || request.source_context.is_null() {
        return Err(NATIVE_STATUS_INVALID_ARGUMENT);
    }
    if !executable_primary(acquire as *const () as usize, module)
        || !executable_primary(close as *const () as usize, module)
    {
        return Err(NATIVE_STATUS_UNAVAILABLE);
    }
    track.owner = Some(owner.clone());
    let slot = &owner.owner_adoption;
    let _active = Active::acquire(slot)?;
    let binding = {
        let mut r = slot.lock();
        if r.phase != VACANT {
            return Err(NATIVE_STATUS_CONFLICT);
        }
        if slot.fenced.load(Ordering::Acquire) {
            return Err(NATIVE_STATUS_UNAVAILABLE);
        }
        if !context.initializing.get() || context.services_revoked.get() {
            return Err(NATIVE_STATUS_UNAVAILABLE);
        }
        let handle = allocate_handle(&NEXT_HANDLE)?;
        r.generation = context.package_generation;
        r.handle = handle;
        r.state = request.state_identity as usize;
        r.close = Some(close);
        r.phase = ACQUIRING;
        track.started = true;
        r.binding()
    };
    let mut attempt = Attempt {
        slot,
        scratch: StrongLease::default(),
        last: NATIVE_STATUS_FAILED,
        finished: false,
    };
    let callback_status = unsafe {
        acquire(
            request.source_context,
            &binding,
            size_of::<StrongLease>() as u32,
            &mut attempt.scratch,
        )
    };
    attempt.last = callback_status;
    #[cfg(test)]
    if slot.panic_after_acquire.swap(false, Ordering::AcqRel) {
        panic!("owner adoption post-acquire test panic");
    }
    let current = ready(context, true).is_ok();
    {
        let mut r = slot.lock();
        let raw = RawLease::from(attempt.scratch);
        r.valid = r.valid_lease(raw);
        r.lease = raw;
        r.last = callback_status;
        if callback_status == NATIVE_STATUS_OK
            && r.valid
            && current
            && !slot.fenced.load(Ordering::Acquire)
        {
            r.phase = ADOPTED;
        } else {
            slot.fenced.store(true, Ordering::Release);
            r.phase = INDETERMINATE;
        }
    }
    // Reentrant/internal revoke can only close after the ownership unit is known.
    if slot.fenced.load(Ordering::Acquire) {
        close_once(&owner, module);
    }
    attempt.finished = true;
    let state = slot.snapshot();
    Ok((
        if state.phase == ADOPTED {
            NATIVE_STATUS_OK
        } else {
            process_lifetime::RESTART_REQUIRED
        },
        state,
    ))
}

unsafe extern "C" fn read_status(
    pointer: *mut c_void,
    handle: u64,
    out_size: u32,
    output: *mut State,
) -> i32 {
    catch_unwind(AssertUnwindSafe(|| {
        let result = (|| {
            let context = unsafe { caller(pointer) }?;
            let (owner, _) = ready(context, false)?;
            if output.is_null() {
                return Err(NATIVE_STATUS_INVALID_ARGUMENT);
            }
            if out_size < size_of::<State>() as u32 {
                return Err(NATIVE_STATUS_BUFFER_TOO_SMALL);
            }
            owner.owner_adoption.known(handle)?;
            if owner.owner_adoption.is_active() {
                return Err(NATIVE_STATUS_BUSY);
            }
            let state = owner.owner_adoption.snapshot();
            unsafe { output.write(state) };
            Ok(())
        })();
        result.map_or_else(|e| e, |_| NATIVE_STATUS_OK)
    }))
    .unwrap_or(NATIVE_STATUS_FAILED)
}

fn close_once(owner: &Arc<process_lifetime::Admission>, module: usize) {
    let slot = &owner.owner_adoption;
    let work = {
        let mut r = slot.lock();
        if !r.valid || r.close_state != CLOSE_NONE {
            return;
        }
        r.close_state = CLOSE_ACTIVE;
        (r.close, r.lease.owned, r.binding(), r.phase == ADOPTED)
    };
    let mut guard = CloseAttempt {
        slot,
        last: None,
        finished: false,
    };
    let result = work
        .0
        .filter(|f| executable_primary(*f as *const () as usize, module))
        .map(|f| unsafe { f(work.1 as *mut c_void, &work.2) });
    guard.last = result;
    #[cfg(test)]
    if slot.panic_after_close.swap(false, Ordering::AcqRel) {
        panic!("owner adoption post-close test panic");
    }
    {
        let mut r = slot.lock();
        if let Some(status) = result {
            r.last = status;
        }
        if result == Some(NATIVE_STATUS_OK) {
            r.close_state = CLOSE_DONE;
            r.phase = if work.3 { REVOKED } else { INDETERMINATE };
        } else {
            r.close_state = CLOSE_UNKNOWN;
            r.phase = INDETERMINATE;
        }
    }
    guard.finished = true;
}

fn revoke_owner(owner: &Arc<process_lifetime::Admission>, module: usize) -> i32 {
    let slot = &owner.owner_adoption;
    slot.fenced.store(true, Ordering::Release);
    let _active = match Active::acquire(slot) {
        Ok(a) => a,
        Err(e) => return e,
    };
    if slot.lock().phase == VACANT {
        return NATIVE_STATUS_OK;
    }
    let result = catch_unwind(AssertUnwindSafe(|| close_once(owner, module)));
    if result.is_err() {
        slot.indeterminate();
    }
    if slot.snapshot().phase == REVOKED {
        NATIVE_STATUS_OK
    } else {
        process_lifetime::RESTART_REQUIRED
    }
}
unsafe extern "C" fn revoke(pointer: *mut c_void, handle: u64) -> i32 {
    catch_unwind(AssertUnwindSafe(|| {
        let context = match unsafe { caller(pointer) } {
            Ok(c) => c,
            Err(e) => return e,
        };
        let (owner, module) = match ready(context, false) {
            Ok(v) => v,
            Err(e) => return e,
        };
        if let Err(e) = owner.owner_adoption.known(handle) {
            return e;
        }
        revoke_owner(&owner, module)
    }))
    .unwrap_or(process_lifetime::RESTART_REQUIRED)
}
pub(super) fn host_revoke(context: &NativePluginHostContext) -> i32 {
    let Some(owner) = &context.process_admission else {
        return NATIVE_STATUS_OK;
    };
    owner.owner_adoption.fenced.store(true, Ordering::Release);
    if owner.owner_adoption.lock().phase == VACANT {
        return NATIVE_STATUS_OK;
    }
    match ready(context, false) {
        Ok((root, module)) => revoke_owner(&root, module),
        Err(e) => e,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn owner_adoption_layout_and_nonwrapping_handles() {
        assert_eq!(size_of::<Binding>(), 32);
        assert_eq!(size_of::<StrongLease>(), 40);
        assert_eq!(size_of::<Request>(), 40);
        assert_eq!(size_of::<State>(), 56);
        assert_eq!(size_of::<Service>(), 40);
        let counter = AtomicU64::new(u64::MAX - 1);
        assert_eq!(allocate_handle(&counter), Ok(u64::MAX - 1));
        assert_eq!(allocate_handle(&counter), Err(NATIVE_STATUS_UNAVAILABLE));
    }
}

#[cfg(all(test, windows))]
#[path = "owner_adoption_tests.rs"]
mod fixture_tests;

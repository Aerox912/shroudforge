use super::*;
use crate::session::native_plugin_abi::tests::TestPackage;
use crate::session::native_plugins::{
    NativeLibrary, preload_artifact::PreloadArtifact, spawn_admitted_native_load,
};
use std::{fs, io::Write, time::Duration};
#[repr(C)]
#[derive(Default, Debug)]
struct Observation {
    size: u32,
    version: u32,
    state_alive: u32,
    strong_count: u32,
    guard_retained: u32,
    acquire_calls: u32,
    close_calls: u32,
    destructor_calls: u32,
    last_status: i32,
    last_phase: u32,
    generation: u64,
    handle: u64,
    identity: u64,
}
unsafe fn symbol<T: Copy>(library: &NativeLibrary, name: &[u8]) -> T {
    let address = unsafe {
        windows_sys::Win32::System::LibraryLoader::GetProcAddress(
            library.handle as _,
            name.as_ptr(),
        )
    }
    .unwrap();
    assert_eq!(size_of::<T>(), size_of_val(&address));
    unsafe { std::mem::transmute_copy(&address) }
}
struct Fixture {
    _package: TestPackage,
    owner: Arc<process_lifetime::Admission>,
    library: NativeLibrary,
    instance: Option<NativePluginInstance>,
}
impl Fixture {
    fn new(mode: u32) -> Self {
        Self::with_override(mode, None)
    }
    fn with_override(mode: u32, override_acquire: Option<Acquire>) -> Self {
        Self::construct(mode, override_acquire, true)
    }
    fn construct(mode: u32, override_acquire: Option<Acquire>, unique_overlay: bool) -> Self {
        let generation = process_lifetime::next_generation().unwrap();
        let id = format!("owner-adoption-{mode}-{generation}");
        let package = TestPackage::named(false, "client", &id);
        let path = package
            .package
            .fs()
            .root()
            .as_std_path()
            .join(format!("owner-{generation}.dll"));
        fs::copy(
            std::env::var_os("SF_OWNER_ADOPTION_FIXTURE_DLL")
                .expect("compile owned C++ fixture and set SF_OWNER_ADOPTION_FIXTURE_DLL"),
            &path,
        )
        .unwrap();
        if unique_overlay {
            writeln!(
                fs::OpenOptions::new().append(true).open(&path).unwrap(),
                "SF-RCALL-03:{generation}:{mode}"
            )
            .unwrap();
        }
        let owner = process_lifetime::reserve_preload(
            &id,
            generation,
            process_lifetime::PreloadIntent {
                artifact: PreloadArtifact::open(&path).unwrap(),
                package_binding: package.package_binding(),
                relative_path: path.file_name().unwrap().to_str().unwrap().into(),
                process_target: NATIVE_TARGET_CLIENT,
            },
        )
        .unwrap();
        let task =
            spawn_admitted_native_load(id.clone(), path, None, Some(owner.clone()), || {}).unwrap();
        let library = task.recv_timeout(Duration::from_secs(10)).unwrap().unwrap();
        task.worker.join().unwrap();
        // Generic component replay requires only the two common exports. Host-self
        // negative modes use additional private controls and unique inert overlays.
        if unique_overlay {
            let configure: unsafe extern "C" fn(u32) =
                unsafe { symbol(&library, b"SFOwnerHostFixtureModeV1\0") };
            unsafe { configure(mode) };
        }
        if let Some(acquire) = override_acquire {
            let set: unsafe extern "C" fn(Option<Acquire>) =
                unsafe { symbol(&library, b"SFOwnerHostFixtureAcquireOverrideV1\0") };
            unsafe { set(Some(acquire)) };
        }
        let entry = library.entrypoint().unwrap().unwrap();
        let instance = NativePluginInstance::load_preloaded(
            entry,
            NativePluginHostMetadata {
                package_id: id,
                package: package.package.clone(),
                package_binding: package.package_binding(),
                package_generation: generation,
                is_server: false,
                installation_root: package.root.clone(),
                settings_json: b"{}".to_vec(),
                services: Arc::new(NoNativePluginServices),
            },
            owner.clone(),
            library.handle as usize,
        )
        .unwrap();
        Self {
            _package: package,
            owner,
            library,
            instance: Some(instance),
        }
    }
    fn start(&mut self) {
        self.instance.as_mut().unwrap().start().unwrap();
    }
    fn observe(&self) -> Observation {
        let read: unsafe extern "C" fn(u32, *mut Observation) -> i32 =
            unsafe { symbol(&self.library, b"SFOwnerFixtureReadV1\0") };
        let mut o = Observation::default();
        assert_eq!(unsafe { read(size_of::<Observation>() as u32, &mut o) }, 0);
        assert_eq!(o.size, 64);
        assert_eq!(o.version, 1);
        o
    }
    fn drop_locals(&self) -> i32 {
        let drop: unsafe extern "C" fn() -> i32 =
            unsafe { symbol(&self.library, b"SFOwnerFixtureDropLocalRefsV1\0") };
        unsafe { drop() }
    }
    fn data(&self) -> (Service, State, Request) {
        let get: unsafe extern "C" fn(*mut Service, *mut State, *mut Request) -> i32 =
            unsafe { symbol(&self.library, b"SFOwnerHostFixtureDataV1\0") };
        let (mut s, mut r, mut q) = (
            std::mem::MaybeUninit::uninit(),
            std::mem::MaybeUninit::uninit(),
            std::mem::MaybeUninit::uninit(),
        );
        assert_eq!(
            unsafe { get(s.as_mut_ptr(), r.as_mut_ptr(), q.as_mut_ptr()) },
            0
        );
        unsafe { (s.assume_init(), r.assume_init(), q.assume_init()) }
    }
    fn state(&self) -> State {
        let (s, r, _) = self.data();
        let mut state = State::default();
        assert_eq!(
            unsafe { s.read_status.unwrap()(s.context, r.owner_handle, 56, &mut state) },
            0
        );
        state
    }
    fn stop(&mut self) {
        assert!(matches!(
            self.instance.as_mut().unwrap().stop(),
            TeardownOutcome::Retain { .. }
        ));
    }
}

#[test]
fn owner_adoption_real_cpp_host_lease_is_sole_strong_owner_after_local_guard_drop() {
    let mut f = Fixture::new(0);
    f.start();
    let (s, r, _) = f.data();
    let o = f.observe();
    assert_eq!(
        (
            r.phase,
            r.host_fenced,
            r.lease_retained,
            r.close_state,
            r.native_operations_available,
            r.last_callback_status
        ),
        (ADOPTED, 0, 1, CLOSE_NONE, 0, 0)
    );
    assert_eq!(r.package_generation, o.generation);
    assert_eq!(r.owner_handle, o.handle);
    assert_ne!(o.handle, 0);
    assert_eq!(r.state_identity as u64, o.identity);
    assert_eq!(o.acquire_calls, 1);
    assert_eq!(f.drop_locals(), 0);
    let o = f.observe();
    assert_eq!(
        (
            o.state_alive,
            o.strong_count,
            o.guard_retained,
            o.destructor_calls
        ),
        (1, 1, 0, 0)
    );
    f.stop();
    let state = f.state();
    assert_eq!(
        (
            state.phase,
            state.host_fenced,
            state.lease_retained,
            state.close_state
        ),
        (REVOKED, 1, 1, CLOSE_DONE)
    );
    let o = f.observe();
    assert_eq!(
        (
            o.state_alive,
            o.strong_count,
            o.guard_retained,
            o.close_calls,
            o.destructor_calls
        ),
        (1, 1, 0, 1, 0)
    );
    assert_eq!(unsafe { s.revoke.unwrap()(s.context, r.owner_handle) }, 0);
    assert_eq!(f.observe().close_calls, 1);
}
#[test]
fn owner_adoption_valid_lease_callback_error_is_retained_but_never_adopted() {
    let mut f = Fixture::new(1);
    f.start();
    let state = f.state();
    assert_eq!(
        (state.phase, state.host_fenced, state.lease_retained),
        (INDETERMINATE, 1, 1)
    );
    assert_eq!(f.observe().last_status, process_lifetime::RESTART_REQUIRED);
    assert_eq!(f.drop_locals(), process_lifetime::RESTART_REQUIRED);
    let o = f.observe();
    assert_eq!(o.state_alive, 1);
    assert_eq!(o.guard_retained, 1);
    assert!(o.strong_count >= 2);
    assert_eq!(o.close_calls, 1);
}
#[test]
fn owner_adoption_bad_generation_handle_state_partial_and_caught_exception_are_quarantined() {
    for mode in [2, 3, 4, 5, 6] {
        let mut f = Fixture::new(mode);
        f.start();
        let state = f.state();
        assert_eq!(
            (
                state.phase,
                state.host_fenced,
                state.lease_retained,
                state.close_state
            ),
            (INDETERMINATE, 1, 0, CLOSE_NONE),
            "mode{mode}"
        );
        assert_eq!(f.observe().last_status, process_lifetime::RESTART_REQUIRED);
        assert_eq!(f.drop_locals(), process_lifetime::RESTART_REQUIRED);
        assert_eq!(f.observe().guard_retained, 1);
        f.stop();
        assert_eq!(f.observe().close_calls, 0);
    }
}
#[test]
fn owner_adoption_precommit_missing_acquire_version_sizes_and_nulls_do_not_attempt() {
    for (mode, status) in [(9, 1), (12, 2), (13, 1), (14, 5), (15, 1), (16, 1), (17, 1)] {
        let mut f = Fixture::new(mode);
        f.start();
        let (_, receipt, _) = f.data();
        let o = f.observe();
        assert_eq!(o.last_status, status, "mode{mode}");
        assert_eq!(o.acquire_calls, 0);
        assert_eq!(receipt, State::default());
        assert_eq!(f.owner.owner_adoption.lock().phase, VACANT);
    }
}
unsafe extern "C" fn foreign_acquire(
    _: *mut c_void,
    _: *const Binding,
    _: u32,
    _: *mut StrongLease,
) -> i32 {
    NATIVE_STATUS_FAILED
}
#[test]
fn owner_adoption_foreign_module_and_nonexecutable_primary_callback_are_refused() {
    let mut foreign = Fixture::with_override(0, Some(foreign_acquire));
    foreign.start();
    assert_eq!(foreign.observe().last_status, NATIVE_STATUS_UNAVAILABLE);
    assert_eq!(foreign.observe().acquire_calls, 0);
    assert_eq!(foreign.data().1, State::default());
    let mut data = Fixture::new(10);
    data.start();
    assert_eq!(data.observe().last_status, NATIVE_STATUS_UNAVAILABLE);
    assert_eq!(data.observe().acquire_calls, 0);
    assert_eq!(data.data().1, State::default());
}
#[test]
fn owner_adoption_reentry_is_busy_and_acquire_runs_exactly_once() {
    let mut f = Fixture::new(11);
    f.start();
    let read: unsafe extern "C" fn(*mut i32, *mut i32, *mut i32) =
        unsafe { symbol(&f.library, b"SFOwnerHostFixtureReentryV1\0") };
    let (mut r, mut v, mut d) = (0, 0, 0);
    unsafe { read(&mut r, &mut v, &mut d) };
    assert_eq!(r, NATIVE_STATUS_BUSY);
    assert_eq!(f.observe().acquire_calls, 1);
    assert_eq!(f.state().phase, ADOPTED);
    assert_eq!(f.drop_locals(), 0);
    assert_eq!(f.observe().strong_count, 1);
}
#[test]
fn owner_adoption_revoke_and_drain_during_acquire_permanently_prevent_adopted() {
    let mut f = Fixture::new(7);
    f.start();
    let read: unsafe extern "C" fn(*mut i32, *mut i32, *mut i32) =
        unsafe { symbol(&f.library, b"SFOwnerHostFixtureReentryV1\0") };
    let (mut r, mut v, mut d) = (0, 0, 0);
    unsafe { read(&mut r, &mut v, &mut d) };
    assert_eq!((v, d), (NATIVE_STATUS_BUSY, NATIVE_STATUS_BUSY));
    let state = f.state();
    assert_eq!(
        (
            state.phase,
            state.host_fenced,
            state.lease_retained,
            state.close_state
        ),
        (INDETERMINATE, 1, 1, CLOSE_DONE)
    );
    assert_eq!(f.observe().last_status, process_lifetime::RESTART_REQUIRED);
    assert_eq!(f.observe().close_calls, 1);
    assert_eq!(f.drop_locals(), process_lifetime::RESTART_REQUIRED);
    assert_eq!(f.observe().guard_retained, 1);
}
#[test]
fn owner_adoption_close_error_is_terminal_without_retry_or_state_reclamation() {
    let mut f = Fixture::new(8);
    f.start();
    assert_eq!(f.drop_locals(), 0);
    f.stop();
    let (s, r, _) = f.data();
    let state = f.state();
    assert_eq!(
        (
            state.phase,
            state.host_fenced,
            state.lease_retained,
            state.close_state,
            state.last_callback_status
        ),
        (INDETERMINATE, 1, 1, CLOSE_UNKNOWN, NATIVE_STATUS_FAILED)
    );
    assert_eq!(
        unsafe { s.revoke.unwrap()(s.context, r.owner_handle) },
        process_lifetime::RESTART_REQUIRED
    );
    let o = f.observe();
    assert_eq!(
        (
            o.close_calls,
            o.state_alive,
            o.strong_count,
            o.destructor_calls
        ),
        (1, 1, 1, 0)
    );
}
#[test]
fn owner_adoption_post_acquire_panic_retains_concrete_lease_and_complete_indeterminate_output() {
    let mut f = Fixture::new(0);
    f.owner
        .owner_adoption
        .panic_after_acquire
        .store(true, Ordering::Release);
    f.start();
    let state = f.state();
    assert_eq!(
        (
            state.size,
            state.version,
            state.phase,
            state.host_fenced,
            state.lease_retained,
            state.close_state,
            state.native_operations_available
        ),
        (56, 1, INDETERMINATE, 1, 1, CLOSE_NONE, 0)
    );
    assert_eq!(f.observe().last_status, process_lifetime::RESTART_REQUIRED);
    assert_eq!(f.drop_locals(), process_lifetime::RESTART_REQUIRED);
    assert_eq!(f.observe().guard_retained, 1);
}
#[test]
fn owner_adoption_post_close_panic_is_terminal_and_cannot_repeat_callback() {
    let mut f = Fixture::new(0);
    f.start();
    assert_eq!(f.drop_locals(), 0);
    f.owner
        .owner_adoption
        .panic_after_close
        .store(true, Ordering::Release);
    f.stop();
    let (s, r, _) = f.data();
    assert_eq!(f.state().phase, INDETERMINATE);
    assert_eq!(f.state().close_state, CLOSE_UNKNOWN);
    assert_eq!(
        unsafe { s.revoke.unwrap()(s.context, r.owner_handle) },
        process_lifetime::RESTART_REQUIRED
    );
    assert_eq!(f.observe().close_calls, 1);
    assert_eq!(f.observe().strong_count, 1);
}
#[test]
fn owner_adoption_saved_table_cannot_adopt_late_and_checks_thread_foreign_context_handle() {
    let mut a = Fixture::new(0);
    a.start();
    let (sa, ra, qa) = a.data();
    let mut output = State {
        phase: 99,
        ..Default::default()
    };
    let sentinel = output;
    assert_eq!(
        unsafe { sa.adopt.unwrap()(sa.context, 40, &qa, 56, &mut output) },
        NATIVE_STATUS_UNAVAILABLE
    );
    assert_eq!(output, sentinel);
    assert_eq!(a.observe().acquire_calls, 1);
    let mut b = Fixture::new(0);
    b.start();
    let (sb, _, _) = b.data();
    assert_eq!(
        unsafe { sa.read_status.unwrap()(sb.context, ra.owner_handle, 56, &mut output) },
        NATIVE_STATUS_CONFLICT
    );
    assert_eq!(output, sentinel);
    assert_eq!(
        unsafe { sa.revoke.unwrap()(sb.context, ra.owner_handle) },
        NATIVE_STATUS_CONFLICT
    );
    assert_eq!(b.observe().close_calls, 0);
    let context_a = a.instance.as_ref().unwrap().host_context.as_ref();
    let context_identity = context_a as *const _ as usize;
    for (identity, generation, target) in [
        (
            context_identity + 1,
            ra.package_generation,
            context_a.process_target,
        ),
        (
            context_identity,
            ra.package_generation + 1,
            context_a.process_target,
        ),
        (
            context_identity,
            ra.package_generation,
            NATIVE_TARGET_SERVER,
        ),
    ] {
        assert_eq!(
            a.owner.owner_adoption_module(identity, generation, target),
            Err(NATIVE_STATUS_UNAVAILABLE)
        );
    }
    let address = sa.context as usize;
    let callback = sa.read_status.unwrap();
    let handle = ra.owner_handle;
    let adopt_callback = sa.adopt.unwrap();
    let revoke_callback = sa.revoke.unwrap();
    let request_address = &qa as *const Request as usize;
    let (status, result) = std::thread::spawn(move || {
        let mut out = State {
            phase: 99,
            ..Default::default()
        };
        let code = unsafe { callback(address as *mut c_void, handle, 56, &mut out) };
        assert_eq!(
            unsafe {
                adopt_callback(
                    address as *mut c_void,
                    40,
                    request_address as *const Request,
                    56,
                    &mut out,
                )
            },
            NATIVE_STATUS_WRONG_THREAD
        );
        assert_eq!(
            unsafe { revoke_callback(address as *mut c_void, handle) },
            NATIVE_STATUS_WRONG_THREAD
        );
        (code, out.phase)
    })
    .join()
    .unwrap();
    assert_eq!((status, result), (NATIVE_STATUS_WRONG_THREAD, 99));
    let host = a.instance.as_ref().unwrap().host.as_ref();
    let name = SERVICE.as_bytes();
    let mut table = [0x5555555555555555u64; 8];
    assert_eq!(
        unsafe {
            host.query_service.unwrap()(
                host.package_context,
                name.as_ptr(),
                name.len() as u32,
                1,
                64,
                table.as_mut_ptr().cast(),
            )
        },
        NATIVE_STATUS_UNAVAILABLE
    );
    assert_eq!(table, [0x5555555555555555u64; 8]);
}
#[test]
fn owner_adoption_unexpected_instance_drop_closes_once_and_preserves_saved_status_and_cpp_state() {
    let mut f = Fixture::new(0);
    f.start();
    assert_eq!(f.drop_locals(), 0);
    f.instance.take();
    let state = f.state();
    assert_eq!(
        (state.phase, state.host_fenced, state.lease_retained),
        (REVOKED, 1, 1)
    );
    let o = f.observe();
    assert_eq!(
        (
            o.state_alive,
            o.strong_count,
            o.guard_retained,
            o.close_calls,
            o.destructor_calls
        ),
        (1, 1, 0, 1, 0)
    );
}

/// Exact DLL bytes and only the common fixture protocol are used here. This same
/// test can be replayed with a separately frozen actual consumer fixture DLL.
#[test]
fn owner_adoption_component_roundtrip_from_parameterized_dll() {
    let mut f = Fixture::construct(0, None, false);
    f.start();
    let o = f.observe();
    assert_eq!((o.last_status, o.last_phase), (NATIVE_STATUS_OK, ADOPTED));
    assert_eq!(
        o.generation,
        f.instance.as_ref().unwrap().host.package_generation
    );
    assert_ne!(o.handle, 0);
    assert_ne!(o.identity, 0);
    assert_eq!(o.acquire_calls, 1);
    let context = (&**f.instance.as_ref().unwrap().host_context) as *const _ as *mut c_void;
    let status = || {
        let mut out = State::default();
        assert_eq!(unsafe { read_status(context, o.handle, 56, &mut out) }, 0);
        assert_eq!(
            (
                out.package_generation,
                out.owner_handle,
                out.state_identity as u64
            ),
            (o.generation, o.handle, o.identity)
        );
        out
    };
    let state = status();
    assert_eq!(
        (
            state.phase,
            state.host_fenced,
            state.lease_retained,
            state.close_state,
            state.native_operations_available,
            state.last_callback_status
        ),
        (ADOPTED, 0, 1, CLOSE_NONE, 0, 0)
    );
    assert_eq!(f.drop_locals(), 0);
    let alive = f.observe();
    assert_eq!(
        (
            alive.state_alive,
            alive.strong_count,
            alive.guard_retained,
            alive.destructor_calls
        ),
        (1, 1, 0, 0)
    );
    f.stop();
    let stopped = status();
    assert_eq!(
        (
            stopped.phase,
            stopped.host_fenced,
            stopped.lease_retained,
            stopped.close_state
        ),
        (REVOKED, 1, 1, CLOSE_DONE)
    );
    let alive = f.observe();
    assert_eq!(
        (
            alive.state_alive,
            alive.strong_count,
            alive.guard_retained,
            alive.close_calls,
            alive.destructor_calls
        ),
        (1, 1, 0, 1, 0)
    );
}

#[test]
fn owner_adoption_duplicate_after_success_within_initialize_is_refused_once() {
    let mut f = Fixture::new(18);
    f.start();
    let read: unsafe extern "C" fn(*mut i32, *mut i32, *mut i32) =
        unsafe { symbol(&f.library, b"SFOwnerHostFixtureReentryV1\0") };
    let (mut r, mut v, mut d) = (0, 0, 0);
    unsafe { read(&mut r, &mut v, &mut d) };
    assert_eq!(r, NATIVE_STATUS_CONFLICT);
    assert_eq!(f.observe().acquire_calls, 1);
    assert_eq!(f.state().phase, ADOPTED);
    assert_eq!(f.drop_locals(), 0);
    assert_eq!(f.observe().strong_count, 1);
}
#[test]
fn owner_adoption_close_reentry_status_revoke_and_drain_are_busy_without_retry() {
    let mut f = Fixture::new(19);
    f.start();
    assert_eq!(f.drop_locals(), 0);
    f.stop();
    let read: unsafe extern "C" fn(*mut i32, *mut i32, *mut i32) =
        unsafe { symbol(&f.library, b"SFOwnerHostFixtureReentryV1\0") };
    let (mut r, mut v, mut d) = (0, 0, 0);
    unsafe { read(&mut r, &mut v, &mut d) };
    assert_eq!(
        (r, v, d),
        (NATIVE_STATUS_BUSY, NATIVE_STATUS_BUSY, NATIVE_STATUS_BUSY)
    );
    assert_eq!(f.state().phase, REVOKED);
    assert_eq!(f.observe().close_calls, 1);
    assert_eq!(f.observe().strong_count, 1);
}

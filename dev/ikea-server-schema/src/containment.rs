use anyhow::{Result, ensure};
use std::ffi::c_void;
#[repr(C)]
#[derive(Default)]
struct Basic {
    process_time: i64,
    job_time: i64,
    flags: u32,
    min_working: usize,
    max_working: usize,
    active_process: u32,
    affinity: usize,
    priority: u32,
    scheduling: u32,
}
#[repr(C)]
#[derive(Default)]
struct IoCounters {
    a: u64,
    b: u64,
    c: u64,
    d: u64,
    e: u64,
    f: u64,
}
#[repr(C)]
#[derive(Default)]
struct Extended {
    basic: Basic,
    io: IoCounters,
    process_memory: usize,
    job_memory: usize,
    peak_process: usize,
    peak_job: usize,
}
#[link(name = "kernel32")]
unsafe extern "system" {
    fn CreateJobObjectW(attr: *const c_void, name: *const u16) -> *mut c_void;
    fn SetInformationJobObject(job: *mut c_void, class: i32, info: *const c_void, len: u32) -> i32;
    fn AssignProcessToJobObject(job: *mut c_void, process: *mut c_void) -> i32;
    fn GetCurrentProcess() -> *mut c_void;
    fn CloseHandle(handle: *mut c_void) -> i32;
}
pub fn install() -> Result<()> {
    let mut info = Extended::default();
    info.basic.flags = 0x100 | 0x2 | 0x8;
    info.basic.process_time = 600000000;
    info.basic.active_process = 1;
    info.process_memory = 1073741824;
    unsafe {
        let h = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        ensure!(!h.is_null(), "containment create");
        if SetInformationJobObject(
            h,
            9,
            &info as *const _ as *const c_void,
            std::mem::size_of::<Extended>() as u32,
        ) == 0
            || AssignProcessToJobObject(h, GetCurrentProcess()) == 0
        {
            CloseHandle(h);
            anyhow::bail!("containment unsupported");
        }
    }
    // The job handle deliberately lives until process exit; never release a self-limit early.
    std::thread::spawn(|| {
        std::thread::sleep(std::time::Duration::from_secs(180));
        eprintln!("{{\"status\":\"refused\",\"kind\":\"wall_budget\",\"rawErrorIncluded\":false}}");
        std::process::exit(2);
    });
    Ok(())
}

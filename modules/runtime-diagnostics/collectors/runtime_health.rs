use serde_json::{Value, json};

#[cfg(test)]
thread_local! {
    static SNAPSHOT_CALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
pub(crate) fn reset_snapshot_calls() {
    SNAPSHOT_CALLS.with(|calls| calls.set(0));
}

#[cfg(test)]
pub(crate) fn snapshot_calls() -> usize {
    SNAPSHOT_CALLS.with(std::cell::Cell::get)
}

#[cfg(test)]
pub(crate) fn snapshot() -> Value {
    SNAPSHOT_CALLS.with(|calls| calls.set(calls.get() + 1));
    json!({"testRuntimeHealth":true})
}

#[cfg(not(test))]
pub(crate) fn snapshot() -> Value {
    native_snapshot()
}

pub(crate) fn without_queue_details(mut snapshot: Value) -> Value {
    if let Some(object) = snapshot.as_object_mut() {
        object.remove("dispatcher");
    }
    snapshot
}

#[cfg(all(windows, not(test)))]
fn native_snapshot() -> Value {
    use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
    let name: Vec<u16> = "kfc-runtime.dll\0".encode_utf16().collect();
    unsafe {
        let module = GetModuleHandleW(name.as_ptr());
        if module.is_null() {
            return json!({"error":"runtime-provider-not-loaded"});
        }
        let Some(symbol) = GetProcAddress(module, c"KfcRuntimeDiagnostics".as_ptr().cast()) else {
            return json!({"error":"runtime-provider-missing-diagnostics-export"});
        };
        let function: unsafe extern "C" fn(*mut u8, usize) = std::mem::transmute(symbol);
        let mut buffer = vec![0u8; 8 << 20];
        function(buffer.as_mut_ptr(), buffer.len());
        let length = buffer
            .iter()
            .position(|byte| *byte == 0)
            .unwrap_or(buffer.len());
        serde_json::from_slice(&buffer[..length])
            .unwrap_or_else(|error| json!({"error":format!("invalid-provider-report: {error}")}))
    }
}

#[cfg(all(not(windows), not(test)))]
fn native_snapshot() -> Value {
    json!({"error":"native-provider-requires-windows"})
}

//! Bounded event-time notice submission. No Lua or Rust callback enters rendering.
use std::{ffi::c_void, mem::size_of};
#[repr(C)]
#[derive(Clone, Copy)]
struct NoticeService {
    size: u32,
    version: u32,
    context: *mut c_void,
    show: Option<unsafe extern "C" fn(*mut c_void, *const u8, u32) -> i32>,
}
pub fn show_notice(package: &str, generation: u64, message: &str) -> Result<(), i32> {
    if message.is_empty() || message.len() > 256 || message.bytes().any(|b| b < 32 || b == 127) {
        return Err(1);
    }
    let mut table = NoticeService {
        size: 0,
        version: 0,
        context: std::ptr::null_mut(),
        show: None,
    };
    let status = unsafe {
        crate::query_map_overlay_service(
            package,
            generation,
            "shroudforge.notices",
            1,
            size_of::<NoticeService>() as u32,
            (&mut table as *mut NoticeService).cast(),
        )
    };
    if status != 0 {
        return Err(status);
    }
    if table.size < size_of::<NoticeService>() as u32
        || table.version != 1
        || table.context.is_null()
    {
        return Err(2);
    }
    let call = table.show.ok_or(2)?;
    let status = unsafe { call(table.context, message.as_ptr(), message.len() as u32) };
    if status == 0 { Ok(()) } else { Err(status) }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_invalid_messages_before_querying_provider() {
        assert_eq!(size_of::<NoticeService>(), 24);
        for text in [
            String::new(),
            "a".repeat(257),
            "hello\nworld".into(),
            "\0".into(),
        ] {
            assert_eq!(show_notice("example", 1, &text), Err(1));
        }
    }
}

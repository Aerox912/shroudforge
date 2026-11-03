#[path="../../kfc-parser/crates/dinput8-proxy/src/provider.rs"] mod provider;
use std::ffi::c_void;
#[link(name="kernel32")] unsafe extern "system" {fn LoadLibraryW(name:*const u16)->*mut c_void;fn GetModuleHandleW(name:*const u16)->*mut c_void;fn GetProcAddress(module:*mut c_void,name:*const u8)->*mut c_void;}
unsafe extern "system" fn system(_: *mut c_void,_:u32,_:*const c_void,_:*mut *mut c_void,_:*mut c_void)->i32{17}
#[test] fn validated_registration_and_exact_real_call_forwarding(){unsafe{
    let slot=provider::Provider::new();let mut out=std::ptr::null_mut();
    assert_eq!(slot.forward(system as *const () as usize,std::ptr::null_mut(),0,std::ptr::null(),&mut out,std::ptr::null_mut()),17);
    let wide:Vec<u16>=std::env::var("SF_XHL_PROVIDER_FIXTURE").unwrap().encode_utf16().chain(Some(0)).collect();
    let module=LoadLibraryW(wide.as_ptr());assert!(!module.is_null());let proxy=GetModuleHandleW(std::ptr::null());
    assert_eq!(slot.register(std::ptr::null_mut(),proxy),0);assert_eq!(slot.register(proxy,proxy),0);
    assert_eq!(slot.register(module,proxy),1);assert_eq!(slot.register(module,proxy),2);assert_eq!(slot.register(proxy,module),0);
    for n in 1..=3 {let iid=0x1234usize as *const c_void;let outer=0x5678usize as *mut c_void;
        assert_eq!(slot.forward(system as *const () as usize,0x9876usize as *mut c_void,42,iid,&mut out,outer),0x12345678);
        assert_eq!(out as usize,n);
    }
    let count:unsafe extern "system" fn()->u32=std::mem::transmute(GetProcAddress(module,c"FixtureCount".as_ptr().cast()));assert_eq!(count(),3);
}}

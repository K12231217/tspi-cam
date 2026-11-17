//! HIL diagnostic: prove the board can `dlopen` MPP and that the `MppApi` table
//! matches `rk_mpi.h` (size field), without touching encoder configuration.
//!
//! Build via the Ubuntu 20.04 container:
//!   deploy/container/build.sh --example mpp_dlopen
//! Run on the board: `./mpp_dlopen`

#[cfg(target_os = "linux")]
fn main() {
    use std::ffi::{c_char, c_void, CString};

    const RTLD_NOW: i32 = 2;
    type Ret = i32;

    unsafe fn sym(handle: *mut c_void, name: &str) -> Option<*mut c_void> {
        let c = CString::new(name).ok()?;
        let p = libc::dlsym(handle, c.as_ptr());
        (!p.is_null()).then_some(p)
    }

    let name = CString::new("librockchip_mpp.so.1").unwrap();
    // SAFETY: plain dlopen; handle lives for the whole process.
    let handle = unsafe { libc::dlopen(name.as_ptr(), RTLD_NOW) };
    if handle.is_null() {
        let err = unsafe {
            let e = libc::dlerror();
            if e.is_null() {
                "unknown".to_string()
            } else {
                std::ffi::CStr::from_ptr(e).to_string_lossy().into_owned()
            }
        };
        eprintln!("dlopen failed: {err}");
        std::process::exit(1);
    }
    println!("dlopen: ok");

    // SAFETY: signatures from rk_mpi.h.
    let create: extern "C" fn(*mut *mut c_void, *mut *mut c_void) -> Ret =
        unsafe { std::mem::transmute(sym(handle, "mpp_create").expect("mpp_create")) };
    let init: extern "C" fn(*mut c_void, u32, u32) -> Ret =
        unsafe { std::mem::transmute(sym(handle, "mpp_init").expect("mpp_init")) };
    let destroy: extern "C" fn(*mut c_void) -> Ret =
        unsafe { std::mem::transmute(sym(handle, "mpp_destroy").expect("mpp_destroy")) };

    let mut ctx: *mut c_void = std::ptr::null_mut();
    let mut api: *mut c_void = std::ptr::null_mut();
    let r = create(&mut ctx, &mut api);
    println!("mpp_create: ret={r} ctx={ctx:p} api={api:p}");
    // SAFETY: api is the table returned by mpp_create; the first field is `size`.
    let api_size = unsafe { *(api as *const u32) };
    // SAFETY: same table, second field is `version`.
    let api_version = unsafe { *((api as *const u32).add(1)) };
    println!("api: size={api_size} version={api_version}");
    let r = init(ctx, 1, 7); // MPP_CTX_ENC, MPP_VIDEO_CodingAVC
    println!("mpp_init(enc, avc): ret={r}");
    let r = destroy(ctx);
    println!("mpp_destroy: ret={r}");
    // SAFETY: handle from dlopen.
    unsafe { libc::dlclose(handle) };
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("mpp_dlopen runs on the board only (linux)");
}

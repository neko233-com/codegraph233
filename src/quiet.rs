//! Optional global Windows quiet scheduling. This changes only this process;
//! power/frequency settings and other applications are never touched.
pub fn initialize() -> Result<(), String> {
    if std::env::var("CODEGRAPH233_QUIET").as_deref() != Ok("1") {
        return Ok(());
    }
    #[cfg(windows)]
    unsafe {
        use std::ffi::c_void;
        #[link(name = "kernel32")]
        extern "system" {
            fn GetCurrentProcess() -> *mut c_void;
            fn GetProcessAffinityMask(
                process: *mut c_void,
                process_mask: *mut usize,
                system_mask: *mut usize,
            ) -> i32;
            fn SetProcessAffinityMask(process: *mut c_void, mask: usize) -> i32;
            fn SetPriorityClass(process: *mut c_void, class: u32) -> i32;
        }
        let current = GetCurrentProcess();
        let mut allowed = 0usize;
        let mut system = 0usize;
        if GetProcessAffinityMask(current, &mut allowed, &mut system) == 0
            || allowed == 0
            || SetProcessAffinityMask(current, allowed & allowed.wrapping_neg()) == 0
            || SetPriorityClass(current, 0x4000) == 0
        {
            return Err(format!(
                "quiet scheduling: {}",
                std::io::Error::last_os_error()
            ));
        }
    }
    if std::env::var_os("CODEGRAPH233_JOBS").is_none() {
        std::env::set_var("CODEGRAPH233_JOBS", "1");
    }
    Ok(())
}

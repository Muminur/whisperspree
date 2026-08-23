//! macOS-only ASR platform probes.

use std::os::raw::{c_char, c_int, c_void};

/// Read the physical-core count through the native sysctl API. This stays in
/// the macOS boundary rather than spawning a process from the recognizer.
pub fn physical_core_count() -> Option<usize> {
    extern "C" {
        fn sysctlbyname(
            name: *const c_char,
            oldp: *mut c_void,
            oldlenp: *mut usize,
            newp: *mut c_void,
            newlen: usize,
        ) -> c_int;
    }

    let name = b"hw.physicalcpu\0";
    let mut value: u32 = 0;
    let mut size = std::mem::size_of::<u32>();
    // SAFETY: `name` is NUL-terminated and both output pointers refer to
    // initialized writable storage of the declared size.
    let result = unsafe {
        sysctlbyname(
            name.as_ptr().cast(),
            (&mut value as *mut u32).cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    (result == 0 && size == std::mem::size_of::<u32>() && value > 0).then_some(value as usize)
}

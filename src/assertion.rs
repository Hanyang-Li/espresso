//! Thin FFI wrapper around IOKit power assertions.
//!
//! Exposes [`SleepAssertion`], an RAII guard that holds a
//! `PreventUserIdleSystemSleep` assertion for as long as it is alive and
//! releases it on [`Drop`].

use core_foundation::base::TCFType;
use core_foundation::string::CFString;
use core_foundation_sys::string::CFStringRef;
use std::io;

#[allow(non_upper_case_globals)]
const kIOPMAssertionLevelOn: u32 = 255;

type IOPMAssertionID = u32;
type IOReturn = i32;

#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    fn IOPMAssertionCreateWithName(
        assertion_type: CFStringRef,
        assertion_level: u32,
        assertion_name: CFStringRef,
        assertion_id: *mut IOPMAssertionID,
    ) -> IOReturn;
    fn IOPMAssertionRelease(assertion_id: IOPMAssertionID) -> IOReturn;
}

/// Holds a `PreventUserIdleSystemSleep` assertion for the guard's lifetime.
///
/// The kernel also releases the assertion automatically if the process dies,
/// but the guard releases it deterministically on [`Drop`] during normal
/// operation.
pub struct SleepAssertion {
    id: IOPMAssertionID,
}

impl SleepAssertion {
    /// Creates and holds a `PreventUserIdleSystemSleep` assertion, labeled
    /// with `reason` (as shown by `pmset -g assertions`).
    pub fn prevent_idle_sleep(reason: &str) -> io::Result<Self> {
        let assertion_type = CFString::new("PreventUserIdleSystemSleep");
        let name = CFString::new(reason);
        let mut id: IOPMAssertionID = 0;
        let rc = unsafe {
            IOPMAssertionCreateWithName(
                assertion_type.as_concrete_TypeRef(),
                kIOPMAssertionLevelOn,
                name.as_concrete_TypeRef(),
                &mut id,
            )
        };
        if rc != 0 {
            return Err(io::Error::other(format!(
                "IOPMAssertionCreateWithName failed: {rc:#x}"
            )));
        }
        Ok(Self { id })
    }
}

impl Drop for SleepAssertion {
    fn drop(&mut self) {
        unsafe {
            IOPMAssertionRelease(self.id);
        }
    }
}

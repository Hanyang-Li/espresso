//! Thin FFI wrapper around IOKit's system power settings, scoped to the
//! global `SleepDisabled` flag.
//!
//! `IOPMSetSystemPowerSetting` requires root; `IOPMCopySystemPowerSettings`
//! does not.

use core_foundation::base::{CFType, TCFType};
use core_foundation::boolean::CFBoolean;
use core_foundation::dictionary::CFDictionary;
use core_foundation::string::CFString;
use core_foundation_sys::base::CFTypeRef;
use core_foundation_sys::dictionary::CFDictionaryRef;
use core_foundation_sys::string::CFStringRef;
use std::ffi::c_void;
use std::io;

type IOReturn = i32;

const SLEEP_DISABLED_KEY: &str = "SleepDisabled";

#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    fn IOPMSetSystemPowerSetting(key: CFStringRef, value: CFTypeRef) -> IOReturn;
    fn IOPMCopySystemPowerSettings() -> CFDictionaryRef;
}

/// Sets the global `SleepDisabled` power setting. Requires root; returns an
/// `Err` (never panics) if the underlying IOKit call fails, e.g. because the
/// caller lacks the required privileges.
pub fn set_sleep_disabled(disabled: bool) -> io::Result<()> {
    let key = CFString::new(SLEEP_DISABLED_KEY);
    let value = CFBoolean::from(disabled);
    let rc = unsafe { IOPMSetSystemPowerSetting(key.as_concrete_TypeRef(), value.as_CFTypeRef()) };
    if rc != 0 {
        return Err(io::Error::other(format!(
            "IOPMSetSystemPowerSetting(SleepDisabled) failed: {rc:#x} (requires root)"
        )));
    }
    Ok(())
}

/// Reads the current `SleepDisabled` power setting. Does not require root.
/// Returns `false` if the setting is absent or the system settings
/// dictionary could not be retrieved.
pub fn read_sleep_disabled() -> io::Result<bool> {
    let dict_ref = unsafe { IOPMCopySystemPowerSettings() };
    if dict_ref.is_null() {
        return Ok(false);
    }
    // IOPMCopySystemPowerSettings follows the create rule: we own this
    // reference and must release it, which CFDictionary's Drop impl does.
    let dict: CFDictionary<*const c_void, CFType> =
        unsafe { CFDictionary::wrap_under_create_rule(dict_ref) };
    let key = CFString::new(SLEEP_DISABLED_KEY);

    let disabled = dict
        .find(key.as_CFTypeRef())
        .and_then(|value| value.downcast::<CFBoolean>())
        .map(bool::from)
        .unwrap_or(false);
    Ok(disabled)
}

//! Thin FFI wrapper around IOKit's IORegistry, scoped to reading the
//! `AppleClamshellState` property off the `IOPMrootDomain` service.
//!
//! This reflects whether the built-in display (lid) is currently closed.
//! Desktop Macs have no clamshell and the property is simply absent, which
//! reads as "open" (`false`).

use core_foundation::base::TCFType;
use core_foundation::string::CFString;
use core_foundation_sys::base::{CFRelease, CFTypeRef, kCFAllocatorDefault};
use core_foundation_sys::dictionary::__CFDictionary;
use core_foundation_sys::number::{CFBooleanRef, kCFBooleanTrue};
use core_foundation_sys::string::CFStringRef;
use std::io;
use std::os::raw::c_char;

type IOOptionBits = u32;
type IOReturn = i32;
type MachPort = u32;
type IoObject = u32;

const APPLE_CLAMSHELL_STATE_KEY: &str = "AppleClamshellState";

#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    fn IOServiceMatching(name: *const c_char) -> *mut __CFDictionary;
    fn IOServiceGetMatchingService(main_port: MachPort, matching: *const __CFDictionary) -> IoObject;
    fn IORegistryEntryCreateCFProperty(
        entry: IoObject,
        key: CFStringRef,
        allocator: core_foundation_sys::base::CFAllocatorRef,
        options: IOOptionBits,
    ) -> CFTypeRef;
    fn IOObjectRelease(object: IoObject) -> IOReturn;
}

/// Reads whether the lid (built-in clamshell display) is currently closed.
///
/// Returns `Ok(false)` (open) if the `AppleClamshellState` property is
/// absent, e.g. on a desktop Mac with no lid.
pub fn lid_closed() -> io::Result<bool> {
    unsafe {
        let matching = IOServiceMatching(c"IOPMrootDomain".as_ptr());
        if matching.is_null() {
            return Err(io::Error::other("IOServiceMatching(IOPMrootDomain) returned null"));
        }
        // IOServiceGetMatchingService consumes the matching dictionary
        // reference; it must not also be released here.
        let service = IOServiceGetMatchingService(0, matching);
        if service == 0 {
            return Err(io::Error::other("IOPMrootDomain service not found"));
        }
        let key = CFString::new(APPLE_CLAMSHELL_STATE_KEY);
        let prop = IORegistryEntryCreateCFProperty(
            service,
            key.as_concrete_TypeRef(),
            kCFAllocatorDefault,
            0,
        );
        IOObjectRelease(service);
        if prop.is_null() {
            // No clamshell (e.g. desktop Mac): treat as open.
            return Ok(false);
        }
        // IORegistryEntryCreateCFProperty follows the create rule: we own
        // this reference and must release it.
        let is_true = prop as CFBooleanRef == kCFBooleanTrue;
        CFRelease(prop);
        Ok(is_true)
    }
}

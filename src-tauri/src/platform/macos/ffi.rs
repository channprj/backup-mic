//! Minimal Disk Arbitration FFI boundary.
//!
//! `DASessionCreate` and `DADiskCopyDescription` follow Core Foundation's
//! create/copy ownership rule and are released here. Disk and dictionary values
//! received by callbacks are borrowed; they are converted to owned Rust values
//! before an event crosses the channel.

use std::{
    ffi::{CStr, c_char, c_void},
    ptr,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{Sender, SyncSender},
    },
};

use core_foundation::{
    base::TCFType, boolean::CFBoolean, dictionary::CFDictionary, number::CFNumber,
    string::CFString, url::CFURL, uuid::CFUUID,
};
use core_foundation_sys::{
    array::CFArrayRef,
    base::{CFRelease, CFTypeRef, kCFAllocatorDefault},
    dictionary::CFDictionaryRef,
    runloop::{CFRunLoopGetCurrent, CFRunLoopRef, CFRunLoopRunInMode, kCFRunLoopDefaultMode},
    string::CFStringRef,
    uuid::CFUUIDCreateString,
};

use crate::platform::device_registry::{NativeDiskDescription, NativeDiskEvent};

type DASessionRef = *const c_void;
type DADiskRef = *const c_void;

#[link(name = "DiskArbitration", kind = "framework")]
unsafe extern "C" {
    fn DASessionCreate(allocator: *const c_void) -> DASessionRef;
    fn DASessionScheduleWithRunLoop(
        session: DASessionRef,
        run_loop: CFRunLoopRef,
        run_loop_mode: CFStringRef,
    );
    fn DASessionUnscheduleFromRunLoop(
        session: DASessionRef,
        run_loop: CFRunLoopRef,
        run_loop_mode: CFStringRef,
    );
    fn DARegisterDiskAppearedCallback(
        session: DASessionRef,
        matches: CFDictionaryRef,
        callback: extern "C" fn(DADiskRef, *mut c_void),
        context: *mut c_void,
    );
    fn DARegisterDiskDescriptionChangedCallback(
        session: DASessionRef,
        matches: CFDictionaryRef,
        watch: CFArrayRef,
        callback: extern "C" fn(DADiskRef, CFArrayRef, *mut c_void),
        context: *mut c_void,
    );
    fn DARegisterDiskDisappearedCallback(
        session: DASessionRef,
        matches: CFDictionaryRef,
        callback: extern "C" fn(DADiskRef, *mut c_void),
        context: *mut c_void,
    );
    fn DADiskCopyDescription(disk: DADiskRef) -> CFDictionaryRef;
    fn DADiskGetBSDName(disk: DADiskRef) -> *const c_char;

    static kDADiskDescriptionVolumeUUIDKey: CFStringRef;
    static kDADiskDescriptionVolumePathKey: CFStringRef;
    static kDADiskDescriptionVolumeNameKey: CFStringRef;
    static kDADiskDescriptionDeviceProtocolKey: CFStringRef;
    static kDADiskDescriptionDeviceInternalKey: CFStringRef;
    static kDADiskDescriptionMediaRemovableKey: CFStringRef;
    static kDADiskDescriptionMediaWritableKey: CFStringRef;
    static kDADiskDescriptionMediaNameKey: CFStringRef;
    static kDADiskDescriptionMediaSizeKey: CFStringRef;
}

struct CallbackContext {
    sender: Sender<NativeDiskEvent>,
}

pub(super) fn run(
    sender: Sender<NativeDiskEvent>,
    stop: Arc<AtomicBool>,
    ready: &SyncSender<Result<(), String>>,
) -> Result<(), String> {
    // SAFETY: Every pointer passed to Disk Arbitration is valid for the whole
    // scheduled run-loop lifetime. Cleanup happens only after unscheduling.
    unsafe {
        let session = DASessionCreate(ptr::null());
        if session.is_null() {
            return Err("DASessionCreate returned null".to_owned());
        }
        let run_loop = CFRunLoopGetCurrent();
        let context = Box::into_raw(Box::new(CallbackContext { sender }));
        DARegisterDiskAppearedCallback(session, ptr::null(), disk_appeared, context.cast());
        DARegisterDiskDescriptionChangedCallback(
            session,
            ptr::null(),
            ptr::null(),
            disk_description_changed,
            context.cast(),
        );
        DARegisterDiskDisappearedCallback(session, ptr::null(), disk_disappeared, context.cast());
        DASessionScheduleWithRunLoop(session, run_loop, kCFRunLoopDefaultMode);
        let _ = ready.send(Ok(()));
        while !stop.load(Ordering::SeqCst) {
            CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.25, 0);
        }
        DASessionUnscheduleFromRunLoop(session, run_loop, kCFRunLoopDefaultMode);
        CFRelease(session.cast::<c_void>() as CFTypeRef);
        drop(Box::from_raw(context));
    }
    Ok(())
}

extern "C" fn disk_appeared(disk: DADiskRef, context: *mut c_void) {
    send_description(disk, context, false);
}

extern "C" fn disk_description_changed(disk: DADiskRef, _keys: CFArrayRef, context: *mut c_void) {
    send_description(disk, context, true);
}

extern "C" fn disk_disappeared(disk: DADiskRef, context: *mut c_void) {
    let _ = std::panic::catch_unwind(|| {
        // SAFETY: Disk Arbitration supplies borrowed callback pointers valid for
        // the callback duration; the context is our live Box.
        unsafe {
            let Some(disk_id) = disk_id(disk) else {
                return;
            };
            let callback = &*context.cast::<CallbackContext>();
            let _ = callback
                .sender
                .send(NativeDiskEvent::Disappeared { disk_id });
        }
    });
}

fn send_description(disk: DADiskRef, context: *mut c_void, changed: bool) {
    let _ = std::panic::catch_unwind(|| {
        // SAFETY: Disk Arbitration supplies borrowed callback pointers valid for
        // the callback duration; description conversion copies every value.
        unsafe {
            let Some(description) = copy_description(disk) else {
                return;
            };
            let callback = &*context.cast::<CallbackContext>();
            let event = if changed {
                NativeDiskEvent::DescriptionChanged(description)
            } else {
                NativeDiskEvent::Appeared(description)
            };
            let _ = callback.sender.send(event);
        }
    });
}

unsafe fn copy_description(disk: DADiskRef) -> Option<NativeDiskDescription> {
    // SAFETY: `disk` is borrowed and valid during the callback.
    let raw = unsafe { DADiskCopyDescription(disk) };
    if raw.is_null() {
        return None;
    }
    // SAFETY: DADiskCopyDescription returns an owned +1 reference.
    let dictionary: CFDictionary<*const c_void, *const c_void> =
        unsafe { TCFType::wrap_under_create_rule(raw) };
    Some(NativeDiskDescription {
        // SAFETY: All keys and their Core Foundation value types are specified
        // by DADisk.h. Helpers retain/copy before returning Rust values.
        disk_id: unsafe { disk_id(disk) }?,
        volume_uuid: unsafe { uuid_value(&dictionary, kDADiskDescriptionVolumeUUIDKey) },
        mount_root: unsafe { url_value(&dictionary, kDADiskDescriptionVolumePathKey) },
        protocol: unsafe { string_value(&dictionary, kDADiskDescriptionDeviceProtocolKey) },
        is_internal: unsafe { boolean_value(&dictionary, kDADiskDescriptionDeviceInternalKey) },
        is_removable: unsafe { boolean_value(&dictionary, kDADiskDescriptionMediaRemovableKey) },
        is_writable: unsafe { boolean_value(&dictionary, kDADiskDescriptionMediaWritableKey) },
        media_name: unsafe { string_value(&dictionary, kDADiskDescriptionMediaNameKey) },
        nominal_capacity: unsafe { number_value(&dictionary, kDADiskDescriptionMediaSizeKey) },
        display_name: unsafe { string_value(&dictionary, kDADiskDescriptionVolumeNameKey) },
    })
}

unsafe fn disk_id(disk: DADiskRef) -> Option<String> {
    // SAFETY: The returned C string is borrowed from the live disk object.
    let name = unsafe { DADiskGetBSDName(disk) };
    (!name.is_null()).then(|| {
        // SAFETY: Disk Arbitration documents a null-terminated BSD name.
        unsafe { CStr::from_ptr(name) }
            .to_string_lossy()
            .into_owned()
    })
}

unsafe fn value(
    dictionary: &CFDictionary<*const c_void, *const c_void>,
    key: CFStringRef,
) -> Option<*const c_void> {
    dictionary.find(key.cast::<c_void>()).map(|value| *value)
}

unsafe fn string_value(
    dictionary: &CFDictionary<*const c_void, *const c_void>,
    key: CFStringRef,
) -> Option<String> {
    // SAFETY: The key's value type is CFString according to DADisk.h.
    let value = unsafe { value(dictionary, key) }?;
    let string = unsafe { CFString::wrap_under_get_rule(value.cast()) };
    Some(string.to_string())
}

unsafe fn boolean_value(
    dictionary: &CFDictionary<*const c_void, *const c_void>,
    key: CFStringRef,
) -> Option<bool> {
    // SAFETY: The key's value type is CFBoolean according to DADisk.h.
    let value = unsafe { value(dictionary, key) }?;
    let boolean = unsafe { CFBoolean::wrap_under_get_rule(value.cast()) };
    Some(bool::from(boolean))
}

unsafe fn number_value(
    dictionary: &CFDictionary<*const c_void, *const c_void>,
    key: CFStringRef,
) -> Option<u64> {
    // SAFETY: The key's value type is CFNumber according to DADisk.h.
    let value = unsafe { value(dictionary, key) }?;
    let number = unsafe { CFNumber::wrap_under_get_rule(value.cast()) };
    number
        .to_i64()
        .and_then(|number| u64::try_from(number).ok())
}

unsafe fn url_value(
    dictionary: &CFDictionary<*const c_void, *const c_void>,
    key: CFStringRef,
) -> Option<std::path::PathBuf> {
    // SAFETY: The key's value type is CFURL according to DADisk.h.
    let value = unsafe { value(dictionary, key) }?;
    let url = unsafe { CFURL::wrap_under_get_rule(value.cast()) };
    url.to_path()
}

unsafe fn uuid_value(
    dictionary: &CFDictionary<*const c_void, *const c_void>,
    key: CFStringRef,
) -> Option<String> {
    // SAFETY: The key's value type is CFUUID according to DADisk.h.
    let value = unsafe { value(dictionary, key) }?;
    let uuid = unsafe { CFUUID::wrap_under_get_rule(value.cast()) };
    // SAFETY: CFUUIDCreateString returns an owned +1 CFString for this live UUID.
    let string = unsafe {
        CFString::wrap_under_create_rule(CFUUIDCreateString(
            kCFAllocatorDefault,
            uuid.as_concrete_TypeRef(),
        ))
    };
    Some(string.to_string())
}

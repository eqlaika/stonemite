//! Dynamic code patch to disable the foreground check in EQ's keyboard_process.
//!
//! keyboard_process at RVA 0x351E10 gates keyboard input on:
//!     call GetForegroundWindow
//!     cmp  rax, [stored_fg_hwnd]
//!     jne  skip                    ; <-- RVA 0x351E47, bytes 0F 85 xx xx xx xx
//!
//! We NOP this JNE when shared-memory input is active so background windows
//! process keyboard input.  When deactivated, the original bytes are restored.

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};
use windows::Win32::System::Memory::{VirtualProtect, PAGE_EXECUTE_READWRITE, PAGE_PROTECTION_FLAGS};

/// RVA of the JNE instruction inside keyboard_process.
const KBD_JNE_RVA: usize = 0x351E47;
/// Expected first two bytes (JNE near rel32).
const JNE_OPCODE: [u8; 2] = [0x0F, 0x85];

static mut PATCH_ADDR: *mut u8 = std::ptr::null_mut();
static mut ORIGINAL_BYTES: [u8; 6] = [0; 6];
static INITIALIZED: AtomicBool = AtomicBool::new(false);
static PATCHED: AtomicBool = AtomicBool::new(false);

/// Call once at startup to locate and validate the patch site.
pub unsafe fn init() {
    let base = match windows::Win32::System::LibraryLoader::GetModuleHandleW(None) {
        Ok(h) => h.0 as *mut u8,
        Err(_) => return,
    };

    let addr = base.add(KBD_JNE_RVA);

    // Verify the opcode before saving — protects against EQ version mismatch.
    if *addr != JNE_OPCODE[0] || *addr.add(1) != JNE_OPCODE[1] {
        crate::log::write(&format!(
            "kbd_patch: opcode mismatch at {:p} ({:#04x} {:#04x}), expected 0F 85",
            addr,
            *addr,
            *addr.add(1),
        ));
        return;
    }

    std::ptr::copy_nonoverlapping(addr, ORIGINAL_BYTES.as_mut_ptr(), 6);
    PATCH_ADDR = addr;
    INITIALIZED.store(true, Ordering::Release);
    crate::log::write(&format!("kbd_patch: ready at {:p}", addr));
}

/// NOP the JNE so keyboard_process always proceeds (call when shm active).
pub unsafe fn enable() {
    if !INITIALIZED.load(Ordering::Acquire) || PATCHED.load(Ordering::Relaxed) {
        return;
    }
    let addr = PATCH_ADDR;
    let mut old = PAGE_PROTECTION_FLAGS(0);
    if VirtualProtect(addr as *const c_void, 6, PAGE_EXECUTE_READWRITE, &mut old).is_err() {
        return;
    }
    std::ptr::write_bytes(addr, 0x90, 6); // 6× NOP
    let _ = VirtualProtect(addr as *const c_void, 6, old, &mut old);
    PATCHED.store(true, Ordering::Release);
    crate::log::write("kbd_patch: enabled (JNE NOPped)");
}

/// Restore the original JNE (call when shm deactivates).
pub unsafe fn disable() {
    if !INITIALIZED.load(Ordering::Acquire) || !PATCHED.load(Ordering::Relaxed) {
        return;
    }
    let addr = PATCH_ADDR;
    let mut old = PAGE_PROTECTION_FLAGS(0);
    if VirtualProtect(addr as *const c_void, 6, PAGE_EXECUTE_READWRITE, &mut old).is_err() {
        return;
    }
    std::ptr::copy_nonoverlapping(ORIGINAL_BYTES.as_ptr(), addr, 6);
    let _ = VirtualProtect(addr as *const c_void, 6, old, &mut old);
    PATCHED.store(false, Ordering::Release);
}

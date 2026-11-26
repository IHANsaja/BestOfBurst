//! Licensing for the shipped app: the embedded public key, this machine's fingerprint, and
//! reading/writing the stored key in `%APPDATA%\bestofburst`.
//!
//! The app can only ever *check* a key. The private key that *makes* keys lives in the
//! `crates/keygen` tool, which is never committed and never shipped. See `crates/license-core`
//! for the key format and the Ed25519 verification itself.
//!
//! What stops a swapped-in public key? Nothing client-side can fully stop a motivated person
//! who patches the binary — that is true of every offline license check ever written. The
//! point of signing is narrower and solid: without the private key, no one can forge a key
//! that verifies. The masking and the re-check-every-frame below just raise the effort for the
//! casual patcher; they are not claimed to be unbreakable.

use std::path::PathBuf;

pub use license_core::{Edition, License, LicenseError};

// The app's public verifying key, XOR-masked with a random mask so the raw 32 bytes are not a
// single findable run in the binary. Regenerate both lines with `bob-keygen pubkey`.
const PK_MASK: [u8; 32] = [
    0xa9, 0x84, 0x17, 0x31, 0x5a, 0xe9, 0xdc, 0xfd, 0x68, 0x27, 0x3c, 0x85, 0x8c, 0xb2, 0x70, 0x4b, 0xa3, 0x39, 0x7c,
    0x62, 0x8e, 0x8c, 0x6e, 0x4d, 0xba, 0x2c, 0x7e, 0xd4, 0x81, 0xcf, 0xa2, 0xd9,
];
const PK_MASKED: [u8; 32] = [
    0xed, 0xdd, 0xb8, 0xf2, 0x3a, 0xba, 0x0a, 0xc1, 0x2a, 0x38, 0x07, 0x26, 0xd0, 0xde, 0x20, 0x38, 0x70, 0x93, 0xe4,
    0xda, 0x45, 0xd4, 0x61, 0xa6, 0x9f, 0x73, 0x67, 0x9e, 0x51, 0x56, 0x56, 0xd1,
];

/// Reconstruct the public key at runtime. Kept `#[inline(never)]` so there is one site to find
/// and it is not folded into every caller.
#[inline(never)]
fn public_key() -> [u8; 32] {
    let mut k = PK_MASKED;
    for i in 0..32 {
        k[i] ^= PK_MASK[i];
    }
    k
}

const KEY_FILE: &str = "license.key";

fn app_dir() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join("bestofburst"))
}

/// Current day number (days since the Unix epoch), for expiry checks.
fn today_day() -> u32 {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    (secs / 86_400) as u32
}

/// A stable per-machine fingerprint. On Windows it is the registry `MachineGuid`, which
/// survives reboots and app reinstalls. Shown to the customer so you can issue a node-locked
/// key for their machine; also checked here when such a key is entered.
pub fn machine_fingerprint() -> String {
    #[cfg(windows)]
    {
        if let Some(guid) = read_machine_guid() {
            return guid;
        }
    }
    // Fallback (and non-Windows): a weaker fingerprint from the environment.
    let name = std::env::var("COMPUTERNAME").unwrap_or_default();
    let cpu = std::env::var("PROCESSOR_IDENTIFIER").unwrap_or_default();
    format!("env:{name}:{cpu}")
}

/// The short, human-readable form of the fingerprint shown on the activation screen (the tag
/// the keygen actually binds to, as uppercase hex). Customers read this to you for node-locking.
pub fn machine_code() -> String {
    let tag = license_core::machine_tag(&machine_fingerprint());
    tag.iter().map(|b| format!("{b:02X}")).collect()
}

#[cfg(windows)]
fn read_machine_guid() -> Option<String> {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::ERROR_SUCCESS;
    use windows::Win32::System::Registry::{
        RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RRF_SUBKEY_WOW6464KEY,
    };

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }
    let subkey = wide("SOFTWARE\\Microsoft\\Cryptography");
    let value = wide("MachineGuid");
    let mut buf = [0u16; 128];
    let mut len: u32 = (buf.len() * 2) as u32;
    // SAFETY: valid null-terminated UTF-16 pointers, a buffer sized by `len`, and `len` is
    // updated in place by the call. RegGetValueW does not retain the pointers.
    let status = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            PCWSTR(subkey.as_ptr()),
            PCWSTR(value.as_ptr()),
            RRF_RT_REG_SZ | RRF_SUBKEY_WOW6464KEY,
            None,
            Some(buf.as_mut_ptr() as *mut _),
            Some(&mut len),
        )
    };
    if status != ERROR_SUCCESS {
        return None;
    }
    // `len` is bytes including the trailing NUL.
    let chars = (len as usize / 2).saturating_sub(1).min(buf.len());
    let s = String::from_utf16_lossy(&buf[..chars]);
    let s = s.trim_matches('\0').trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

/// Verify a key the user typed. On success the normalized key is stored so the app opens
/// straight to work next time. Returns the license details for display.
pub fn activate(input: &str) -> Result<License, LicenseError> {
    let machine = license_core::machine_tag(&machine_fingerprint());
    let license = license_core::verify(&public_key(), input, today_day(), &machine)?;
    // Store the exact text the user gave (it is signed; it cannot be usefully altered).
    if let Some(dir) = app_dir() {
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::write(dir.join(KEY_FILE), input.trim().as_bytes());
    }
    Ok(license)
}

/// Re-verify the stored key, if any. Called at launch and periodically while running, so the
/// gate is the signature itself every time — not a flag that was true once.
pub fn active_license() -> Option<License> {
    let dir = app_dir()?;
    let stored = std::fs::read_to_string(dir.join(KEY_FILE)).ok()?;
    let machine = license_core::machine_tag(&machine_fingerprint());
    license_core::verify(&public_key(), &stored, today_day(), &machine).ok()
}

/// Remove the stored key (used if the app ever needs a "deactivate" affordance).
#[allow(dead_code)]
pub fn clear() {
    if let Some(dir) = app_dir() {
        let _ = std::fs::remove_file(dir.join(KEY_FILE));
    }
}

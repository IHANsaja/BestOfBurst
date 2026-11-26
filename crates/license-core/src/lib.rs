//! The license-key format and its cryptography, shared by the app and the (private) keygen.
//!
//! A license key is a signed, self-contained token. The keygen holds the Ed25519 **private**
//! key and is the only thing that can produce a valid key. The app embeds only the matching
//! **public** key and can do nothing but check a key against it. Because forging an Ed25519
//! signature without the private key is infeasible, no key works with the app unless it came
//! out of the one keygen — which is exactly the guarantee a commercial product needs.
//!
//! Wire format of a key, before text encoding:
//! ```text
//!   [ 16-byte payload ] [ 64-byte Ed25519 signature over the payload ]
//! ```
//! The 80 bytes are Crockford base32-encoded (no padding, no ambiguous letters) and grouped
//! into dashed blocks so a human can read one back over the phone if they must.

#![forbid(unsafe_code)]

use ed25519_dalek::{Signature, VerifyingKey};
use sha2::{Digest, Sha512};

/// Marks our payloads so a random 80 bytes almost never decodes to a "valid-looking" one.
const MAGIC: u8 = 0xB0;
/// Payload layout version. Bump only if the 16-byte layout below changes.
const VERSION: u8 = 0x01;

const PAYLOAD_LEN: usize = 16;
const SIG_LEN: usize = 64;
const BLOB_LEN: usize = PAYLOAD_LEN + SIG_LEN;

/// Which edition a key unlocks. Stored as one byte so the app can offer tiers later.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edition {
    Standard,
    Pro,
    Studio,
}

impl Edition {
    #[cfg(feature = "sign")]
    fn to_byte(self) -> u8 {
        match self {
            Edition::Standard => 0,
            Edition::Pro => 1,
            Edition::Studio => 2,
        }
    }
    fn from_byte(b: u8) -> Option<Edition> {
        match b {
            0 => Some(Edition::Standard),
            1 => Some(Edition::Pro),
            2 => Some(Edition::Studio),
            _ => None,
        }
    }
    /// The word shown on the activation screen once a key checks out.
    pub fn label(self) -> &'static str {
        match self {
            Edition::Standard => "Standard",
            Edition::Pro => "Pro",
            Edition::Studio => "Studio",
        }
    }
}

/// Everything a verified key tells the app.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct License {
    pub edition: Edition,
    /// Days since the Unix epoch after which the key stops working. `None` is perpetual.
    pub expires_day: Option<u32>,
    /// When set, the key only works on the machine whose fingerprint matches `machine`.
    pub node_locked: bool,
    /// First 8 bytes of SHA-512 over the bound machine fingerprint (zeros when not locked).
    pub machine: [u8; 8],
}

/// Why a key did not verify. Deliberately coarse: the app should never tell an attacker
/// *which* check failed, only that the key is not valid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LicenseError {
    /// Wrong length, bad characters, bad magic/version, or a signature that did not verify.
    Invalid,
    /// The signature verified but the key's day has passed.
    Expired,
    /// The signature verified but this key is locked to a different machine.
    WrongMachine,
}

impl core::fmt::Display for LicenseError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let s = match self {
            LicenseError::Invalid => "that key is not valid",
            LicenseError::Expired => "that key has expired",
            LicenseError::WrongMachine => "that key belongs to a different computer",
        };
        f.write_str(s)
    }
}

impl std::error::Error for LicenseError {}

/// Hash a machine fingerprint string down to the 8 bytes stored in a node-locked key.
/// The app computes the fingerprint (it is OS-specific) and passes it here so the format
/// and the hashing live in exactly one place.
pub fn machine_tag(fingerprint: &str) -> [u8; 8] {
    let digest = Sha512::digest(fingerprint.as_bytes());
    let mut out = [0u8; 8];
    out.copy_from_slice(&digest[..8]);
    out
}

#[cfg(feature = "sign")]
fn build_payload(license: &License) -> [u8; PAYLOAD_LEN] {
    let mut p = [0u8; PAYLOAD_LEN];
    p[0] = MAGIC;
    p[1] = VERSION;
    p[2] = license.edition.to_byte();
    p[3] = if license.node_locked { 1 } else { 0 };
    let days = license.expires_day.unwrap_or(0);
    p[4..8].copy_from_slice(&days.to_le_bytes());
    if license.node_locked {
        p[8..16].copy_from_slice(&license.machine);
    }
    p
}

fn parse_payload(p: &[u8; PAYLOAD_LEN]) -> Option<License> {
    if p[0] != MAGIC || p[1] != VERSION {
        return None;
    }
    let edition = Edition::from_byte(p[2])?;
    let node_locked = match p[3] {
        0 => false,
        1 => true,
        _ => return None,
    };
    let days = u32::from_le_bytes([p[4], p[5], p[6], p[7]]);
    let mut machine = [0u8; 8];
    machine.copy_from_slice(&p[8..16]);
    Some(License { edition, expires_day: if days == 0 { None } else { Some(days) }, node_locked, machine })
}

/// Decode a user-entered key and verify it against the embedded public key.
///
/// `today_day` is the current day number (days since the Unix epoch); pass it in so the
/// core stays free of a clock and is trivial to test. `machine` is this computer's tag
/// from [`machine_tag`]; it is only consulted for node-locked keys.
pub fn verify(public_key: &[u8; 32], key: &str, today_day: u32, machine: &[u8; 8]) -> Result<License, LicenseError> {
    let blob = base32_decode(key).ok_or(LicenseError::Invalid)?;
    if blob.len() != BLOB_LEN {
        return Err(LicenseError::Invalid);
    }
    let mut payload = [0u8; PAYLOAD_LEN];
    payload.copy_from_slice(&blob[..PAYLOAD_LEN]);
    let mut sig_bytes = [0u8; SIG_LEN];
    sig_bytes.copy_from_slice(&blob[PAYLOAD_LEN..]);

    let vk = VerifyingKey::from_bytes(public_key).map_err(|_| LicenseError::Invalid)?;
    let sig = Signature::from_bytes(&sig_bytes);
    // verify_strict rejects the small-order edge cases that plain verify accepts.
    vk.verify_strict(&payload, &sig).map_err(|_| LicenseError::Invalid)?;

    let license = parse_payload(&payload).ok_or(LicenseError::Invalid)?;
    if let Some(day) = license.expires_day {
        if today_day > day {
            return Err(LicenseError::Expired);
        }
    }
    if license.node_locked && &license.machine != machine {
        return Err(LicenseError::WrongMachine);
    }
    Ok(license)
}

/// Sign a license into a key string. Only compiled into the keygen (feature = "sign").
#[cfg(feature = "sign")]
pub fn sign(signing_key: &ed25519_dalek::SigningKey, license: &License) -> String {
    use ed25519_dalek::Signer;
    let payload = build_payload(license);
    let sig = signing_key.sign(&payload);
    let mut blob = Vec::with_capacity(BLOB_LEN);
    blob.extend_from_slice(&payload);
    blob.extend_from_slice(&sig.to_bytes());
    group(&base32_encode(&blob))
}

/// Group a bare base32 string into dashed blocks of five for readability.
#[cfg(feature = "sign")]
fn group(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + s.len() / 5);
    for (i, c) in s.chars().enumerate() {
        if i > 0 && i % 5 == 0 {
            out.push('-');
        }
        out.push(c);
    }
    out
}

// ---- Crockford base32 (no padding). Dashes and spaces are ignored on decode, and the
// decoder folds the usual look-alikes (I/L -> 1, O -> 0) so a retyped key still works.

const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// Encode bytes as Crockford base32 without padding.
pub fn base32_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len() * 8 / 5 + 1);
    let mut buffer: u32 = 0;
    let mut bits: u32 = 0;
    for &b in data {
        buffer = (buffer << 8) | b as u32;
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            let idx = ((buffer >> bits) & 0x1f) as usize;
            out.push(ALPHABET[idx] as char);
        }
    }
    if bits > 0 {
        let idx = ((buffer << (5 - bits)) & 0x1f) as usize;
        out.push(ALPHABET[idx] as char);
    }
    out
}

fn decode_symbol(c: char) -> Option<u8> {
    let up = c.to_ascii_uppercase();
    match up {
        '0' | 'O' => Some(0),
        '1' | 'I' | 'L' => Some(1),
        '2' => Some(2),
        '3' => Some(3),
        '4' => Some(4),
        '5' => Some(5),
        '6' => Some(6),
        '7' => Some(7),
        '8' => Some(8),
        '9' => Some(9),
        'A' => Some(10),
        'B' => Some(11),
        'C' => Some(12),
        'D' => Some(13),
        'E' => Some(14),
        'F' => Some(15),
        'G' => Some(16),
        'H' => Some(17),
        'J' => Some(18),
        'K' => Some(19),
        'M' => Some(20),
        'N' => Some(21),
        'P' => Some(22),
        'Q' => Some(23),
        'R' => Some(24),
        'S' => Some(25),
        'T' => Some(26),
        'V' => Some(27),
        'W' => Some(28),
        'X' => Some(29),
        'Y' => Some(30),
        'Z' => Some(31),
        _ => None,
    }
}

/// Decode Crockford base32, ignoring dashes and whitespace. Returns `None` on any bad symbol.
pub fn base32_decode(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() * 5 / 8 + 1);
    let mut buffer: u32 = 0;
    let mut bits: u32 = 0;
    for c in s.chars() {
        if c == '-' || c.is_whitespace() {
            continue;
        }
        let v = decode_symbol(c)? as u32;
        buffer = (buffer << 5) | v;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base32_roundtrip() {
        for n in 0..90usize {
            let data: Vec<u8> = (0..n).map(|i| (i * 37 + 11) as u8).collect();
            let enc = base32_encode(&data);
            let dec = base32_decode(&enc).unwrap();
            assert_eq!(dec, data, "roundtrip failed at len {n}");
        }
    }

    #[test]
    fn verify_rejects_garbage() {
        // A key that is not even base32, and one of the right shape but random bytes.
        let pk = [7u8; 32];
        assert_eq!(verify(&pk, "not a key!!!", 0, &[0; 8]), Err(LicenseError::Invalid));
        let random = base32_encode(&[0xABu8; 80]);
        assert_eq!(verify(&pk, &random, 0, &[0; 8]), Err(LicenseError::Invalid));
    }

    #[cfg(feature = "sign")]
    #[test]
    fn signed_key_roundtrips_and_expiry_is_enforced() {
        use ed25519_dalek::SigningKey;
        let sk = SigningKey::from_bytes(&[42u8; 32]);
        let pk = sk.verifying_key().to_bytes();

        let perpetual = License { edition: Edition::Pro, expires_day: None, node_locked: false, machine: [0; 8] };
        let key = sign(&sk, &perpetual);
        assert_eq!(verify(&pk, &key, 20_000, &[0; 8]), Ok(perpetual));

        // Expires on day 100: valid up to and including 100, rejected after.
        let dated = License { edition: Edition::Standard, expires_day: Some(100), node_locked: false, machine: [0; 8] };
        let key = sign(&sk, &dated);
        assert_eq!(verify(&pk, &key, 100, &[0; 8]), Ok(dated));
        assert_eq!(verify(&pk, &key, 101, &[0; 8]), Err(LicenseError::Expired));

        // A key signed by a DIFFERENT private key never verifies — the forgery guarantee.
        let other = SigningKey::from_bytes(&[99u8; 32]);
        let forged = sign(&other, &perpetual);
        assert_eq!(verify(&pk, &forged, 0, &[0; 8]), Err(LicenseError::Invalid));
    }

    #[test]
    fn decode_tolerates_dashes_and_case() {
        let enc = base32_encode(b"hello world");
        let grouped: String = enc
            .chars()
            .enumerate()
            .flat_map(|(i, c)| if i % 4 == 0 && i > 0 { vec!['-', c] } else { vec![c] })
            .collect();
        let lowered = grouped.to_lowercase();
        assert_eq!(base32_decode(&lowered).unwrap(), b"hello world");
    }
}

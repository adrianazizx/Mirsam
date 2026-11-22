//! The standard security handler (ISO 32000-2, 7.6.4), for writing.
//!
//! In an encrypted file every string and stream is encrypted with a key
//! derived from the file key and the object's number. An object an
//! incremental update *rewrites* keeps its number, so its strings (copied as
//! read) stay valid. An object the update *adds* is written in plain text by
//! the writer, and must be encrypted here before it goes in.
//!
//! The file key comes from the password the document was opened with (the
//! empty user password when it opened without one), checked against the
//! file's `/U` or `/O` entry. Supported: RC4 (40 to 128 bits, revisions 2–4),
//! AES-128 (`/AESV2`), and AES-256 (`/AESV3`, revisions 5 and 6), with
//! `/Identity` crypt filters. Public-key handlers are not.

use aes::cipher::{Array, BlockCipherDecrypt, BlockCipherEncrypt, KeyInit};
use aes::{Aes128, Aes256};
use md5::{Digest, Md5};
use sha2::{Sha256, Sha384, Sha512};

use super::{Dict, File, Object};
use crate::error::{Error, Result};

/// Pads short passwords in revisions 2–4 (Algorithm 2, step a).
const PADDING: [u8; 32] = [
    0x28, 0xBF, 0x4E, 0x5E, 0x4E, 0x75, 0x8A, 0x41, 0x64, 0x00, 0x4E, 0x56, 0xFF, 0xFA, 0x01, 0x08, 0x2E,
    0x2E, 0x00, 0xB6, 0xD0, 0x68, 0x3E, 0x80, 0x2F, 0x0C, 0xA9, 0xFE, 0x64, 0x53, 0x69, 0x7A,
];

/// How one kind of data (strings or streams) is encrypted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Method {
    Identity,
    Rc4,
    /// AES-128-CBC with a per-object key.
    AesV2,
    /// AES-256-CBC with the file key itself.
    AesV3,
}

/// A file's unlocked security handler.
#[derive(Clone)]
pub(crate) struct Crypt {
    key: Vec<u8>,
    strings: Method,
    streams: Method,
}

impl std::fmt::Debug for Crypt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never the key.
        f.debug_struct("Crypt").field("strings", &self.strings).field("streams", &self.streams).finish()
    }
}

impl Crypt {
    /// The file's security handler unlocked with `password`, or `None` when
    /// the file is not encrypted.
    pub(crate) fn of(file: &File<'_>, password: Option<&str>) -> Result<Option<Self>> {
        let Some(reference) = file.trailer.get(b"Encrypt") else { return Ok(None) };
        let unsupported = || Error::EditUnsupported("its encryption is of a kind this editor cannot write");
        let dict = file.resolve(reference).and_then(|d| d.as_dict().cloned()).ok_or_else(unsupported)?;
        if dict.get(b"Filter").and_then(Object::as_name) != Some(b"Standard") {
            return Err(unsupported());
        }
        let int = |key: &[u8]| dict.get(key).and_then(Object::as_int);
        let string = |key: &[u8]| match dict.get(key) {
            Some(Object::Str(bytes)) => Some(bytes.clone()),
            _ => None,
        };
        let version = int(b"V").unwrap_or(0);
        let revision = int(b"R").ok_or_else(unsupported)?;
        let (owner, user) = (string(b"O").ok_or_else(unsupported)?, string(b"U").ok_or_else(unsupported)?);
        let password = password.unwrap_or("");

        let (strings, streams, key_length) = match version {
            1 => (Method::Rc4, Method::Rc4, 5),
            2 => {
                let bits = int(b"Length").unwrap_or(40);
                if !(40..=128).contains(&bits) || bits % 8 != 0 {
                    return Err(unsupported());
                }
                (Method::Rc4, Method::Rc4, usize::try_from(bits / 8).map_err(|_| unsupported())?)
            }
            4 | 5 => {
                let filter = |key: &[u8]| -> Result<Method> {
                    let name = dict.get(key).and_then(Object::as_name).unwrap_or(b"Identity");
                    if name == b"Identity" {
                        return Ok(Method::Identity);
                    }
                    let method = dict
                        .get(b"CF")
                        .and_then(|cf| file.resolve(cf))
                        .and_then(|cf| cf.as_dict().and_then(|cf| cf.get(name)).cloned())
                        .and_then(|cf| file.resolve(&cf))
                        .and_then(|cf| cf.as_dict().and_then(|cf| cf.get(b"CFM")).cloned());
                    match method.as_ref().and_then(Object::as_name) {
                        None | Some(b"None") => Ok(Method::Identity),
                        Some(b"V2") => Ok(Method::Rc4),
                        Some(b"AESV2") => Ok(Method::AesV2),
                        Some(b"AESV3") => Ok(Method::AesV3),
                        Some(_) => Err(unsupported()),
                    }
                };
                (filter(b"StrF")?, filter(b"StmF")?, if version == 5 { 32 } else { 16 })
            }
            _ => return Err(unsupported()),
        };

        let wrong = || Error::EditUnsupported("its password could not be confirmed for writing");
        let key = match revision {
            2..=4 => {
                let id = match file.trailer.get(b"ID").and_then(Object::as_array).and_then(|id| id.first()) {
                    Some(Object::Str(id)) => id.clone(),
                    _ => Vec::new(),
                };
                let legacy = Legacy {
                    revision,
                    key_length,
                    owner: &owner,
                    user: &user,
                    permissions: int(b"P").ok_or_else(unsupported)?,
                    id: &id,
                    encrypt_metadata: !matches!(dict.get(b"EncryptMetadata"), Some(Object::Bool(false))),
                };
                let typed = latin1(password).ok_or_else(wrong)?;
                legacy.user_key(&padded(&typed)).or_else(|| legacy.owner_key(&typed)).ok_or_else(wrong)?
            }
            5 | 6 => {
                let typed = password.as_bytes();
                let typed = &typed[..typed.len().min(127)];
                let (owner_e, user_e) =
                    (string(b"OE").ok_or_else(unsupported)?, string(b"UE").ok_or_else(unsupported)?);
                modern_key(revision, typed, &owner, &user, &owner_e, &user_e).ok_or_else(wrong)?
            }
            _ => return Err(unsupported()),
        };
        Ok(Some(Self { key, strings, streams }))
    }

    /// Encrypts, in place, every string in `object` (the value of object
    /// `number`), which must have been built in plain text.
    pub(crate) fn encrypt_strings(&self, number: u32, generation: u16, object: &mut Object) {
        match object {
            Object::Str(bytes) => *bytes = self.apply(self.strings, number, generation, bytes),
            Object::Array(items) => {
                for item in items {
                    self.encrypt_strings(number, generation, item);
                }
            }
            Object::Dict(Dict(entries)) => {
                for (_, value) in entries {
                    self.encrypt_strings(number, generation, value);
                }
            }
            _ => {}
        }
    }

    /// The data of stream object `number`, encrypted.
    pub(crate) fn encrypt_stream(&self, number: u32, generation: u16, data: &[u8]) -> Vec<u8> {
        self.apply(self.streams, number, generation, data)
    }

    fn apply(&self, method: Method, number: u32, generation: u16, data: &[u8]) -> Vec<u8> {
        match method {
            Method::Identity => data.to_vec(),
            Method::Rc4 => rc4(&self.object_key(number, generation, false), data),
            Method::AesV2 => aes_encrypt(&self.object_key(number, generation, true), data),
            Method::AesV3 => aes_encrypt(&self.key, data),
        }
    }

    /// Algorithm 1: the file key extended with the object's number.
    fn object_key(&self, number: u32, generation: u16, aes: bool) -> Vec<u8> {
        let mut hasher = Md5::new();
        hasher.update(&self.key);
        hasher.update(&number.to_le_bytes()[..3]);
        hasher.update(generation.to_le_bytes());
        if aes {
            hasher.update(b"sAlT");
        }
        let digest = hasher.finalize();
        digest[..(self.key.len() + 5).min(16)].to_vec()
    }

    /// A string of object `number` as written, decrypted (a string read from
    /// the file, e.g. a `/DA` to parse).
    pub(crate) fn decrypt_string(&self, number: u32, generation: u16, data: &[u8]) -> Option<Vec<u8>> {
        self.reverse(self.strings, number, generation, data)
    }

    /// The inverse of [`encrypt_stream`](Self::encrypt_stream), for tests.
    #[cfg(test)]
    fn decrypt_stream(&self, number: u32, generation: u16, data: &[u8]) -> Option<Vec<u8>> {
        self.reverse(self.streams, number, generation, data)
    }

    fn reverse(&self, method: Method, number: u32, generation: u16, data: &[u8]) -> Option<Vec<u8>> {
        match method {
            Method::Identity => Some(data.to_vec()),
            Method::Rc4 => Some(rc4(&self.object_key(number, generation, false), data)),
            Method::AesV2 => aes_decrypt(&self.object_key(number, generation, true), data),
            Method::AesV3 => aes_decrypt(&self.key, data),
        }
    }
}

/// The entries revisions 2–4 derive the key from.
struct Legacy<'a> {
    revision: i64,
    key_length: usize,
    owner: &'a [u8],
    user: &'a [u8],
    permissions: i64,
    id: &'a [u8],
    encrypt_metadata: bool,
}

impl Legacy<'_> {
    /// Algorithm 2: the file key for a padded user password.
    fn key(&self, padded: &[u8; 32]) -> Vec<u8> {
        let mut hasher = Md5::new();
        hasher.update(padded);
        hasher.update(&self.owner[..self.owner.len().min(32)]);
        // /P is a signed 32-bit value; its low four bytes, low-order first.
        hasher.update((self.permissions as u32).to_le_bytes());
        hasher.update(self.id);
        if self.revision >= 4 && !self.encrypt_metadata {
            hasher.update([0xFF; 4]);
        }
        let mut hash = hasher.finalize().to_vec();
        if self.revision >= 3 {
            for _ in 0..50 {
                hash = Md5::digest(&hash[..self.key_length]).to_vec();
            }
        }
        hash.truncate(self.key_length);
        hash
    }

    /// Algorithms 4–6: the key, if `padded` is the user password.
    fn user_key(&self, padded: &[u8; 32]) -> Option<Vec<u8>> {
        let key = self.key(padded);
        let matches = if self.revision == 2 {
            rc4(&key, &PADDING) == self.user.get(..32)?
        } else {
            let mut hasher = Md5::new();
            hasher.update(PADDING);
            hasher.update(self.id);
            let mut value = hasher.finalize().to_vec();
            for i in 0..20u8 {
                value = rc4(&xored(&key, i), &value);
            }
            value[..16] == *self.user.get(..16)?
        };
        matches.then_some(key)
    }

    /// Algorithm 7: the key, if `typed` is the owner password (which unlocks
    /// the user password stored in `/O`).
    fn owner_key(&self, typed: &[u8]) -> Option<Vec<u8>> {
        let mut hash = Md5::digest(padded(typed)).to_vec();
        if self.revision >= 3 {
            for _ in 0..50 {
                hash = Md5::digest(&hash).to_vec();
            }
        }
        let key = &hash[..self.key_length];
        let mut user = self.owner.get(..32)?.to_vec();
        if self.revision == 2 {
            user = rc4(key, &user);
        } else {
            for i in (0..20u8).rev() {
                user = rc4(&xored(key, i), &user);
            }
        }
        self.user_key(&user.try_into().ok()?)
    }
}

/// Algorithm 2.A: the file key of revisions 5 and 6, unlocked by the owner
/// or the user password.
fn modern_key(
    revision: i64,
    typed: &[u8],
    owner: &[u8],
    user: &[u8],
    owner_e: &[u8],
    user_e: &[u8],
) -> Option<Vec<u8>> {
    let (owner, user) = (owner.get(..48)?, user.get(..48)?);
    let (o_hash, o_check, o_key) = (&owner[..32], &owner[32..40], &owner[40..48]);
    let (u_hash, u_check, u_key) = (&user[..32], &user[32..40], &user[40..48]);
    let (intermediate, encrypted) = if hash_r6(revision, typed, o_check, user) == o_hash {
        (hash_r6(revision, typed, o_key, user), owner_e)
    } else if hash_r6(revision, typed, u_check, &[]) == u_hash {
        (hash_r6(revision, typed, u_key, &[]), user_e)
    } else {
        return None;
    };
    // The file key is stored encrypted with AES-256, no padding, a zero IV.
    let cipher = Aes256::new_from_slice(&intermediate).ok()?;
    let mut previous = [0u8; 16];
    let mut key = Vec::with_capacity(32);
    for chunk in encrypted.get(..32)?.as_chunks::<16>().0 {
        let mut block = Array::from(*chunk);
        cipher.decrypt_block(&mut block);
        key.extend(block.iter().zip(previous).map(|(a, b)| a ^ b));
        previous.copy_from_slice(chunk);
    }
    Some(key)
}

/// Algorithm 2.B (revision 6), or the single SHA-256 of revision 5.
fn hash_r6(revision: i64, password: &[u8], salt: &[u8], extra: &[u8]) -> [u8; 32] {
    let mut k: Vec<u8> =
        Sha256::new().chain_update(password).chain_update(salt).chain_update(extra).finalize().to_vec();
    if revision == 6 {
        let mut round: u32 = 0;
        loop {
            let mut k1 = Vec::with_capacity(64 * (password.len() + k.len() + extra.len()));
            for _ in 0..64 {
                k1.extend_from_slice(password);
                k1.extend_from_slice(&k);
                k1.extend_from_slice(extra);
            }
            // k1 is 64 repeats, so a multiple of 16 bytes: no padding.
            let mut iv = [0u8; 16];
            iv.copy_from_slice(&k[16..32]);
            let e = cbc_encrypt(&k[..16], iv, &k1).unwrap_or_default();
            // The first 16 bytes as a number, mod 3 (256 ≡ 1 mod 3).
            k = match e.iter().take(16).map(|&b| u32::from(b)).sum::<u32>() % 3 {
                0 => Sha256::digest(&e).to_vec(),
                1 => Sha384::digest(&e).to_vec(),
                _ => Sha512::digest(&e).to_vec(),
            };
            round += 1;
            let last = u32::from(e.last().copied().unwrap_or(0));
            if round >= 64 && last <= round - 32 {
                break;
            }
        }
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(&k[..32]);
    out
}

/// A password typed for revisions 2–4: PDFDocEncoding, which agrees with
/// Latin-1 for the characters people type.
fn latin1(password: &str) -> Option<Vec<u8>> {
    password.chars().map(|c| u8::try_from(u32::from(c)).ok()).collect()
}

fn padded(typed: &[u8]) -> [u8; 32] {
    let mut out = PADDING;
    let n = typed.len().min(32);
    out[..n].copy_from_slice(&typed[..n]);
    out[n..].copy_from_slice(&PADDING[..32 - n]);
    out
}

fn xored(key: &[u8], value: u8) -> Vec<u8> {
    key.iter().map(|b| b ^ value).collect()
}

fn rc4(key: &[u8], data: &[u8]) -> Vec<u8> {
    if key.is_empty() {
        return data.to_vec();
    }
    let mut s: [u8; 256] = std::array::from_fn(|i| i as u8);
    let mut j: u8 = 0;
    for i in 0..256 {
        j = j.wrapping_add(s[i]).wrapping_add(key[i % key.len()]);
        s.swap(i, usize::from(j));
    }
    let (mut i, mut j) = (0u8, 0u8);
    data.iter()
        .map(|&byte| {
            i = i.wrapping_add(1);
            j = j.wrapping_add(s[usize::from(i)]);
            s.swap(usize::from(i), usize::from(j));
            byte ^ s[usize::from(s[usize::from(i)].wrapping_add(s[usize::from(j)]))]
        })
        .collect()
}

/// AES-CBC with a 16- or 32-byte key over whole blocks.
fn cbc_encrypt(key: &[u8], iv: [u8; 16], data: &[u8]) -> Option<Vec<u8>> {
    enum Cipher {
        Short(Aes128),
        Long(Aes256),
    }
    let cipher = match key.len() {
        16 => Cipher::Short(Aes128::new_from_slice(key).ok()?),
        32 => Cipher::Long(Aes256::new_from_slice(key).ok()?),
        _ => return None,
    };
    let mut previous = iv;
    let mut out = Vec::with_capacity(data.len());
    for chunk in data.as_chunks::<16>().0 {
        let mut plain = [0u8; 16];
        for (slot, (a, b)) in plain.iter_mut().zip(chunk.iter().zip(previous)) {
            *slot = a ^ b;
        }
        let mut block = Array::from(plain);
        match &cipher {
            Cipher::Short(c) => c.encrypt_block(&mut block),
            Cipher::Long(c) => c.encrypt_block(&mut block),
        }
        previous.copy_from_slice(&block);
        out.extend_from_slice(&block);
    }
    Some(out)
}

/// AES-CBC as PDF stores it: a random 16-byte IV, then the data padded to
/// whole blocks (PKCS #7).
fn aes_encrypt(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut iv = [0u8; 16];
    if getrandom::fill(&mut iv).is_err() {
        // Without the system's randomness, an IV from the data and key. A
        // PDF's IV only has to differ between objects; nothing interactive
        // depends on it being unpredictable.
        let mut hasher = Md5::new();
        hasher.update(key);
        hasher.update(data);
        iv.copy_from_slice(&hasher.finalize());
    }
    let pad = 16 - data.len() % 16;
    let mut padded = Vec::with_capacity(data.len() + pad);
    padded.extend_from_slice(data);
    padded.resize(data.len() + pad, u8::try_from(pad).unwrap_or(16));
    let mut out = iv.to_vec();
    out.extend(cbc_encrypt(key, iv, &padded).unwrap_or_default());
    out
}

fn aes_decrypt(key: &[u8], data: &[u8]) -> Option<Vec<u8>> {
    if data.len() < 32 || !data.len().is_multiple_of(16) {
        return None;
    }
    let (iv, body) = data.split_at(16);
    let mut previous: [u8; 16] = iv.try_into().ok()?;
    let mut out = Vec::with_capacity(body.len());
    for chunk in body.as_chunks::<16>().0 {
        let mut block = Array::from(*chunk);
        match key.len() {
            16 => Aes128::new_from_slice(key).ok()?.decrypt_block(&mut block),
            32 => Aes256::new_from_slice(key).ok()?.decrypt_block(&mut block),
            _ => return None,
        }
        out.extend(block.iter().zip(previous).map(|(a, b)| a ^ b));
        previous.copy_from_slice(chunk);
    }
    let pad = usize::from(*out.last()?);
    if !(1..=16).contains(&pad) {
        return None;
    }
    out.truncate(out.len() - pad);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rc4_matches_the_published_vectors() {
        // Test vectors from RC4's description (key "Key", "Wiki", "Secret").
        assert_eq!(rc4(b"Key", b"Plaintext"), [0xBB, 0xF3, 0x16, 0xE8, 0xD9, 0x40, 0xAF, 0x0A, 0xD3]);
        assert_eq!(rc4(b"Wiki", b"pedia"), [0x10, 0x21, 0xBF, 0x04, 0x20]);
        assert_eq!(rc4(b"Secret", b"Attack at dawn")[..4], [0x45, 0xA0, 0x1F, 0x64]);
    }

    #[test]
    fn aes_cbc_matches_nist_and_round_trips() {
        // NIST SP 800-38A, F.2.1 (CBC-AES128.Encrypt), first block.
        let key =
            [0x2b, 0x7e, 0x15, 0x16, 0x28, 0xae, 0xd2, 0xa6, 0xab, 0xf7, 0x15, 0x88, 0x09, 0xcf, 0x4f, 0x3c];
        let iv: [u8; 16] = std::array::from_fn(|i| i as u8);
        let plain =
            [0x6b, 0xc1, 0xbe, 0xe2, 0x2e, 0x40, 0x9f, 0x96, 0xe9, 0x3d, 0x7e, 0x11, 0x73, 0x93, 0x17, 0x2a];
        let expected =
            [0x76, 0x49, 0xab, 0xac, 0x81, 0x19, 0xb2, 0x46, 0xce, 0xe9, 0x8e, 0x9b, 0x12, 0xe9, 0x19, 0x7d];
        assert_eq!(cbc_encrypt(&key, iv, &plain), Some(expected.to_vec()));

        for data in [&b""[..], b"BT /F1 12 Tf ET", &[7u8; 32]] {
            let encrypted = aes_encrypt(&[9u8; 32], data);
            assert_eq!(encrypted.len() % 16, 0);
            assert_eq!(aes_decrypt(&[9u8; 32], &encrypted).as_deref(), Some(data));
        }
    }

    #[test]
    fn a_stream_round_trips_with_each_method() {
        for (streams, key) in
            [(Method::Rc4, vec![1u8; 5]), (Method::AesV2, vec![2u8; 16]), (Method::AesV3, vec![3u8; 32])]
        {
            let crypt = Crypt { key, strings: streams, streams };
            let encrypted = crypt.encrypt_stream(12, 0, b"q 1 0 0 rg 0 0 10 10 re f Q");
            assert_ne!(encrypted, b"q 1 0 0 rg 0 0 10 10 re f Q");
            assert_eq!(
                crypt.decrypt_stream(12, 0, &encrypted).as_deref(),
                Some(&b"q 1 0 0 rg 0 0 10 10 re f Q"[..])
            );
            // Before AES-256, each object has its own key: another object's does not decrypt it.
            if streams != Method::AesV3 {
                assert_ne!(
                    crypt.decrypt_stream(13, 0, &encrypted).as_deref(),
                    Some(&b"q 1 0 0 rg 0 0 10 10 re f Q"[..])
                );
            }
        }
    }

    #[test]
    fn identity_filters_leave_data_alone() {
        let crypt = Crypt { key: vec![1; 16], strings: Method::Identity, streams: Method::Identity };
        let mut object = Object::Array(vec![Object::Str(b"abc".to_vec())]);
        crypt.encrypt_strings(5, 0, &mut object);
        assert_eq!(object, Object::Array(vec![Object::Str(b"abc".to_vec())]));
        assert_eq!(crypt.encrypt_stream(5, 0, b"data"), b"data");
    }
}

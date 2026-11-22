//! Encryption and permissions (ADR 0043): what a file is protected with, and
//! how a save changes it.
//!
//! The standard security handler (ISO 32000-2, 7.6.4) has two passwords. The
//! **user** password opens the file (empty: anyone may open it). The **owner**
//! password also lifts the permissions, which are flags a conforming viewer
//! honours: printing, copying text, editing, annotating, and so on.

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::pdf::crypt::Cipher as Handler;

/// What a reader may do with a file opened with the user password (the `/P`
/// entry). Everything is allowed unless the file says otherwise.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Permissions {
    pub print: bool,
    /// Print at full quality (without it, only a degraded copy may print).
    pub print_high_quality: bool,
    /// Change the document's content.
    pub modify: bool,
    /// Copy text and graphics out.
    pub copy: bool,
    /// Add or change annotations, and fill form fields.
    pub annotate: bool,
    /// Fill in existing form fields (even where `annotate` is off).
    pub fill_forms: bool,
    /// Extract text for accessibility tools.
    pub accessibility: bool,
    /// Insert, delete, and rotate pages.
    pub assemble: bool,
}

const PRINT: u32 = 1 << 2;
const MODIFY: u32 = 1 << 3;
const COPY: u32 = 1 << 4;
const ANNOTATE: u32 = 1 << 5;
const FILL_FORMS: u32 = 1 << 8;
const ACCESSIBILITY: u32 = 1 << 9;
const ASSEMBLE: u32 = 1 << 10;
const PRINT_HIGH: u32 = 1 << 11;

impl Permissions {
    pub const fn all() -> Self {
        Self {
            print: true,
            print_high_quality: true,
            modify: true,
            copy: true,
            annotate: true,
            fill_forms: true,
            accessibility: true,
            assemble: true,
        }
    }

    /// The `/P` value: bits 1–2 clear, bits 7–8 and 13–32 set (reserved), the
    /// rest the flags.
    pub fn bits(&self) -> u32 {
        let mut bits = 0xFFFF_F0C0;
        for (on, bit) in [
            (self.print, PRINT),
            (self.modify, MODIFY),
            (self.copy, COPY),
            (self.annotate, ANNOTATE),
            (self.fill_forms, FILL_FORMS),
            (self.accessibility, ACCESSIBILITY),
            (self.assemble, ASSEMBLE),
            (self.print && self.print_high_quality, PRINT_HIGH),
        ] {
            if on {
                bits |= bit;
            }
        }
        bits
    }

    pub fn from_bits(bits: u32) -> Self {
        let has = |bit: u32| bits & bit != 0;
        Self {
            print: has(PRINT),
            print_high_quality: has(PRINT) && has(PRINT_HIGH),
            modify: has(MODIFY),
            copy: has(COPY),
            annotate: has(ANNOTATE),
            fill_forms: has(FILL_FORMS),
            accessibility: has(ACCESSIBILITY),
            assemble: has(ASSEMBLE),
        }
    }
}

/// The ciphers a file can be written with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Cipher {
    /// AES-256 (revision 6): the default. Passwords may be any Unicode text.
    Aes256,
    /// AES-128 (revision 4): for old readers. Passwords must be ASCII.
    Aes128,
}

/// What a saved file is protected with.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Protection {
    pub cipher: Cipher,
    /// Opens the file; empty means no password is needed to open it.
    #[serde(default)]
    pub user_password: String,
    /// Lifts the permissions; empty means a random one nobody knows, so the
    /// permissions cannot be lifted.
    #[serde(default)]
    pub owner_password: String,
    pub permissions: Permissions,
}

impl std::fmt::Debug for Protection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never the passwords.
        f.debug_struct("Protection")
            .field("cipher", &self.cipher)
            .field("permissions", &self.permissions)
            .finish()
    }
}

/// What a save does to the file's protection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SecurityChange {
    /// As the file is now: encrypted stays encrypted, the same way.
    Keep,
    /// Save without a password or permissions.
    Remove,
    /// Save protected with these passwords and permissions.
    Set(Protection),
}

/// How a file's data is encrypted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CipherName {
    Rc4,
    Aes128,
    Aes256,
}

/// A document's protection, as opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SecurityInfo {
    pub encrypted: bool,
    pub cipher: Option<CipherName>,
    /// The security handler's revision (2–6); 0 when not encrypted.
    pub revision: i64,
    pub permissions: Permissions,
    /// Opened with the owner password (or not encrypted): nothing is
    /// restricted, and the protection may be changed.
    pub owner: bool,
}

impl SecurityInfo {
    pub(crate) fn open() -> Self {
        Self { encrypted: false, cipher: None, revision: 0, permissions: Permissions::all(), owner: true }
    }

    pub(crate) fn of(crypt: &crate::pdf::crypt::Crypt) -> Self {
        Self {
            encrypted: true,
            cipher: Some(match crypt.cipher() {
                Handler::Rc4 => CipherName::Rc4,
                Handler::Aes128 => CipherName::Aes128,
                Handler::Aes256 => CipherName::Aes256,
            }),
            revision: crypt.revision,
            permissions: Permissions::from_bits(crypt.permissions),
            owner: crypt.owner,
        }
    }

    /// Whether what the user opened with lets them do `flag`: the owner may
    /// do anything.
    pub fn allows(&self, flag: impl Fn(&Permissions) -> bool) -> bool {
        self.owner || flag(&self.permissions)
    }
}

impl Protection {
    pub(crate) fn handler(&self) -> Handler {
        match self.cipher {
            Cipher::Aes256 => Handler::Aes256,
            Cipher::Aes128 => Handler::Aes128,
        }
    }

    /// The password the file will open with, as it must be given again: the
    /// normalised form for AES-256. `None` when no password is needed.
    pub fn opening_password(&self) -> Option<String> {
        if self.user_password.is_empty() {
            return None;
        }
        Some(match self.cipher {
            Cipher::Aes256 => crate::pdf::crypt::normalise_password(&self.user_password),
            Cipher::Aes128 => self.user_password.clone(),
        })
    }

    pub(crate) fn check(&self) -> Result<()> {
        if self.cipher == Cipher::Aes128 && !(self.user_password.is_ascii() && self.owner_password.is_ascii())
        {
            return Err(Error::PasswordNotEncodable);
        }
        if self.user_password.chars().any(char::is_control)
            || self.owner_password.chars().any(char::is_control)
        {
            return Err(Error::PasswordNotEncodable);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permission_bits_round_trip_and_reserve_the_right_bits() {
        assert_eq!(Permissions::all().bits(), 0xFFFF_FFFC);
        let none = Permissions {
            print: false,
            print_high_quality: false,
            modify: false,
            copy: false,
            annotate: false,
            fill_forms: false,
            accessibility: false,
            assemble: false,
        };
        // The reserved bits stay set, as the standard requires.
        assert_eq!(none.bits(), 0xFFFF_F0C0);
        let some = Permissions { copy: false, ..Permissions::all() };
        assert_eq!(Permissions::from_bits(some.bits()), some);
        // High-quality printing means nothing without printing.
        let odd = Permissions { print: false, ..Permissions::all() };
        assert!(!Permissions::from_bits(odd.bits()).print_high_quality);
    }
}

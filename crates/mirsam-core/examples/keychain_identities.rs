//! Lists the signing identities the app would offer from the macOS keychain
//! (ADR 0048). Listing asks nothing of the user; only signing does.
//!
//! ```text
//! cargo run -p mirsam-core --example keychain_identities
//! ```

#[cfg(target_os = "macos")]
fn main() {
    match mirsam_core::sign::keychain::identities() {
        Ok(found) if found.is_empty() => println!("no signing identities in the keychain"),
        Ok(found) => {
            for identity in found {
                println!(
                    "{} — issued by {} — key {:?} — SHA-256 {}",
                    identity.name, identity.issuer, identity.key, identity.fingerprint
                );
            }
        }
        Err(err) => eprintln!("the keychain could not be searched: {err}"),
    }
}

#[cfg(not(target_os = "macos"))]
fn main() {
    println!("the keychain is macOS only; elsewhere certificates come from PKCS#12 files");
}

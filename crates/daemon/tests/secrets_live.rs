//! The real keyring (Secret Service on Linux, the Keychain on macOS): opt-in, it writes one item to the owner's
//! keyring (`service=singularity account=probe`) and removes it. `cargo test -p singularityd --test secrets_live -- --ignored`

use singularityd::secrets::{self, Stored};

#[test]
#[ignore]
fn the_keyring_keeps_a_secret_and_forgets_it() {
    let value = "{\"user\": \"x\", \"blob\": \"a b\\\"c\"}";
    assert_eq!(secrets::set("probe", value).unwrap(), Stored::Keyring, "no {} reachable", secrets::KEYRING);
    assert_eq!(secrets::in_keyring("probe").as_deref(), Some(value));
    assert_eq!(secrets::set("probe", "second").unwrap(), Stored::Keyring, "replaced in place");
    assert_eq!(secrets::get("probe").as_deref(), Some("second"));
    secrets::clear("probe");
    assert_eq!(secrets::in_keyring("probe"), None);
}

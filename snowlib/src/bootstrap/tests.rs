//! Integration tests for the bootstrap module

use super::*;
use crate::backends::keygen::MockKeyGenBackend;

#[test]
fn test_bootstrap_host_full_workflow() {
    let custom_age_key = "age1v35854ga7r4xl5x8ehwz5tf32dn2z6asrpsrfekke4tsredcwalszjx6np";
    let backend = MockKeyGenBackend::new().with_age_key(custom_age_key);

    let keys = bootstrap_host("heron", &backend).expect("Bootstrap should succeed");

    assert_eq!(keys.hostname, "heron");
    assert_eq!(keys.age_public_key, custom_age_key);

    // Verify keygen was called exactly once for the host
    let keygen_calls = backend.get_keygen_calls();
    assert_eq!(
        keygen_calls.len(),
        1,
        "Should generate keypair exactly once"
    );
    assert!(
        keygen_calls[0].contains("heron"),
        "Should generate keys in heron's directory"
    );

    // Verify age derivation was called with the generated public key
    let age_calls = backend.get_age_calls();
    assert_eq!(age_calls.len(), 1, "Should derive age key exactly once");
    assert!(
        age_calls[0].ends_with("ssh_host_ed25519_key.pub"),
        "Should derive age key from SSH public key"
    );

    // Cleanup
    keys.cleanup().expect("Cleanup should succeed");
}

#[test]
fn test_bootstrap_multiple_hosts() {
    let backend = MockKeyGenBackend::new();

    let keys1 = bootstrap_host("host1", &backend).expect("Bootstrap host1 should succeed");
    let keys2 = bootstrap_host("host2", &backend).expect("Bootstrap host2 should succeed");

    assert_eq!(keys1.hostname, "host1");
    assert_eq!(keys2.hostname, "host2");

    // Paths should be different
    assert_ne!(keys1.ssh_private_key, keys2.ssh_private_key);
    assert_ne!(keys1.temp_dir_root, keys2.temp_dir_root);

    // Backend should have been called twice
    let keygen_calls = backend.get_keygen_calls();
    assert_eq!(keygen_calls.len(), 2);

    // Cleanup both
    keys1.cleanup().expect("Cleanup host1 should succeed");
    keys2.cleanup().expect("Cleanup host2 should succeed");
}

#[test]
fn test_host_keys_contains_all_required_fields() {
    let keys = HostKeys::new(
        "testhost".to_string(),
        std::path::PathBuf::from("/tmp/ssh_host_ed25519_key"),
        std::path::PathBuf::from("/tmp/ssh_host_ed25519_key.pub"),
        "age1v35854ga7r4xl5x8ehwz5tf32dn2z6asrpsrfekke4tsredcwalszjx6np".to_string(),
        std::path::PathBuf::from("/tmp/snow-bootstrap-testhost"),
    );

    // All fields should be accessible
    assert!(!keys.hostname.is_empty());
    assert!(!keys.ssh_private_key.as_os_str().is_empty());
    assert!(!keys.ssh_public_key.as_os_str().is_empty());
    assert!(!keys.age_public_key.is_empty());
    assert!(!keys.temp_dir_root.as_os_str().is_empty());
}

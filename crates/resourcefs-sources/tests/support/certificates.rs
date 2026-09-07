//! Short-lived test identities, issued together so hostname controls share trust.

use std::time::{Duration, SystemTime};

use rcgen::{
    BasicConstraints, CertificateParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose, IsCa,
    Issuer, KeyPair, KeyUsagePurpose,
};

pub struct TestIdentity {
    pub certificate: Vec<u8>,
    pub private_key: Vec<u8>,
}

/// Issues exactly the requested leaves from one fresh CA, valid around this run.
///
/// `CertificateParams::new` encodes IP literals as IP SANs and other names as DNS
/// SANs. Both CA and leaves live for two days, with five minutes of clock skew.
pub fn issue_test_certificates<const N: usize>(
    names: [&[&str]; N],
) -> (Vec<u8>, [TestIdentity; N]) {
    let now = SystemTime::now();
    let not_before = (now - Duration::from_secs(5 * 60)).into();
    let not_after = (now + Duration::from_secs(2 * 24 * 60 * 60)).into();
    let mut ca_params = CertificateParams::default();
    ca_params.not_before = not_before;
    ca_params.not_after = not_after;
    ca_params.distinguished_name = DistinguishedName::new();
    ca_params
        .distinguished_name
        .push(DnType::CommonName, "ResourceFS test CA");
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
    ca_params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    let ca_key = KeyPair::generate().expect("test CA key generation succeeds");
    let ca = ca_params
        .self_signed(&ca_key)
        .expect("test CA self-signing succeeds");
    let issuer = Issuer::from_params(&ca_params, &ca_key);
    let identities = names.map(|names| {
        let mut params = CertificateParams::new(
            names
                .iter()
                .map(|name| (*name).to_owned())
                .collect::<Vec<_>>(),
        )
        .expect("test certificate SANs are valid");
        params.not_before = not_before;
        params.not_after = not_after;
        params.distinguished_name = DistinguishedName::new();
        params
            .distinguished_name
            .push(DnType::CommonName, "ResourceFS test server");
        params.is_ca = IsCa::ExplicitNoCa;
        params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
        params.use_authority_key_identifier_extension = true;
        let key = KeyPair::generate().expect("test server key generation succeeds");
        let certificate = params
            .signed_by(&key, &issuer)
            .expect("test server certificate signing succeeds");
        TestIdentity {
            certificate: certificate.der().to_vec(),
            private_key: key.serialize_der(),
        }
    });
    (ca.der().to_vec(), identities)
}

//! TLS for the Dreame MQTT broker (protocols.md sections 4.2 and 11.1a).
//!
//! The broker's chain ends in a Dreame-private root named `GlobalSign Root CA`,
//! valid from 1969 to 2969. OpenSSL accepts it, but `rustls-webpki` rejects it
//! with `BadEncoding`. [`DreamePinned`] therefore checks the chain itself:
//!
//! - every certificate is valid now and is signed by the next one; the last
//!   one sent (other than the root itself) must be signed by the pinned root;
//! - the host name matches a DNS name in the leaf's SANs;
//! - the TLS 1.3 handshake signature verifies with the leaf's public key.
//!
//! Only TLS 1.3 is accepted. Verification is never switched off.

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::CryptoProvider;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, ServerName, SubjectPublicKeyInfoDer, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};
use std::sync::Arc;
use x509_parser::prelude::*;

/// The Dreame private MQTT root CA. Copied from tasshack/dreame-vacuum
/// (`DREAME_CLIENT_ROOT_CA`, MIT licence).
pub const DREAME_ROOT_CA_PEM: &str = include_str!("dreame_root_ca.pem");

/// Certificate verifier that pins the Dreame private root.
#[derive(Debug)]
pub struct DreamePinned {
    root_der: Vec<u8>,
    provider: Arc<CryptoProvider>,
}

impl DreamePinned {
    /// Pins the root given as PEM.
    pub fn new(root_pem: &str, provider: Arc<CryptoProvider>) -> Result<Self, rustls::Error> {
        let root = CertificateDer::from_pem_slice(root_pem.as_bytes()).map_err(|e| verr(format!("root pem: {e}")))?;
        let root_der = root.as_ref().to_vec();
        X509Certificate::from_der(&root_der).map_err(|e| verr(format!("root: {e}")))?;
        Ok(Self { root_der, provider })
    }
}

/// A client config for the broker: TLS 1.3 only, with [`DreamePinned`].
pub fn client_config() -> Result<rustls::ClientConfig, rustls::Error> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let verifier = DreamePinned::new(DREAME_ROOT_CA_PEM, provider.clone())?;
    Ok(rustls::ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(verifier))
        .with_no_client_auth())
}

fn verr(msg: impl Into<String>) -> rustls::Error {
    rustls::Error::General(msg.into())
}

/// Matches a SAN DNS pattern, with a single leading `*.` wildcard label.
pub(crate) fn host_matches(pattern: &str, host: &str) -> bool {
    match pattern.strip_prefix("*.") {
        Some(suffix) => host.split_once('.').is_some_and(|(label, rest)| !label.is_empty() && rest.eq_ignore_ascii_case(suffix)),
        None => pattern.eq_ignore_ascii_case(host),
    }
}

impl ServerCertVerifier for DreamePinned {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        _ocsp: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let (_, root) = X509Certificate::from_der(&self.root_der).map_err(|e| verr(format!("root: {e}")))?;
        let mut chain = vec![end_entity.as_ref()];
        chain.extend(intermediates.iter().map(|c| c.as_ref()).filter(|c| *c != self.root_der.as_slice()));
        let certs: Vec<X509Certificate> = chain
            .iter()
            .map(|d| X509Certificate::from_der(d).map(|(_, c)| c).map_err(|e| verr(format!("parse: {e}"))))
            .collect::<Result<_, _>>()?;
        let now = ASN1Time::from_timestamp(now.as_secs() as i64).map_err(|e| verr(e.to_string()))?;
        for (i, c) in certs.iter().enumerate() {
            if !c.validity().is_valid_at(now) {
                return Err(verr(format!("certificate {i} is not valid now")));
            }
            let issuer = certs.get(i + 1).unwrap_or(&root);
            c.verify_signature(Some(issuer.public_key()))
                .map_err(|e| verr(format!("certificate {i} signature: {e}")))?;
        }
        let host = match server_name {
            ServerName::DnsName(d) => d.as_ref().to_string(),
            _ => return Err(verr("server name is not a DNS name")),
        };
        let ok = certs[0]
            .subject_alternative_name()
            .ok()
            .flatten()
            .is_some_and(|san| {
                san.value.general_names.iter().any(|g| matches!(g, GeneralName::DNSName(p) if host_matches(p, &host)))
            });
        if !ok {
            return Err(verr("host name not in certificate"));
        }
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Err(verr("TLS 1.2 is not supported"))
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        let (_, c) = X509Certificate::from_der(cert.as_ref()).map_err(|e| verr(format!("parse: {e}")))?;
        let spki = SubjectPublicKeyInfoDer::from(c.public_key().raw);
        rustls::crypto::verify_tls13_signature_with_raw_key(message, &spki, dss, &self.provider.signature_verification_algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider.signature_verification_algorithms.supported_schemes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wildcard_matching() {
        assert!(host_matches("*.mt.us.iot.dreame.tech", "10000.mt.us.iot.dreame.tech"));
        assert!(!host_matches("*.mt.us.iot.dreame.tech", "a.b.mt.us.iot.dreame.tech"));
        assert!(!host_matches("*.mt.us.iot.dreame.tech", "mt.us.iot.dreame.tech"));
        assert!(host_matches("iot.dreame.tech", "IOT.dreame.tech"));
        assert!(!host_matches("iot.dreame.tech", "evil.tech"));
    }

    #[test]
    fn config_builds_with_pinned_root() {
        client_config().unwrap();
    }
}

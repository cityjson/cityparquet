//! Opening the connection, encrypted as its `sslmode` asks, through rustls.

use std::path::Path;
use std::sync::Arc;

use citygml_core::{Error, Result};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{CryptoProvider, verify_tls12_signature, verify_tls13_signature};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{ClientConfig, DigitallySignedStruct, RootCertStore, SignatureScheme};
use tokio_postgres_rustls::MakeRustlsConnect;

use crate::config::{Resolved, SslMode};

/// Connect as `resolved` says.
pub(crate) fn connect(resolved: &Resolved) -> Result<postgres::Client> {
    let connected = match resolved.ssl_mode {
        SslMode::Disable => resolved.postgres.connect(postgres::NoTls),
        mode => {
            let config = client_config(mode, resolved.root_cert.as_deref())?;
            resolved.postgres.connect(MakeRustlsConnect::new(config))
        }
    };
    connected.map_err(|e| Error::invalid(format!("cannot connect to {}: {e}", describe(resolved))))
}

/// Where the connection goes, without its password.
pub(crate) fn describe(resolved: &Resolved) -> String {
    let config = &resolved.postgres;
    let host = match config.get_hosts().first() {
        Some(postgres::config::Host::Tcp(host)) => host.clone(),
        #[cfg(unix)]
        Some(postgres::config::Host::Unix(path)) => path.display().to_string(),
        None => String::new(),
    };
    let port = config.get_ports().first().copied().unwrap_or(5432);
    format!(
        "database {:?} on {host}:{port} as {:?}",
        config.get_dbname().unwrap_or_default(),
        config.get_user().unwrap_or_default()
    )
}

fn client_config(mode: SslMode, root_cert: Option<&Path>) -> Result<ClientConfig> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let builder = ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .map_err(|e| Error::invalid(format!("TLS: {e}")))?;
    let config = if mode == SslMode::VerifyFull {
        builder
            .with_root_certificates(roots(root_cert)?)
            .with_no_client_auth()
    } else {
        // libpq's `prefer` and `require` encrypt without verifying whom
        // they talk to.
        builder
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(Unverified(provider)))
            .with_no_client_auth()
    };
    Ok(config)
}

/// The certificates `sslrootcert` names, else the Mozilla roots.
fn roots(root_cert: Option<&Path>) -> Result<RootCertStore> {
    let mut store = RootCertStore::empty();
    match root_cert {
        Some(path) => {
            let unreadable = |e: rustls::pki_types::pem::Error| {
                Error::invalid(format!("sslrootcert {}: {e}", path.display()))
            };
            for cert in CertificateDer::pem_file_iter(path).map_err(unreadable)? {
                store
                    .add(cert.map_err(unreadable)?)
                    .map_err(|e| Error::invalid(format!("sslrootcert {}: {e}", path.display())))?;
            }
        }
        None => store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned()),
    }
    Ok(store)
}

/// A verifier that accepts any certificate, checking only that the
/// handshake is signed by it.
#[derive(Debug)]
struct Unverified(Arc<CryptoProvider>);

impl ServerCertVerifier for Unverified {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> std::result::Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

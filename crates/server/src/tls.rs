//! TLS on an address the developer names with `--listen` (ADR-0036): through rustls with its
//! defaults, TLS 1.2 and 1.3, with the certificate and key the developer gives, or a self-signed
//! certificate made on the first start for the host name and the address listened on, kept with
//! its key in the user's configuration directory readable by the developer's account alone, and
//! reused on every later start. Its SHA-256 fingerprint is printed at each start.

use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName};
use sha2::{Digest, Sha256};
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio_rustls::TlsAcceptor;

/// The certificate a server speaks TLS with.
pub struct Tls {
    pub acceptor: TlsAcceptor,
    /// The certificate itself, which decides the names a request may carry as its `Host`.
    cert: CertificateDer<'static>,
}

/// The files a self-signed certificate and its key are kept in, in `dir`.
pub const CERT_FILE: &str = "cert.pem";
pub const KEY_FILE: &str = "key.pem";

/// Where a self-signed certificate is kept unless the server is told otherwise: the user's
/// configuration directory.
pub fn default_dir() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("ocelli").join("server"))
}

/// The SHA-256 fingerprint of a certificate in DER, as browsers show it: pairs of uppercase
/// hexadecimal digits between colons.
pub fn fingerprint(der: &[u8]) -> String {
    Sha256::digest(der)
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(":")
}

impl Tls {
    /// TLS with the certificate chain and key in the PEM files `cert` and `key`.
    pub fn from_files(cert: &Path, key: &Path) -> Result<Tls, String> {
        let chain = CertificateDer::pem_file_iter(cert)
            .and_then(|certs| certs.collect::<Result<Vec<_>, _>>())
            .map_err(|e| format!("The certificate {} cannot be read: {e}", cert.display()))?;
        let key = PrivateKeyDer::from_pem_file(key)
            .map_err(|e| format!("The key {} cannot be read: {e}", key.display()))?;
        Tls::new(chain, key)
    }

    /// TLS with the self-signed certificate kept in `dir`, made for this machine's host name and
    /// `address` where none is kept yet. One kept that does not name `address` is refused rather
    /// than replaced: the developer removes it, or gives a certificate of their own.
    ///
    /// The names checked are those it is reached by: the address where it is a specific one, and
    /// the host name where it is unspecified, `0.0.0.0` or `::`, which names no address a browser
    /// reaches. A pair kept in part, or a key other accounts may read, is refused with what to do.
    pub fn self_signed(dir: &Path, address: IpAddr) -> Result<Tls, String> {
        let (cert, key) = (dir.join(CERT_FILE), dir.join(KEY_FILE));
        let remove = format!(
            "remove {CERT_FILE} and {KEY_FILE} in {} to make a new certificate, or give --cert \
             and --key",
            dir.display()
        );
        match (cert.exists(), key.exists()) {
            (false, false) => make(dir, address)?,
            (true, true) => {}
            (kept, _) => {
                let (lone, missing) = if kept {
                    (CERT_FILE, KEY_FILE)
                } else {
                    (KEY_FILE, CERT_FILE)
                };
                return Err(format!(
                    "{lone} is kept in {} without its {missing}: {remove}",
                    dir.display()
                ));
            }
        }
        private_key(&key, &remove)?;
        let tls = Tls::from_files(&cert, &key)?;
        for name in reached_by(address) {
            if !tls.names(&name) {
                return Err(format!(
                    "The certificate kept in {} is not for {name}: {remove}",
                    dir.display()
                ));
            }
        }
        Ok(tls)
    }

    fn new(
        chain: Vec<CertificateDer<'static>>,
        key: PrivateKeyDer<'static>,
    ) -> Result<Tls, String> {
        let cert = chain
            .first()
            .cloned()
            .ok_or("The certificate file holds no certificate")?;
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let config = rustls::ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .and_then(|b| b.with_no_client_auth().with_single_cert(chain, key))
            .map_err(|e| format!("The certificate and key cannot be used: {e}"))?;
        Ok(Tls {
            acceptor: TlsAcceptor::from(Arc::new(config)),
            cert,
        })
    }

    /// The certificate's SHA-256 fingerprint.
    pub fn fingerprint(&self) -> String {
        fingerprint(&self.cert)
    }

    /// Whether the certificate is valid for `name`, the name part of a request's `Host`: an
    /// address, an IPv6 one in brackets, or a DNS name. A name the certificate does not carry is
    /// what a page reaching the server through DNS rebinding sends (ADR-0036).
    pub fn names(&self, name: &str) -> bool {
        let name = name
            .strip_prefix('[')
            .and_then(|n| n.strip_suffix(']'))
            .unwrap_or(name);
        let Ok(server_name) = ServerName::try_from(name.to_string()) else {
            return false;
        };
        webpki::EndEntityCert::try_from(&self.cert)
            .is_ok_and(|cert| cert.verify_is_valid_for_subject_name(&server_name).is_ok())
    }
}

/// This machine's host name, where it is a DNS name a certificate can carry.
pub fn host_name() -> Option<String> {
    let host = gethostname::gethostname().to_str()?.to_string();
    ServerName::try_from(host.clone())
        .is_ok_and(|n| matches!(n, ServerName::DnsName(_)))
        .then_some(host)
}

/// The names a server listening on `address` is reached by: the address itself where it is a
/// specific one, the host name where it is unspecified.
fn reached_by(address: IpAddr) -> Vec<String> {
    if address.is_unspecified() {
        host_name().into_iter().collect()
    } else {
        vec![address.to_string()]
    }
}

/// That the kept key `key` is readable by the developer's account alone.
fn private_key(key: &Path, remove: &str) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(key)
            .map_err(|e| format!("The key {} cannot be read: {e}", key.display()))?
            .permissions()
            .mode();
        if mode & 0o077 != 0 {
            return Err(format!(
                "The key {} may be read by other accounts: {remove}",
                key.display()
            ));
        }
    }
    // On Windows the user's configuration directory inherits an access list that admits the
    // account, administrators, and the system alone.
    #[cfg(not(unix))]
    let _ = (key, remove);
    Ok(())
}

/// Makes a self-signed certificate for this machine's host name, where it is a DNS name, and
/// `address`, unless it is the unspecified one, and writes it and its key into `dir`, readable by
/// the developer's account alone.
fn make(dir: &Path, address: IpAddr) -> Result<(), String> {
    let mut names: Vec<String> = host_name().into_iter().collect();
    if !address.is_unspecified() {
        names.push(address.to_string());
    }
    if names.is_empty() {
        return Err(
            "No host name or address to make a certificate for: give --cert and --key".into(),
        );
    }
    let made = rcgen::generate_simple_self_signed(names)
        .map_err(|e| format!("A self-signed certificate cannot be made: {e}"))?;
    private_dir(dir)?;
    // The key first: a certificate without its key is never left behind alone.
    private_file(
        &dir.join(KEY_FILE),
        made.signing_key.serialize_pem().as_bytes(),
    )?;
    private_file(&dir.join(CERT_FILE), made.cert.pem().as_bytes())
}

/// Creates `dir` where it is missing, and leaves it to the developer's account alone.
fn private_dir(dir: &Path) -> Result<(), String> {
    let cannot = |e: std::io::Error| format!("{} cannot be made: {e}", dir.display());
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        builder.mode(0o700);
        builder.create(dir).map_err(cannot)?;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).map_err(cannot)
    }
    // On Windows the user's configuration directory inherits an access list that admits the
    // account, administrators, and the system alone.
    #[cfg(not(unix))]
    builder.create(dir).map_err(cannot)
}

/// Writes a new file at `path`, readable and writable by the developer's account alone; one that
/// exists is not overwritten.
fn private_file(path: &Path, content: &[u8]) -> Result<(), String> {
    use std::io::Write as _;
    let cannot = |e: std::io::Error| format!("{} cannot be written: {e}", path.display());
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path).map_err(cannot)?;
    file.write_all(content).map_err(cannot)?;
    file.sync_all().map_err(cannot)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    // Verifies: ADR-0036
    fn the_fingerprint_is_the_sha_256_of_the_certificate_as_browsers_show_it() {
        let print = fingerprint(b"abc");
        assert_eq!(
            print,
            "BA:78:16:BF:8F:01:CF:EA:41:41:40:DE:5D:AE:22:23:B0:03:61:A3:96:17:7A:9C:B4:10:FF:61:F2:00:15:AD"
        );
    }
}

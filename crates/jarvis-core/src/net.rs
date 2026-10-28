// Réseau : client HTTP partagé et connexions TLS brutes (IMAP / SMTP de Gmail).

use std::sync::{Arc, OnceLock};

use tokio::net::TcpStream;
use tokio_rustls::client::TlsStream;
use tokio_rustls::rustls::{self, pki_types::ServerName};
use tokio_rustls::TlsConnector;

static HTTP: OnceLock<reqwest::Client> = OnceLock::new();

/// Client HTTP partagé (fetch de la v3).
pub fn http() -> reqwest::Client {
    HTTP.get_or_init(|| reqwest::Client::builder().user_agent("Jarvis/4.0").build().unwrap_or_default()).clone()
}

/// Connexion TLS vérifiée (SNI compris).
pub async fn tls_connect(host: &str, port: u16) -> Result<TlsStream<TcpStream>, String> {
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let config = rustls::ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .with_safe_default_protocol_versions()
        .map_err(|e| e.to_string())?
        .with_root_certificates(roots)
        .with_no_client_auth();
    let connector = TlsConnector::from(Arc::new(config));
    let tcp = TcpStream::connect((host, port)).await.map_err(|e| e.to_string())?;
    let name = ServerName::try_from(host.to_string()).map_err(|e| e.to_string())?;
    connector.connect(name, tcp).await.map_err(|e| e.to_string())
}

use rtc::dtls::crypto::Certificate as DtlsCertificate;
use rtc::peer_connection::certificate::RTCCertificate;
use rtc::shared::error::Error as RtcError;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::mpsc;

#[derive(Clone, Debug)]
pub struct GeneratedCert {
    pub certificate: RTCCertificate,
    pub dtls_certificate: DtlsCertificate,
    pub fingerprint_sha256: String,
}

pub struct CertPool {
    ch: Option<mpsc::Receiver<GeneratedCert>>,
    closed: Arc<AtomicBool>,
}

impl CertPool {
    pub fn new(size: usize) -> Self {
        if size == 0 {
            return Self {
                ch: None,
                closed: Arc::new(AtomicBool::new(false)),
            };
        }

        let (tx, rx) = mpsc::channel(size);
        let closed = Arc::new(AtomicBool::new(false));
        let closed_worker = closed.clone();

        tokio::spawn(async move {
            while !closed_worker.load(Ordering::SeqCst) {
                let cert_res = tokio::task::spawn_blocking(generate_cert).await;
                match cert_res {
                    Ok(Ok(cert)) => {
                        if tx.send(cert).await.is_err() {
                            break;
                        }
                    }
                    _ => {
                        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                    }
                }
            }
        });

        Self {
            ch: Some(rx),
            closed,
        }
    }

    pub fn try_take(&mut self) -> Option<GeneratedCert> {
        if self.closed.load(Ordering::SeqCst) {
            return None;
        }
        if let Some(ref mut rx) = self.ch
            && let Ok(cert) = rx.try_recv()
        {
            return Some(cert);
        }
        None
    }

    pub async fn take(&mut self) -> Result<GeneratedCert, RtcError> {
        if let Some(cert) = self.try_take() {
            return Ok(cert);
        }

        tokio::task::spawn_blocking(generate_cert)
            .await
            .map_err(|e| RtcError::Other(e.to_string()))?
    }

    pub fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
    }
}

pub fn generate_cert() -> Result<GeneratedCert, RtcError> {
    let key_pair = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256)
        .map_err(|e| RtcError::Other(e.to_string()))?;
    let certificate = RTCCertificate::from_key_pair(key_pair)?;
    let dtls_certificate = certificate.dtls_certificate.clone();
    let fp = certificate
        .get_fingerprints()
        .first()
        .map(|f| f.value.to_uppercase())
        .unwrap_or_default();

    Ok(GeneratedCert {
        certificate,
        dtls_certificate,
        fingerprint_sha256: fp,
    })
}

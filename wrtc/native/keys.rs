//! Keys handling and DTLS-SRTP key derivation constants.

use crate::models::errors::RustTgCallsError;

/// dtlsSrtpExporterLabel is the RFC 5764 label used to derive SRTP keys
/// from the DTLS-SRTP master secret.
pub const DTLS_SRTP_EXPORTER_LABEL: &str = "EXTRACTOR-dtls_srtp";

/// ProtectionProfile represents protection profile constants matching IANA registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtectionProfile {
    AeadAes128Gcm,
    Aes128CmHmacSha1_80,
}

impl ProtectionProfile {
    pub fn key_len(&self) -> usize {
        16
    }

    pub fn salt_len(&self) -> usize {
        match self {
            ProtectionProfile::AeadAes128Gcm => 12,
            ProtectionProfile::Aes128CmHmacSha1_80 => 14,
        }
    }
}

/// derive_srtp_context extracts SRTP master key + salt from a completed
/// DTLS handshake (per RFC 5764) and constructs a send-only SRTP Context.
/// We are the DTLS server (setup=passive on the JoinGroupCall payload),
/// so the server-side half of the keying material is what feeds OUR
/// encryption.
///
/// Layout per RFC 5764 §4.2: client_key || server_key || client_salt || server_salt.
/// We are SERVER -> use server_key + server_salt for encryption.
pub fn derive_srtp_keying_material(
    material: &[u8],
    profile: ProtectionProfile,
) -> Result<([u8; 16], Vec<u8>), RustTgCallsError> {
    let (server_key, server_salt, _, _) = derive_srtp_keying_material_both(material, profile)?;
    Ok((server_key, server_salt))
}

pub fn derive_srtp_keying_material_both(
    material: &[u8],
    profile: ProtectionProfile,
) -> Result<([u8; 16], Vec<u8>, [u8; 16], Vec<u8>), RustTgCallsError> {
    let key_len = profile.key_len();
    let salt_len = profile.salt_len();
    let total_len = 2 * (key_len + salt_len);

    if material.len() < total_len {
        return Err(RustTgCallsError::InvalidParams(
            "keying material buffer too small".into(),
        ));
    }

    let mut off = 0;
    let client_key_slice = &material[off..off + key_len];
    off += key_len;
    let server_key_slice = &material[off..off + key_len];
    off += key_len;
    let client_salt = material[off..off + salt_len].to_vec();
    off += salt_len;
    let server_salt = material[off..off + salt_len].to_vec();

    let mut client_key = [0u8; 16];
    client_key.copy_from_slice(client_key_slice);

    let mut server_key = [0u8; 16];
    server_key.copy_from_slice(server_key_slice);

    Ok((server_key, server_salt, client_key, client_salt))
}

/// translate_profile maps the IANA-assigned DTLS-SRTP profile constant to
/// ProtectionProfile type.
pub fn translate_profile(iana_val: u16) -> Result<ProtectionProfile, RustTgCallsError> {
    match iana_val {
        0x0007 => Ok(ProtectionProfile::AeadAes128Gcm),
        0x0001 => Ok(ProtectionProfile::Aes128CmHmacSha1_80),
        _ => Err(RustTgCallsError::InvalidParams(format!(
            "unsupported SRTP profile value 0x{:04x}",
            iana_val
        ))),
    }
}

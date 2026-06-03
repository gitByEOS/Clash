use aes::cipher::{block_padding::Pkcs7, BlockDecryptMut, BlockEncryptMut, KeyIvInit};
use base64::{engine::general_purpose::STANDARD, Engine};
use pbkdf2::pbkdf2_hmac;
use rand::RngCore;
use sha2::Sha256;
use std::env;


type Aes256CbcEnc = cbc::Encryptor<aes::Aes256>;
type Aes256CbcDec = cbc::Decryptor<aes::Aes256>;

const SALT_MAGIC: &[u8] = b"Salted__";
const PBKDF2_ITERATIONS: u32 = 10_000;
const KEY_LEN: usize = 32;
const IV_LEN: usize = 16;
const SALT_LEN: usize = 8;

/// Derive the password used for encryption: "clash:{hostname}:{username}"
pub fn derive_password() -> String {
    let hostname = env::var("HOSTNAME")
        .ok()
        .or_else(|| {
            // Fallback: try to get hostname via command
            std::process::Command::new("hostname")
                .output()
                .ok()
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .map(|s| s.trim().to_string())
        })
        .unwrap_or_else(|| "localhost".to_string());

    let username = env::var("USER")
        .or_else(|_| env::var("LOGNAME"))
        .ok()
        .or_else(|| {
            std::process::Command::new("whoami")
                .output()
                .ok()
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .map(|s| s.trim().to_string())
        })
        .unwrap_or_else(|| "unknown".to_string());

    format!("clash:{}:{}", hostname, username)
}

/// Derive AES key and IV from password + salt using PBKDF2-HMAC-SHA256
fn derive_key_iv(password: &[u8], salt: &[u8]) -> ([u8; KEY_LEN], [u8; IV_LEN]) {
    let mut derived = [0u8; KEY_LEN + IV_LEN];
    pbkdf2_hmac::<Sha256>(password, salt, PBKDF2_ITERATIONS, &mut derived);
    let mut key = [0u8; KEY_LEN];
    let mut iv = [0u8; IV_LEN];
    key.copy_from_slice(&derived[..KEY_LEN]);
    iv.copy_from_slice(&derived[KEY_LEN..]);
    (key, iv)
}

#[derive(Debug)]
pub enum CryptoError {
    Base64Decode,
    InvalidFormat,
    DecryptionFailed,
    EncryptionFailed,
}

impl std::fmt::Display for CryptoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CryptoError::Base64Decode => write!(f, "Base64 解码失败"),
            CryptoError::InvalidFormat => write!(f, "无效的加密格式"),
            CryptoError::DecryptionFailed => write!(f, "解密失败"),
            CryptoError::EncryptionFailed => write!(f, "加密失败"),
        }
    }
}

/// Encrypt a token using AES-256-CBC-PBKDF2 (compatible with `openssl enc -aes-256-cbc -pbkdf2`)
pub fn encrypt_token(plaintext: &str) -> Result<String, CryptoError> {
    let password = derive_password();
    let mut salt = [0u8; SALT_LEN];
    rand::thread_rng().fill_bytes(&mut salt);

    let (key, iv) = derive_key_iv(password.as_bytes(), &salt);

    let plain_bytes = plaintext.as_bytes();
    // Output buffer: padded length (round up to 16-byte boundary, always adds at least 1 byte)
    let pad_len = 16 - (plain_bytes.len() % 16);
    let mut out_buf = vec![0u8; plain_bytes.len() + pad_len];

    // Encrypt (padding applied internally)
    let cipher = Aes256CbcEnc::new(&key.into(), &iv.into());
    let ciphertext = cipher
        .encrypt_padded_b2b_mut::<Pkcs7>(plain_bytes, &mut out_buf)
        .map_err(|_| CryptoError::EncryptionFailed)?;
    let ciphertext = ciphertext.to_vec();

    // Build output: Salted__ + salt + ciphertext
    let mut output = Vec::with_capacity(SALT_MAGIC.len() + SALT_LEN + ciphertext.len());
    output.extend_from_slice(SALT_MAGIC);
    output.extend_from_slice(&salt);
    output.extend_from_slice(&ciphertext);

    // Base64 encode (no line wrapping = -A flag)
    Ok(STANDARD.encode(&output))
}

/// Decrypt a token encrypted with `openssl enc -aes-256-cbc -base64 -A -pass pass:... -pbkdf2`
pub fn decrypt_token(encoded: &str) -> Result<String, CryptoError> {
    let data = STANDARD.decode(encoded).map_err(|_| CryptoError::Base64Decode)?;

    // Verify "Salted__" magic
    if data.len() < SALT_MAGIC.len() + SALT_LEN || &data[..SALT_MAGIC.len()] != SALT_MAGIC {
        return Err(CryptoError::InvalidFormat);
    }

    let salt = &data[SALT_MAGIC.len()..SALT_MAGIC.len() + SALT_LEN];
    let ciphertext = &data[SALT_MAGIC.len() + SALT_LEN..];

    let password = derive_password();
    let (key, iv) = derive_key_iv(password.as_bytes(), salt);

    // Decrypt (padding removed internally)
    let cipher = Aes256CbcDec::new(&key.into(), &iv.into());
    let mut out_buf = vec![0u8; ciphertext.len()];
    let padded = cipher
        .decrypt_padded_b2b_mut::<Pkcs7>(ciphertext, &mut out_buf)
        .map_err(|_| CryptoError::DecryptionFailed)?;

    String::from_utf8(padded.to_vec()).map_err(|_| CryptoError::DecryptionFailed)
}

/// Check if a value looks like a base64-encoded encrypted token
#[allow(dead_code)]
pub fn looks_encrypted(value: &str) -> bool {
    // Matches: ^[A-Za-z0-9+/=]{16,}$
    if value.len() < 16 {
        return false;
    }
    value
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '/' || c == '=')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encrypt_decrypt_roundtrip() {
        let token = "sk-test-api-key-12345";
        let encrypted = encrypt_token(token).unwrap();
        let decrypted = decrypt_token(&encrypted).unwrap();
        assert_eq!(decrypted, token);
    }

    #[test]
    fn test_different_salt_different_output() {
        let token = "sk-test-api-key-12345";
        let enc1 = encrypt_token(token).unwrap();
        let enc2 = encrypt_token(token).unwrap();
        assert_ne!(enc1, enc2);
        assert_eq!(decrypt_token(&enc1).unwrap(), token);
        assert_eq!(decrypt_token(&enc2).unwrap(), token);
    }

    #[test]
    fn test_decrypt_empty_string_fails() {
        assert!(decrypt_token("").is_err());
    }

    #[test]
    fn test_derive_password_format() {
        let pwd = derive_password();
        assert!(pwd.starts_with("clash:"));
        assert!(pwd.matches(':').count() >= 2);
    }
}

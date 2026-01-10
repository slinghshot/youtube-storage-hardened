use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use argon2::Argon2;
use rand::Rng;

// Salt size for Argon2 key derivation
const SALT_SIZE: usize = 16;
// Nonce size for AES-256-GCM
const NONCE_SIZE: usize = 12;

/// Derives a 256-bit key from a password using Argon2id
fn derive_key(password: &str, salt: &[u8]) -> Result<[u8; 32], Box<dyn std::error::Error>> {
    let mut key = [0u8; 32];
    Argon2::default()
        .hash_password_into(password.as_bytes(), salt, &mut key)
        .map_err(|e| format!("Key derivation failed: {}", e))?;
    Ok(key)
}

/// Encrypts data using AES-256-GCM with a password
/// Format: [salt (16 bytes)][nonce (12 bytes)][ciphertext]
pub fn encrypt(data: &[u8], password: &str) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    // Generate random salt and nonce
    let mut rng = rand::rng();
    let salt: [u8; SALT_SIZE] = rng.random();
    let nonce_bytes: [u8; NONCE_SIZE] = rng.random();

    // Derive key from password
    let key = derive_key(password, &salt)?;

    // Create cipher and encrypt
    let cipher = Aes256Gcm::new_from_slice(&key)
        .map_err(|e| format!("Failed to create cipher: {}", e))?;
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher
        .encrypt(nonce, data)
        .map_err(|e| format!("Encryption failed: {}", e))?;

    // Combine salt + nonce + ciphertext
    let mut result = Vec::with_capacity(SALT_SIZE + NONCE_SIZE + ciphertext.len());
    result.extend_from_slice(&salt);
    result.extend_from_slice(&nonce_bytes);
    result.extend_from_slice(&ciphertext);

    Ok(result)
}

/// Decrypts data using AES-256-GCM with a password
/// Expects format: [salt (16 bytes)][nonce (12 bytes)][ciphertext]
pub fn decrypt(encrypted_data: &[u8], password: &str) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    if encrypted_data.len() < SALT_SIZE + NONCE_SIZE {
        return Err("Encrypted data too short".into());
    }

    // Extract salt, nonce, and ciphertext
    let salt = &encrypted_data[..SALT_SIZE];
    let nonce_bytes = &encrypted_data[SALT_SIZE..SALT_SIZE + NONCE_SIZE];
    let ciphertext = &encrypted_data[SALT_SIZE + NONCE_SIZE..];

    // Derive key from password
    let key = derive_key(password, salt)?;

    // Create cipher and decrypt
    let cipher = Aes256Gcm::new_from_slice(&key)
        .map_err(|e| format!("Failed to create cipher: {}", e))?;
    let nonce = Nonce::from_slice(nonce_bytes);

    let plaintext = cipher
        .decrypt(nonce, ciphertext)
        .map_err(|_| "Decryption failed: incorrect password or corrupted data")?;

    Ok(plaintext)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encrypt_decrypt() {
        let data = b"Hello, World!";
        let password = "test_password";

        let encrypted = encrypt(data, password).unwrap();
        let decrypted = decrypt(&encrypted, password).unwrap();

        assert_eq!(data.to_vec(), decrypted);
    }

    #[test]
    fn test_wrong_password() {
        let data = b"Hello, World!";
        let password = "test_password";
        let wrong_password = "wrong_password";

        let encrypted = encrypt(data, password).unwrap();
        let result = decrypt(&encrypted, wrong_password);

        assert!(result.is_err());
    }
}

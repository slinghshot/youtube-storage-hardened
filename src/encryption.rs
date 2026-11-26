use crate::Result;
use aes_gcm::{
    aead::{Aead, AeadInPlace, KeyInit},
    Aes256Gcm, Nonce, Tag,
};
use argon2::Argon2;
use rand::{rngs::OsRng, RngCore};
use std::io::{Read, Write};
use zeroize::Zeroizing;

pub const MAGIC: &[u8; 8] = b"YTSTORE2";
pub const V1_MAGIC: &[u8; 8] = b"YTSTORE1";
const SALT_SIZE: usize = 16;
const TAG_SIZE: usize = 16;
const CHUNK_SIZE: u64 = 1 << 20;
const HEADER_SIZE: u64 = MAGIC.len() as u64 + SALT_SIZE as u64;

fn cipher(password: &str, salt: &[u8]) -> Result<Aes256Gcm> {
    let mut key = Zeroizing::new([0u8; 32]);
    Argon2::default()
        .hash_password_into(password.as_bytes(), salt, &mut *key)
        .map_err(|error| format!("Key derivation failed: {error}"))?;
    Aes256Gcm::new_from_slice(&*key).map_err(|_| "Invalid encryption key".into())
}

// Each archive uses a fresh key derived from its random salt. Within that key,
// the nonce identifies the chunk, and the AAD commits to the total payload size.
fn chunk_nonce(index: u64) -> [u8; 12] {
    let mut nonce = [0; 12];
    nonce[4..].copy_from_slice(&index.to_le_bytes());
    nonce
}

fn associated_data(size: u64) -> [u8; 16] {
    let mut aad = [0; 16];
    aad[..8].copy_from_slice(MAGIC);
    aad[8..].copy_from_slice(&size.to_le_bytes());
    aad
}

pub fn encrypted_size(zip_size: u64) -> Result<u64> {
    zip_size
        .checked_add(zip_size.div_ceil(CHUNK_SIZE).max(1) * TAG_SIZE as u64)
        .and_then(|size| size.checked_add(HEADER_SIZE))
        .ok_or_else(|| "Archive is too large for the video header".into())
}

pub fn encrypt(
    mut zip: impl Read,
    zip_size: u64,
    password: &str,
    mut output: impl Write,
) -> Result<()> {
    let aad = associated_data(encrypted_size(zip_size)?);
    let mut salt = [0; SALT_SIZE];
    OsRng.try_fill_bytes(&mut salt)?;
    let cipher = cipher(password, &salt)?;
    output.write_all(MAGIC)?;
    output.write_all(&salt)?;
    let mut buffer = Zeroizing::new(vec![0; CHUNK_SIZE as usize]);
    let mut remaining = zip_size;
    for index in 0..zip_size.div_ceil(CHUNK_SIZE).max(1) {
        let chunk = &mut buffer[..remaining.min(CHUNK_SIZE) as usize];
        zip.read_exact(chunk)?;
        let tag = cipher
            .encrypt_in_place_detached(&chunk_nonce(index).into(), &aad, chunk)
            .map_err(|_| "Encryption failed")?;
        output.write_all(chunk)?;
        output.write_all(&tag)?;
        remaining -= chunk.len() as u64;
    }
    Ok(())
}

/// The caller has consumed the magic. Size includes the magic and salt.
pub fn decrypt(mut input: impl Read, size: u64, password: &str, mut zip: impl Write) -> Result<()> {
    let aad = associated_data(size);
    let mut remaining = size
        .checked_sub(HEADER_SIZE)
        .ok_or("Invalid encrypted length")?;
    if remaining < TAG_SIZE as u64 {
        return Err("Missing authentication tag".into());
    }
    let mut salt = [0; SALT_SIZE];
    input.read_exact(&mut salt)?;
    let cipher = cipher(password, &salt)?;
    let mut buffer = Zeroizing::new(vec![0; CHUNK_SIZE as usize]);
    let mut index = 0;
    loop {
        let record_size = remaining.min(CHUNK_SIZE + TAG_SIZE as u64);
        let chunk_size = record_size
            .checked_sub(TAG_SIZE as u64)
            .ok_or("Incomplete encrypted record")?;
        let chunk = &mut buffer[..chunk_size as usize];
        let mut tag = Tag::default();
        input.read_exact(chunk)?;
        input.read_exact(&mut tag)?;
        cipher
            .decrypt_in_place_detached(&chunk_nonce(index).into(), &aad, chunk, &tag)
            .map_err(|_| "Decryption failed: incorrect password or corrupted data")?;
        zip.write_all(chunk)?;
        remaining -= record_size;
        if remaining == 0 {
            return Ok(());
        }
        index += 1;
    }
}

/// The caller has consumed the v1 magic and bounded the ciphertext length.
pub fn decrypt_v1(data: &[u8], password: &str) -> Result<Zeroizing<Vec<u8>>> {
    if data.len() < SALT_SIZE + 12 + TAG_SIZE {
        return Err("Encrypted data too short".into());
    }
    let cipher = cipher(password, &data[..SALT_SIZE])?;
    cipher
        .decrypt(
            Nonce::from_slice(&data[SALT_SIZE..SALT_SIZE + 12]),
            &data[SALT_SIZE + 12..],
        )
        .map(Zeroizing::new)
        .map_err(|_| "Decryption failed: incorrect password or corrupted data".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "65 MiB streaming regression, run with --release -- --ignored"]
    fn streams_larger_than_the_previous_payload_limit() -> Result<()> {
        use std::io::{Seek, SeekFrom};

        let size = 65 << 20;
        let mut encrypted = tempfile::tempfile()?;
        encrypt(
            std::io::repeat(42).take(size),
            size,
            "test-only",
            &mut encrypted,
        )?;
        let payload_size = encrypted_size(size)?;
        assert_eq!(encrypted.metadata()?.len(), payload_size);
        encrypted.seek(SeekFrom::Start(MAGIC.len() as u64))?;
        decrypt(encrypted, payload_size, "test-only", std::io::sink())
    }

    #[test]
    fn authenticates_multi_chunk_stream_and_total_size() -> Result<()> {
        let data = vec![42; CHUNK_SIZE as usize + 17];
        let mut ciphertext = Vec::new();
        let size = encrypted_size(data.len() as u64)?;
        encrypt(&data[..], data.len() as u64, "test-only", &mut ciphertext)?;
        assert_eq!(size, ciphertext.len() as u64);
        let mut recovered = Vec::new();
        let body = &ciphertext[MAGIC.len()..];
        decrypt(body, size, "test-only", &mut recovered)?;
        assert_eq!(recovered, data);
        assert!(decrypt(body, size, "wrong", std::io::sink()).is_err());
        assert!(decrypt(&body[..body.len() - 1], size, "test-only", std::io::sink()).is_err());
        assert!(decrypt(body, size - 1, "test-only", std::io::sink()).is_err());
        *ciphertext.last_mut().unwrap() ^= 1;
        assert!(decrypt(
            &ciphertext[MAGIC.len()..],
            size,
            "test-only",
            std::io::sink()
        )
        .is_err());
        Ok(())
    }
}

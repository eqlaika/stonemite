use base64::{engine::general_purpose::STANDARD, Engine};
use windows::Win32::Security::Cryptography::{
    CryptProtectData, CryptUnprotectData, CRYPT_INTEGER_BLOB,
};

/// Encrypt a plaintext string using Windows DPAPI (user-scoped).
/// Returns a base64-encoded ciphertext.
pub fn encrypt(plaintext: &str) -> Result<String, String> {
    let data = plaintext.as_bytes();
    let input = CRYPT_INTEGER_BLOB {
        cbData: data.len() as u32,
        pbData: data.as_ptr() as *mut u8,
    };
    let mut output = CRYPT_INTEGER_BLOB::default();

    unsafe {
        CryptProtectData(&input, None, None, None, None, 0, &mut output)
            .map_err(|e| format!("CryptProtectData failed: {e}"))?;

        let encrypted =
            std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
        Ok(STANDARD.encode(&encrypted))
    }
}

/// Decrypt a base64-encoded DPAPI ciphertext back to plaintext.
pub fn decrypt(base64_ciphertext: &str) -> Result<String, String> {
    let encrypted = STANDARD
        .decode(base64_ciphertext)
        .map_err(|e| format!("base64 decode failed: {e}"))?;

    let input = CRYPT_INTEGER_BLOB {
        cbData: encrypted.len() as u32,
        pbData: encrypted.as_ptr() as *mut u8,
    };
    let mut output = CRYPT_INTEGER_BLOB::default();

    unsafe {
        CryptUnprotectData(&input, None, None, None, None, 0, &mut output)
            .map_err(|e| format!("CryptUnprotectData failed: {e}"))?;

        let decrypted =
            std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
        String::from_utf8(decrypted).map_err(|e| format!("UTF-8 decode failed: {e}"))
    }
}

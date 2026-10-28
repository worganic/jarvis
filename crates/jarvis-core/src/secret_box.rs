// Chiffrement des secrets des connecteurs par Windows (DPAPI, portée « utilisateur courant »),
// core/secret-box.js de la v3. La v3 passait par PowerShell (≈ 0,3 s) ; ici l'API est appelée
// directement. Mêmes données : un secret chiffré par la v3 se lit tel quel. Les valeurs déchiffrées
// sont gardées en mémoire pour la session.

use std::collections::HashMap;
use std::sync::Mutex;

use base64::Engine;
use windows::Win32::Foundation::{LocalFree, HLOCAL};
use windows::Win32::Security::Cryptography::{CryptProtectData, CryptUnprotectData, CRYPT_INTEGER_BLOB};

pub struct SecretBox {
    cache: Mutex<HashMap<String, String>>,
}

fn blob(data: &[u8]) -> CRYPT_INTEGER_BLOB {
    CRYPT_INTEGER_BLOB { cbData: data.len() as u32, pbData: data.as_ptr() as *mut u8 }
}

fn take(out: CRYPT_INTEGER_BLOB) -> Vec<u8> {
    unsafe {
        let v = std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec();
        let _ = LocalFree(Some(HLOCAL(out.pbData as *mut _)));
        v
    }
}

impl SecretBox {
    pub fn new() -> Self {
        Self { cache: Mutex::new(HashMap::new()) }
    }

    pub fn available(&self) -> bool {
        true
    }

    pub fn encrypt(&self, text: &str) -> Result<String, String> {
        let input = blob(text.as_bytes());
        let mut out = CRYPT_INTEGER_BLOB::default();
        unsafe { CryptProtectData(&input, None, None, None, None, 0, &mut out) }.map_err(|e| e.message().to_string())?;
        let enc = base64::engine::general_purpose::STANDARD.encode(take(out));
        self.cache.lock().unwrap().insert(enc.clone(), text.to_string());
        Ok(enc)
    }

    pub fn decrypt(&self, b64: &str) -> Result<String, String> {
        if let Some(c) = self.cache.lock().unwrap().get(b64) {
            return Ok(c.clone());
        }
        let bytes = base64::engine::general_purpose::STANDARD.decode(b64.trim()).map_err(|e| e.to_string())?;
        let input = blob(&bytes);
        let mut out = CRYPT_INTEGER_BLOB::default();
        unsafe { CryptUnprotectData(&input, None, None, None, None, 0, &mut out) }.map_err(|e| e.message().to_string())?;
        let clear = String::from_utf8_lossy(&take(out)).into_owned();
        self.cache.lock().unwrap().insert(b64.to_string(), clear.clone());
        Ok(clear)
    }
}

impl Default for SecretBox {
    fn default() -> Self {
        Self::new()
    }
}

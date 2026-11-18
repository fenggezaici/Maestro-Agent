//! Provider credentials at rest. The plaintext key never lives in the webview
//! (localStorage/JS heap): the frontend only sends it once, the backend stores
//! it protected for the current OS user and resolves it when starting a run.
//!
//! Windows: DPAPI (`CryptProtectData`) binds the blob to the current user, so a
//! copied file cannot be decrypted by another account or machine. Other
//! platforms are development-only and fall back to a 0600 file — the shipped
//! desktop target is Windows.
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use crate::domain::AppResult;

const MAX_KEY_CHARS: usize = 8192;
const MAX_PROVIDER_CHARS: usize = 64;

/// Protected key/value store for provider API keys, keyed by provider id.
pub struct CredentialStore {
    path: PathBuf,
}

/// Why the store could not be read. `Corrupt` is recoverable by quarantining
/// the blob; `Io` is a real filesystem problem and must surface unchanged.
enum LoadError {
    Missing,
    Corrupt,
    Io,
}

impl CredentialStore {
    pub fn open(dir: &Path) -> AppResult<Self> {
        std::fs::create_dir_all(dir).map_err(|_| "无法创建凭据目录".to_string())?;
        Ok(Self { path: dir.join("credentials.bin") })
    }

    pub fn save(&self, provider: &str, api_key: &str) -> AppResult<()> {
        let provider = validate_provider(provider)?;
        let key = api_key.trim();
        if key.is_empty() || key.chars().count() > MAX_KEY_CHARS || key.chars().any(char::is_control) {
            return Err("API Key 无效".into());
        }
        let mut entries = match self.read_entries() {
            Ok(entries) => entries,
            Err(LoadError::Missing) => BTreeMap::new(),
            Err(LoadError::Corrupt) => {
                self.quarantine();
                BTreeMap::new()
            }
            Err(LoadError::Io) => return Err("无法读取凭据文件".into()),
        };
        entries.insert(provider, key.to_string());
        self.store(&entries)
    }

    pub fn get(&self, provider: &str) -> AppResult<Option<String>> {
        let provider = validate_provider(provider)?;
        Ok(self.load()?.remove(&provider))
    }

    pub fn clear(&self, provider: &str) -> AppResult<()> {
        let provider = validate_provider(provider)?;
        let mut entries = self.load()?;
        if entries.remove(&provider).is_some() {
            self.store(&entries)?;
        }
        Ok(())
    }

    /// Providers that currently have a stored key. A store that cannot be
    /// decrypted reports nothing rather than failing the whole settings page.
    pub fn status(&self) -> Vec<String> {
        self.load().map(|entries| entries.into_keys().collect()).unwrap_or_default()
    }

    fn load(&self) -> AppResult<BTreeMap<String, String>> {
        match self.read_entries() {
            Ok(entries) => Ok(entries),
            Err(LoadError::Missing) => Ok(BTreeMap::new()),
            Err(LoadError::Corrupt) => Err("凭据文件已损坏或无法解密".into()),
            Err(LoadError::Io) => Err("无法读取凭据文件".into()),
        }
    }

    /// Read the protected map, keeping the failure kind so `save` can decide
    /// whether a broken blob may be quarantined (corrupt) or must be reported
    /// as-is (IO).
    fn read_entries(&self) -> Result<BTreeMap<String, String>, LoadError> {
        let bytes = match std::fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Err(LoadError::Missing),
            Err(_) => return Err(LoadError::Io),
        };
        let plain = unprotect(&bytes).map_err(|_| LoadError::Corrupt)?;
        serde_json::from_slice(&plain).map_err(|_| LoadError::Corrupt)
    }

    /// A blob from another user/machine (or a half-written file) must not block
    /// every future save: keep it for inspection under a new name and start
    /// over. Only the write path does this — reads stay side-effect free.
    fn quarantine(&self) {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_secs())
            .unwrap_or_default();
        let _ = std::fs::rename(&self.path, self.path.with_extension(format!("corrupt-{stamp}")));
    }

    fn store(&self, entries: &BTreeMap<String, String>) -> AppResult<()> {
        let plain = serde_json::to_vec(entries).map_err(|_| "凭据序列化失败".to_string())?;
        let protected = protect(&plain)?;
        let temporary = self.path.with_extension("tmp");
        std::fs::write(&temporary, &protected).map_err(|_| "无法写入凭据文件".to_string())?;
        restrict_permissions(&temporary)?;
        std::fs::rename(&temporary, &self.path).map_err(|_| "无法保存凭据文件".to_string())
    }
}

fn validate_provider(provider: &str) -> AppResult<String> {
    let trimmed = provider.trim();
    if trimmed.is_empty()
        || trimmed.chars().count() > MAX_PROVIDER_CHARS
        || !trimmed.chars().all(|value| value.is_ascii_alphanumeric() || matches!(value, '-' | '_'))
    {
        return Err("未知 Provider".into());
    }
    Ok(trimmed.to_ascii_lowercase())
}

#[cfg(windows)]
fn protect(data: &[u8]) -> AppResult<Vec<u8>> {
    use windows_sys::Win32::{
        Foundation::LocalFree,
        Security::Cryptography::{CryptProtectData, CRYPT_INTEGER_BLOB},
    };
    unsafe {
        let input = CRYPT_INTEGER_BLOB {
            cbData: data.len() as u32,
            pbData: data.as_ptr() as *mut u8,
        };
        let mut output = CRYPT_INTEGER_BLOB { cbData: 0, pbData: std::ptr::null_mut() };
        let ok = CryptProtectData(
            &input,
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
            0,
            &mut output,
        );
        if ok == 0 || output.pbData.is_null() {
            return Err("凭据加密失败".into());
        }
        let bytes = std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
        LocalFree(output.pbData as *mut _);
        Ok(bytes)
    }
}

#[cfg(windows)]
fn unprotect(data: &[u8]) -> AppResult<Vec<u8>> {
    use windows_sys::Win32::{
        Foundation::LocalFree,
        Security::Cryptography::{CryptUnprotectData, CRYPT_INTEGER_BLOB},
    };
    unsafe {
        let input = CRYPT_INTEGER_BLOB {
            cbData: data.len() as u32,
            pbData: data.as_ptr() as *mut u8,
        };
        let mut output = CRYPT_INTEGER_BLOB { cbData: 0, pbData: std::ptr::null_mut() };
        let ok = CryptUnprotectData(
            &input,
            std::ptr::null_mut(),
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
            0,
            &mut output,
        );
        if ok == 0 || output.pbData.is_null() {
            return Err("凭据无法解密（可能来自其他用户或设备）".into());
        }
        let bytes = std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
        LocalFree(output.pbData as *mut _);
        Ok(bytes)
    }
}

/// Development fallback outside Windows: no OS key protection is available, so
/// the file is at least owner-only. Never reached by the shipped Windows build.
#[cfg(not(windows))]
fn protect(data: &[u8]) -> AppResult<Vec<u8>> {
    Ok(data.to_vec())
}

#[cfg(not(windows))]
fn unprotect(data: &[u8]) -> AppResult<Vec<u8>> {
    Ok(data.to_vec())
}

#[cfg(unix)]
fn restrict_permissions(path: &Path) -> AppResult<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .map_err(|_| "无法设置凭据文件权限".to_string())
}

#[cfg(not(unix))]
fn restrict_permissions(_path: &Path) -> AppResult<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_get_clear_roundtrip_never_leaves_plaintext_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let store = CredentialStore::open(dir.path()).unwrap();
        assert!(store.get("openai").unwrap().is_none());

        store.save("openai", "sk-secret-value").unwrap();
        assert_eq!(store.get("openai").unwrap().as_deref(), Some("sk-secret-value"));
        assert_eq!(store.status(), vec!["openai".to_string()]);

        let raw = std::fs::read(dir.path().join("credentials.bin")).unwrap();
        assert!(
            !raw.windows("sk-secret-value".len()).any(|window| window == b"sk-secret-value"),
            "the key must not be stored in clear text"
        );

        store.save("deepseek", "sk-second").unwrap();
        assert_eq!(store.status(), vec!["deepseek".to_string(), "openai".to_string()]);

        store.clear("openai").unwrap();
        assert!(store.get("openai").unwrap().is_none());
        assert_eq!(store.status(), vec!["deepseek".to_string()]);
    }

    #[test]
    fn invalid_providers_and_keys_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let store = CredentialStore::open(dir.path()).unwrap();
        for provider in ["", "  ", "../etc", "a b", &"x".repeat(65)] {
            assert!(store.save(provider, "key").is_err(), "{provider}");
        }
        assert!(store.save("openai", "   ").is_err());
        assert!(store.save("openai", "bad\u{0}key").is_err());
        assert!(store.save("openai", &"k".repeat(8193)).is_err());
    }

    #[test]
    fn a_corrupt_store_is_quarantined_and_rebuilt() {
        let dir = tempfile::tempdir().unwrap();
        let store = CredentialStore::open(dir.path()).unwrap();
        std::fs::write(dir.path().join("credentials.bin"), b"not a dpapi blob").unwrap();

        // Reads report the damage instead of pretending the key is absent…
        assert!(store.get("openai").is_err());
        assert!(store.status().is_empty());

        // …and the next save keeps the bad blob for inspection, then succeeds.
        store.save("openai", "sk-recovered").unwrap();
        assert_eq!(store.get("openai").unwrap().as_deref(), Some("sk-recovered"));
        let quarantined = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|entry| entry.ok())
            .any(|entry| entry.file_name().to_string_lossy().contains("corrupt-"));
        assert!(quarantined, "the unreadable blob must be kept aside");
    }

    #[test]
    fn an_unreadable_path_is_reported_instead_of_quarantined() {
        let dir = tempfile::tempdir().unwrap();
        let store = CredentialStore::open(dir.path()).unwrap();
        // A directory at the blob path is an IO error, not a corrupt blob.
        std::fs::create_dir(dir.path().join("credentials.bin")).unwrap();
        assert!(store.save("openai", "sk-test").is_err());
        assert!(store.get("openai").is_err());
        assert!(dir.path().join("credentials.bin").is_dir());
    }

    #[test]
    fn a_missing_store_reads_as_empty() {
        let dir = tempfile::tempdir().unwrap();
        let store = CredentialStore::open(dir.path()).unwrap();
        assert!(store.status().is_empty());
        assert!(store.get("ollama").unwrap().is_none());
    }
}

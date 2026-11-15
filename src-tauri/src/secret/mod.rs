//! Where this device's hub token is kept: the OS keychain on desktop (Windows Credential Manager, the macOS keychain,
//! the Secret Service on Linux), and on Android a file sealed by a key in the Android Keystore. The token is the only
//! secret the app has; with it, anyone could read the family's chats, so it never goes in the database or the log.

#[cfg(target_os = "android")]
mod android;

use std::path::{Path, PathBuf};

use crate::error::{AppError, AppResult};

pub struct TokenStore {
    #[cfg_attr(not(target_os = "android"), allow(dead_code))]
    dir: PathBuf,
    /// Separates the token of a second copy run for testing (`KITCHAT_PROFILE`).
    #[cfg_attr(target_os = "android", allow(dead_code))]
    profile: String,
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
const SERVICE: &str = "com.kitchat.app";

impl TokenStore {
    pub fn new(dir: &Path, profile: &str) -> Self {
        Self { dir: dir.to_path_buf(), profile: profile.to_string() }
    }

    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    fn entry(&self) -> AppResult<keyring::Entry> {
        let name =
            if self.profile.is_empty() { "device-token".to_string() } else { format!("device-token:{}", self.profile) };
        keyring::Entry::new(SERVICE, &name).map_err(|e| AppError::Other(format!("keychain: {e}")))
    }

    pub fn save(&self, token: &str) -> AppResult<()> {
        #[cfg(not(any(target_os = "android", target_os = "ios")))]
        {
            self.entry()?.set_password(token).map_err(|e| AppError::Other(format!("keychain: {e}")))?;
        }
        #[cfg(target_os = "android")]
        android::save(&self.dir, token)?;
        #[cfg(target_os = "ios")]
        std::fs::write(self.dir.join("device-token"), token)?;
        // As TerminalLynx does: only report success once it reads back.
        match self.load()? {
            Some(read) if read == token => Ok(()),
            _ => Err(AppError::Other("the keychain didn't keep the device's key".into())),
        }
    }

    pub fn load(&self) -> AppResult<Option<String>> {
        #[cfg(not(any(target_os = "android", target_os = "ios")))]
        {
            match self.entry()?.get_password() {
                Ok(token) => Ok(Some(token)),
                Err(keyring::Error::NoEntry) => Ok(None),
                Err(e) => Err(AppError::Other(format!("keychain: {e}"))),
            }
        }
        #[cfg(target_os = "android")]
        {
            android::load(&self.dir)
        }
        #[cfg(target_os = "ios")]
        {
            Ok(std::fs::read_to_string(self.dir.join("device-token")).ok())
        }
    }

    pub fn delete(&self) {
        #[cfg(not(any(target_os = "android", target_os = "ios")))]
        if let Ok(entry) = self.entry() {
            let _ = entry.delete_credential();
        }
        #[cfg(target_os = "android")]
        android::delete(&self.dir);
        #[cfg(target_os = "ios")]
        let _ = std::fs::remove_file(self.dir.join("device-token"));
    }
}

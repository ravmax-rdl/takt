//! Where refresh tokens live: the OS keychain, or memory in tests.

use std::collections::HashMap;
use std::sync::Mutex;

use crate::GoogleError;

const SERVICE: &str = "takt";

pub trait TokenStore: Send + Sync {
    fn get(&self, email: &str) -> Result<Option<String>, GoogleError>;
    fn set(&self, email: &str, refresh_token: &str) -> Result<(), GoogleError>;
    fn delete(&self, email: &str) -> Result<(), GoogleError>;
}

/// Windows Credential Manager, macOS Keychain or Secret Service.
pub struct KeyringTokenStore;

fn entry(email: &str) -> Result<keyring::Entry, GoogleError> {
    keyring::Entry::new(SERVICE, email).map_err(|e| GoogleError::TokenStore(e.to_string()))
}

impl TokenStore for KeyringTokenStore {
    fn get(&self, email: &str) -> Result<Option<String>, GoogleError> {
        match entry(email)?.get_password() {
            Ok(token) => Ok(Some(token)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(GoogleError::TokenStore(e.to_string())),
        }
    }

    fn set(&self, email: &str, refresh_token: &str) -> Result<(), GoogleError> {
        entry(email)?
            .set_password(refresh_token)
            .map_err(|e| GoogleError::TokenStore(e.to_string()))
    }

    fn delete(&self, email: &str) -> Result<(), GoogleError> {
        match entry(email)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(GoogleError::TokenStore(e.to_string())),
        }
    }
}

#[derive(Default)]
pub struct MemoryTokenStore(Mutex<HashMap<String, String>>);

impl TokenStore for MemoryTokenStore {
    fn get(&self, email: &str) -> Result<Option<String>, GoogleError> {
        Ok(self
            .0
            .lock()
            .expect("token map poisoned")
            .get(email)
            .cloned())
    }

    fn set(&self, email: &str, refresh_token: &str) -> Result<(), GoogleError> {
        self.0
            .lock()
            .expect("token map poisoned")
            .insert(email.to_owned(), refresh_token.to_owned());
        Ok(())
    }

    fn delete(&self, email: &str) -> Result<(), GoogleError> {
        self.0.lock().expect("token map poisoned").remove(email);
        Ok(())
    }
}

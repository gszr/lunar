//! Lunar-managed credentials in `$LUNAR_HOME/recorder/auth.json`.

mod http;
mod openai;
mod store;
mod xai;

use std::sync::atomic::AtomicBool;

#[derive(Clone, Debug, PartialEq)]
pub enum Credential {
    ApiKey {
        key: String,
    },
    OAuth {
        access: String,
        refresh: String,
        expires: u64,
    },
}

pub struct DeviceCode {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub interval: u64,
    pub expires_in: u64,
}

pub fn request_xai_device_code() -> Result<DeviceCode, String> {
    xai::request_device_code()
}

pub fn poll_xai(device: &DeviceCode, cancel: &AtomicBool) -> Result<Credential, String> {
    xai::poll(device, cancel)
}

pub fn request_openai_device_code() -> Result<DeviceCode, String> {
    openai::request_device_code()
}

pub fn poll_openai(device: &DeviceCode, cancel: &AtomicBool) -> Result<Credential, String> {
    openai::poll(device, cancel)
}

pub fn chatgpt_account_id(access: &str) -> Result<String, String> {
    openai::chatgpt_account_id(access)
}

pub fn resolve(provider: &str) -> Result<String, String> {
    let path = store::path();
    let mut credentials = store::load(&path)?;
    match credentials.remove(provider) {
        Some(Credential::ApiKey { key }) => Ok(key),
        Some(Credential::OAuth {
            access,
            refresh: _,
            expires,
        }) if http::now_ms() < expires => {
            if provider == "openai" {
                chatgpt_account_id(&access)?;
            }
            Ok(access)
        }
        Some(Credential::OAuth { refresh, .. }) => {
            let fresh = match provider {
                "xai" => xai::refresh(&refresh),
                "openai" => openai::refresh(&refresh),
                _ => Err(format!("unknown auth provider: {provider}")),
            }?;
            let access = match &fresh {
                Credential::OAuth { access, .. } => access.clone(),
                Credential::ApiKey { .. } => unreachable!(),
            };
            credentials.insert(provider.to_string(), fresh);
            store::save(&path, &credentials).map_err(|e| format!("auth.json: {e}"))?;
            Ok(access)
        }
        None => Err(format!("no auth for {provider}; run /login {provider}")),
    }
}

pub fn save_api_key(provider: &str, key: &str) -> Result<(), String> {
    if key.trim().is_empty() {
        return Err("API key is empty".into());
    }
    store::insert(
        provider,
        Credential::ApiKey {
            key: key.trim().into(),
        },
    )
}

pub fn save_oauth(provider: &str, credential: Credential) -> Result<(), String> {
    store::insert(provider, credential)
}

pub fn logout(provider: &str) -> Result<bool, String> {
    store::remove(provider)
}

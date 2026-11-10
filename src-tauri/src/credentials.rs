use anyhow::{anyhow, Result};
use windows::core::HSTRING;
use windows::Security::Credentials::{PasswordCredential, PasswordVault};

const RESOURCE: &str = "KotobaBeat.DeepSeek";
const USERNAME: &str = "default";

pub fn store_api_key(api_key: &str) -> Result<()> {
    let value = api_key.trim();
    if value.is_empty() {
        return Err(anyhow!("API Key 不能为空"));
    }
    let vault = PasswordVault::new()?;
    let resource = HSTRING::from(RESOURCE);
    let username = HSTRING::from(USERNAME);
    // Add replaces an existing resource/user pair. Never delete the working
    // credential first: a failed replacement must not erase the old key.
    let password = HSTRING::from(value);
    let credential = PasswordCredential::CreatePasswordCredential(&resource, &username, &password)?;
    vault.Add(&credential)?;
    Ok(())
}

pub fn get_api_key() -> Result<Option<String>> {
    let vault = PasswordVault::new()?;
    let resource = HSTRING::from(RESOURCE);
    let username = HSTRING::from(USERNAME);
    let credential = match vault.Retrieve(&resource, &username) {
        Ok(credential) => credential,
        Err(error) if credential_not_found(error.code().0) => return Ok(None),
        Err(_) => return Err(anyhow!("Windows 凭据库暂时不可用，请稍后重试")),
    };
    credential.RetrievePassword()?;
    Ok(Some(credential.Password()?.to_string_lossy()))
}

pub fn delete_api_key() -> Result<()> {
    let vault = PasswordVault::new()?;
    let resource = HSTRING::from(RESOURCE);
    let username = HSTRING::from(USERNAME);
    match vault.Retrieve(&resource, &username) {
        Ok(existing) => vault.Remove(&existing)?,
        Err(error) if credential_not_found(error.code().0) => {}
        Err(_) => return Err(anyhow!("无法访问 Windows 凭据库，未清除 API Key")),
    }
    Ok(())
}

fn credential_not_found(code: i32) -> bool {
    // HRESULT_FROM_WIN32(ERROR_NOT_FOUND), used by PasswordVault for absent entries.
    code as u32 == 0x80070490
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn distinguishes_absent_key_from_access_failure() {
        assert!(credential_not_found(0x80070490_u32 as i32));
        assert!(!credential_not_found(0x80070005_u32 as i32));
        assert!(!credential_not_found(0x80004005_u32 as i32));
    }
}

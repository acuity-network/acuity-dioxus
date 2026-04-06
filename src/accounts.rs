use base64::Engine;
use crypto_secretbox::{
    aead::{Aead, KeyInit},
    Key, Nonce, XSalsa20Poly1305,
};
use rand::{rngs::OsRng, RngCore};
use schnorrkel::{ExpansionMode, MiniSecretKey};
use serde::{Deserialize, Serialize};
use sp_core::{crypto::Ss58Codec, sr25519::Pair as Sr25519Pair, Pair};
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use subxt_signer::{polkadot_js_compat, sr25519::Keypair as SignerKeypair};

const APP_CONFIG_DIR: &str = "acuity-dioxus";
const ACCOUNTS_DIR: &str = "accounts";
const PKCS8_HEADER: [u8; 16] = [48, 83, 2, 1, 1, 48, 5, 6, 3, 43, 101, 112, 4, 34, 4, 32];
const PKCS8_DIVIDER: [u8; 5] = [161, 35, 3, 33, 0];
const SCRYPT_N: u32 = 32768;
const SCRYPT_P: u32 = 1;
const SCRYPT_R: u32 = 8;

#[derive(Clone, PartialEq)]
pub struct AccountEntry {
    pub id: String,
    pub name: String,
    pub address: String,
    pub path: PathBuf,
}

#[derive(Clone)]
pub struct AccountStore {
    pub config_dir: Option<String>,
    pub accounts_dir: Option<String>,
    pub accounts: Vec<AccountEntry>,
    pub active_account_id: Option<String>,
    /// Keyed by account `id` — any account can be independently unlocked.
    pub unlocked_signers: HashMap<String, SignerKeypair>,
    pub notice_message: Option<String>,
    pub error_message: Option<String>,
}

impl Default for AccountStore {
    fn default() -> Self {
        Self {
            config_dir: None,
            accounts_dir: None,
            accounts: Vec::new(),
            active_account_id: None,
            unlocked_signers: HashMap::new(),
            notice_message: None,
            error_message: None,
        }
    }
}

impl PartialEq for AccountStore {
    fn eq(&self, other: &Self) -> bool {
        self.config_dir == other.config_dir
            && self.accounts_dir == other.accounts_dir
            && self.accounts == other.accounts
            && self.active_account_id == other.active_account_id
            // Compare only which IDs are unlocked, not the key material itself
            && {
                let mut a: Vec<&String> = self.unlocked_signers.keys().collect();
                let mut b: Vec<&String> = other.unlocked_signers.keys().collect();
                a.sort();
                b.sort();
                a == b
            }
            && self.notice_message == other.notice_message
            && self.error_message == other.error_message
    }
}

impl AccountStore {
    pub fn active_account(&self) -> Option<&AccountEntry> {
        self.active_account_id
            .as_deref()
            .and_then(|id| self.accounts.iter().find(|account| account.id == id))
    }

    pub fn is_active_unlocked(&self) -> bool {
        self.active_account_id
            .as_deref()
            .map(|id| self.unlocked_signers.contains_key(id))
            .unwrap_or(false)
    }

    pub fn is_account_unlocked(&self, id: &str) -> bool {
        self.unlocked_signers.contains_key(id)
    }

    /// Returns a reference to the signer for the active account, if it is
    /// currently unlocked.
    pub fn active_signer(&self) -> Option<&SignerKeypair> {
        self.active_account_id
            .as_deref()
            .and_then(|id| self.unlocked_signers.get(id))
    }

    fn set_notice(&mut self, message: impl Into<String>) {
        self.notice_message = Some(message.into());
        self.error_message = None;
    }

    fn set_error(&mut self, message: impl Into<String>) {
        self.notice_message = None;
        self.error_message = Some(message.into());
    }
}

#[derive(Deserialize, Serialize)]
struct PolkadotJsAccountFile {
    encoded: String,
    encoding: EncryptionMetadata,
    address: String,
    meta: AccountFileMeta,
}

#[derive(Clone, Deserialize, Serialize)]
struct EncryptionMetadata {
    content: Vec<String>,
    #[serde(rename = "type")]
    kind: Vec<String>,
    version: String,
}

#[derive(Clone, Deserialize, Serialize)]
struct AccountFileMeta {
    name: String,
    #[serde(rename = "whenCreated")]
    when_created: u128,
    #[serde(rename = "genesisHash")]
    genesis_hash: String,
}

pub fn load_account_store() -> AccountStore {
    let config_dir = match config_directory() {
        Ok(path) => path,
        Err(error) => {
            return AccountStore {
                error_message: Some(error),
                ..AccountStore::default()
            };
        }
    };

    let accounts_dir = config_dir.join(ACCOUNTS_DIR);
    if let Err(error) = fs::create_dir_all(&accounts_dir) {
        return AccountStore {
            config_dir: Some(config_dir.display().to_string()),
            accounts_dir: Some(accounts_dir.display().to_string()),
            error_message: Some(format!("Failed to create accounts directory: {error}")),
            ..AccountStore::default()
        };
    }

    let (accounts, skipped_files) = load_accounts_from_directory(&accounts_dir);
    let notice_message = if skipped_files == 0 {
        None
    } else {
        Some(format!(
            "Loaded {} account(s). Skipped {} invalid file(s).",
            accounts.len(),
            skipped_files
        ))
    };

    let active_account_id = accounts.first().map(|account| account.id.clone());

    AccountStore {
        config_dir: Some(config_dir.display().to_string()),
        accounts_dir: Some(accounts_dir.display().to_string()),
        accounts,
        active_account_id,
        unlocked_signers: HashMap::new(),
        notice_message,
        error_message: None,
    }
}

pub fn select_active_account(store: &mut AccountStore, account_id: &str) {
    if store
        .accounts
        .iter()
        .all(|account| account.id != account_id)
    {
        store.set_error("That account is no longer available.");
        return;
    }

    store.active_account_id = Some(account_id.to_string());
    if let Some(account) = store.active_account() {
        store.set_notice(format!("Selected {}.", account.name));
    }
}

pub fn create_account(store: &mut AccountStore, name: &str, password: &str) {
    let name = name.trim();
    if name.is_empty() {
        store.set_error("Enter an account name.");
        return;
    }

    if password.is_empty() {
        store.set_error("Enter a password for the new account.");
        return;
    }

    let accounts_dir = match store.accounts_dir_path() {
        Ok(path) => path,
        Err(error) => {
            store.set_error(error);
            return;
        }
    };

    let (pair, seed) = Sr25519Pair::generate();
    let address = pair.public().to_ss58check();

    if store
        .accounts
        .iter()
        .any(|account| account.address == address)
    {
        store.set_error("An account with that address already exists.");
        return;
    }

    let created_at = unix_timestamp_millis();
    let file_name = format!("{}-{}.json", slugify(name), created_at);
    let account_path = accounts_dir.join(&file_name);

    let account_json = match export_account_json(&pair, &seed, name, password, created_at) {
        Ok(account_json) => account_json,
        Err(error) => {
            store.set_error(error);
            return;
        }
    };

    if let Err(error) = fs::write(&account_path, account_json) {
        store.set_error(format!("Failed to write account file: {error}"));
        return;
    }

    let account = AccountEntry {
        id: file_name,
        name: name.to_string(),
        address,
        path: account_path,
    };

    store.accounts.push(account.clone());
    sort_accounts(&mut store.accounts);
    store.active_account_id = Some(account.id.clone());
    store.set_notice(format!("Created {}.", account.name));
}

/// Run the CPU-heavy scrypt + decrypt on a background thread.
/// Returns `Ok((signer, account_id, account_name))` on success or `Err(message)` on failure.
/// This function is `Send + 'static` so it can be passed to `spawn_blocking`.
pub fn unlock_account_blocking(
    account_path: PathBuf,
    account_id: String,
    account_name: String,
    password: String,
) -> Result<(SignerKeypair, String, String), String> {
    let account_json = fs::read_to_string(&account_path)
        .map_err(|e| format!("Failed to read account file: {e}"))?;
    let signer = polkadot_js_compat::decrypt_json(&account_json, &password)
        .map_err(|e| format!("Failed to unlock account: {e}"))?;
    Ok((signer, account_id, account_name))
}

pub fn apply_unlock_result(
    store: &mut AccountStore,
    result: Result<(SignerKeypair, String, String), String>,
) {
    match result {
        Ok((signer, id, name)) => {
            store.unlocked_signers.insert(id, signer);
            store.set_notice(format!("Unlocked {}.", name));
        }
        Err(message) => store.set_error(message),
    }
}

#[cfg_attr(not(test), allow(dead_code))]
pub fn unlock_active_account(store: &mut AccountStore, password: &str) {
    if password.is_empty() {
        store.set_error("Enter the password for the active account.");
        return;
    }

    let Some(account) = store.active_account().cloned() else {
        store.set_error("Select an account first.");
        return;
    };

    let result =
        unlock_account_blocking(account.path, account.id, account.name, password.to_string());
    apply_unlock_result(store, result);
}

/// Lock the account with the given id.
pub fn lock_account(store: &mut AccountStore, account_id: &str) {
    let name = store
        .accounts
        .iter()
        .find(|a| a.id == account_id)
        .map(|a| a.name.clone());

    if store.unlocked_signers.remove(account_id).is_some() {
        if let Some(name) = name {
            store.set_notice(format!("Locked {}.", name));
        }
    } else if let Some(name) = name {
        store.set_notice(format!("{} is already locked.", name));
    }
}

/// Delete any account by id (not just the active one).
pub fn delete_account(store: &mut AccountStore, account_id: &str) {
    let Some(account_index) = store
        .accounts
        .iter()
        .position(|account| account.id == account_id)
    else {
        store.set_error("That account is no longer available.");
        return;
    };

    let account = store.accounts[account_index].clone();
    if let Err(error) = fs::remove_file(&account.path) {
        store.set_error(format!("Failed to delete account file: {error}"));
        return;
    }

    store.accounts.remove(account_index);
    store.unlocked_signers.remove(account_id);

    // If the deleted account was active, select another one
    if store.active_account_id.as_deref() == Some(account_id) {
        store.active_account_id = store
            .accounts
            .first()
            .map(|next_account| next_account.id.clone());
    }

    store.set_notice(format!("Deleted {}.", account.name));
}

impl AccountStore {
    fn accounts_dir_path(&self) -> Result<PathBuf, String> {
        let Some(accounts_dir) = self.accounts_dir.as_deref() else {
            return Err("Accounts directory is unavailable.".to_string());
        };

        Ok(PathBuf::from(accounts_dir))
    }
}

fn config_directory() -> Result<PathBuf, String> {
    let Some(mut home_dir) = home::home_dir() else {
        return Err("No home directory found for this user.".to_string());
    };

    home_dir.push(".config");
    home_dir.push(APP_CONFIG_DIR);
    Ok(home_dir)
}

fn load_accounts_from_directory(accounts_dir: &Path) -> (Vec<AccountEntry>, usize) {
    let mut accounts = Vec::new();
    let mut skipped_files = 0;

    let entries = match fs::read_dir(accounts_dir) {
        Ok(entries) => entries,
        Err(_) => return (accounts, 0),
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() || path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }

        let Ok(account_json) = fs::read_to_string(&path) else {
            skipped_files += 1;
            continue;
        };
        let Ok(parsed_account) = serde_json::from_str::<PolkadotJsAccountFile>(&account_json)
        else {
            skipped_files += 1;
            continue;
        };

        let Some(file_name) = path.file_name().and_then(|file_name| file_name.to_str()) else {
            skipped_files += 1;
            continue;
        };

        let display_name = if parsed_account.meta.name.trim().is_empty() {
            short_address(&parsed_account.address)
        } else {
            parsed_account.meta.name
        };

        accounts.push(AccountEntry {
            id: file_name.to_string(),
            name: display_name,
            address: parsed_account.address,
            path,
        });
    }

    sort_accounts(&mut accounts);
    (accounts, skipped_files)
}

fn sort_accounts(accounts: &mut [AccountEntry]) {
    accounts.sort_by(|left, right| {
        left.name
            .to_lowercase()
            .cmp(&right.name.to_lowercase())
            .then_with(|| left.address.cmp(&right.address))
    });
}

fn export_account_json(
    pair: &Sr25519Pair,
    seed: &[u8; 32],
    name: &str,
    password: &str,
    created_at: u128,
) -> Result<String, String> {
    let mut salt = [0_u8; 32];
    let mut nonce_bytes = [0_u8; 24];
    OsRng.fill_bytes(&mut salt);
    OsRng.fill_bytes(&mut nonce_bytes);

    let mut scrypt_key = Key::default();
    let scrypt_params = scrypt::Params::new(15, 8, 1, 32)
        .map_err(|error| format!("Failed to configure scrypt: {error}"))?;
    scrypt::scrypt(password.as_bytes(), &salt, &scrypt_params, &mut scrypt_key)
        .map_err(|error| format!("Failed to derive account key: {error}"))?;

    let secret_key = MiniSecretKey::from_bytes(seed)
        .map_err(|error| format!("Failed to derive account secret key: {error}"))?
        .expand(ExpansionMode::Ed25519)
        .to_ed25519_bytes();
    let public_key = pair.public().0;

    let mut plaintext = Vec::with_capacity(117);
    plaintext.extend_from_slice(&PKCS8_HEADER);
    plaintext.extend_from_slice(&secret_key);
    plaintext.extend_from_slice(&PKCS8_DIVIDER);
    plaintext.extend_from_slice(&public_key);

    let cipher = XSalsa20Poly1305::new(&scrypt_key);
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ciphertext = cipher
        .encrypt(nonce, plaintext.as_slice())
        .map_err(|error| format!("Failed to encrypt account: {error}"))?;

    let mut encoded_bytes = Vec::with_capacity(68 + ciphertext.len());
    encoded_bytes.extend_from_slice(&salt);
    encoded_bytes.extend_from_slice(&SCRYPT_N.to_le_bytes());
    encoded_bytes.extend_from_slice(&SCRYPT_P.to_le_bytes());
    encoded_bytes.extend_from_slice(&SCRYPT_R.to_le_bytes());
    encoded_bytes.extend_from_slice(&nonce_bytes);
    encoded_bytes.extend_from_slice(&ciphertext);

    let account_json = PolkadotJsAccountFile {
        encoded: base64::engine::general_purpose::STANDARD.encode(encoded_bytes),
        encoding: EncryptionMetadata {
            content: vec!["pkcs8".to_string(), "sr25519".to_string()],
            kind: vec!["scrypt".to_string(), "xsalsa20-poly1305".to_string()],
            version: "3".to_string(),
        },
        address: pair.public().to_ss58check(),
        meta: AccountFileMeta {
            name: name.to_string(),
            when_created: created_at,
            genesis_hash: String::new(),
        },
    };

    serde_json::to_string_pretty(&account_json)
        .map_err(|error| format!("Failed to serialize account JSON: {error}"))
}

fn slugify(name: &str) -> String {
    let mut slug = String::new();

    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
        } else if (ch == ' ' || ch == '-' || ch == '_') && !slug.ends_with('-') {
            slug.push('-');
        }
    }

    let slug = slug.trim_matches('-').to_string();
    if slug.is_empty() {
        "account".to_string()
    } else {
        slug
    }
}

fn short_address(address: &str) -> String {
    if address.len() <= 14 {
        return address.to_string();
    }

    format!("{}...{}", &address[..6], &address[address.len() - 6..])
}

fn unix_timestamp_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::engine::general_purpose::STANDARD;
    use std::str::FromStr;
    use std::{env, fs, path::PathBuf, process};
    use subxt_signer::SecretUri;

    fn test_store(test_dir: &Path) -> AccountStore {
        AccountStore {
            config_dir: Some(test_dir.display().to_string()),
            accounts_dir: Some(test_dir.display().to_string()),
            accounts: Vec::new(),
            active_account_id: None,
            unlocked_signers: HashMap::new(),
            notice_message: None,
            error_message: None,
        }
    }

    fn account_entry(test_dir: &Path, id: &str, name: &str, address: &str) -> AccountEntry {
        AccountEntry {
            id: id.to_string(),
            name: name.to_string(),
            address: address.to_string(),
            path: test_dir.join(id),
        }
    }

    fn signer_for_test(label: &str) -> SignerKeypair {
        let phrase = format!("//{label}");
        let uri = SecretUri::from_str(&phrase).unwrap();
        subxt_signer::sr25519::Keypair::from_uri(&uri).unwrap()
    }

    fn write_account_file(
        path: &Path,
        address: &str,
        name: &str,
        encoded: &str,
        when_created: u128,
    ) {
        let account = PolkadotJsAccountFile {
            encoded: encoded.to_string(),
            encoding: EncryptionMetadata {
                content: vec!["pkcs8".to_string(), "sr25519".to_string()],
                kind: vec!["scrypt".to_string(), "xsalsa20-poly1305".to_string()],
                version: "3".to_string(),
            },
            address: address.to_string(),
            meta: AccountFileMeta {
                name: name.to_string(),
                when_created,
                genesis_hash: String::new(),
            },
        };

        fs::write(path, serde_json::to_string_pretty(&account).unwrap()).unwrap();
    }

    #[test]
    fn exported_account_json_round_trips_with_subxt_signer() {
        let (pair, seed) = Sr25519Pair::generate();
        let json = export_account_json(&pair, &seed, "Test Account", "secret-pass", 1).unwrap();

        let signer = polkadot_js_compat::decrypt_json(&json, "secret-pass").unwrap();

        assert_eq!(signer.public_key().0, pair.public().0);
    }

    #[test]
    fn exported_account_json_contains_expected_metadata() {
        let (pair, seed) = Sr25519Pair::generate();
        let json = export_account_json(&pair, &seed, "Test Account", "secret-pass", 42).unwrap();

        let parsed: PolkadotJsAccountFile = serde_json::from_str(&json).unwrap();
        let encoded = STANDARD.decode(parsed.encoded).unwrap();

        assert_eq!(parsed.address, pair.public().to_ss58check());
        assert_eq!(parsed.meta.name, "Test Account");
        assert_eq!(parsed.meta.when_created, 42);
        assert_eq!(parsed.meta.genesis_hash, "");
        assert_eq!(parsed.encoding.content, vec!["pkcs8", "sr25519"]);
        assert_eq!(parsed.encoding.kind, vec!["scrypt", "xsalsa20-poly1305"]);
        assert_eq!(parsed.encoding.version, "3");
        assert!(encoded.len() > 68);
    }

    #[test]
    fn created_account_starts_locked_and_unlocks_with_same_password() {
        let test_dir = unique_test_dir("create-unlock");
        fs::create_dir_all(&test_dir).unwrap();

        let mut store = test_store(&test_dir);

        create_account(&mut store, "Regression Account", "same-password");

        assert!(store.error_message.is_none());
        assert_eq!(store.accounts.len(), 1);
        assert!(store.active_account().is_some());
        assert!(!store.is_active_unlocked());

        unlock_active_account(&mut store, "same-password");

        assert!(store.error_message.is_none());
        assert!(store.is_active_unlocked());

        fs::remove_dir_all(&test_dir).unwrap();
    }

    #[test]
    fn wrong_password_keeps_account_locked() {
        let test_dir = unique_test_dir("wrong-password");
        fs::create_dir_all(&test_dir).unwrap();

        let mut store = test_store(&test_dir);

        create_account(&mut store, "Regression Account", "right-password");
        unlock_active_account(&mut store, "wrong-password");

        assert!(!store.is_active_unlocked());
        assert!(store.error_message.is_some());

        fs::remove_dir_all(&test_dir).unwrap();
    }

    #[test]
    fn active_account_returns_none_without_matching_id() {
        let test_dir = unique_test_dir("active-account-none");
        let mut store = test_store(&test_dir);
        store
            .accounts
            .push(account_entry(&test_dir, "one.json", "One", "addr-1"));

        assert!(store.active_account().is_none());

        store.active_account_id = Some("missing.json".to_string());

        assert!(store.active_account().is_none());
    }

    #[test]
    fn active_account_returns_matching_entry() {
        let test_dir = unique_test_dir("active-account-match");
        let mut store = test_store(&test_dir);
        store
            .accounts
            .push(account_entry(&test_dir, "one.json", "One", "addr-1"));
        store
            .accounts
            .push(account_entry(&test_dir, "two.json", "Two", "addr-2"));
        store.active_account_id = Some("two.json".to_string());

        let active = store.active_account().unwrap();
        assert_eq!(active.id, "two.json");
        assert_eq!(active.name, "Two");
    }

    #[test]
    fn active_unlock_helpers_reflect_store_state() {
        let test_dir = unique_test_dir("unlock-helpers");
        let mut store = test_store(&test_dir);
        store
            .accounts
            .push(account_entry(&test_dir, "one.json", "One", "addr-1"));

        assert!(!store.is_active_unlocked());
        assert!(!store.is_account_unlocked("one.json"));

        store.active_account_id = Some("one.json".to_string());
        assert!(!store.is_active_unlocked());

        store
            .unlocked_signers
            .insert("one.json".to_string(), signer_for_test("one"));

        assert!(store.is_active_unlocked());
        assert!(store.is_account_unlocked("one.json"));
        assert!(!store.is_account_unlocked("missing.json"));
    }

    #[test]
    fn account_store_equality_only_compares_unlocked_ids() {
        let test_dir = unique_test_dir("partial-eq");
        let account = account_entry(&test_dir, "one.json", "One", "addr-1");

        let mut left = test_store(&test_dir);
        left.accounts.push(account.clone());
        left.active_account_id = Some(account.id.clone());
        left.notice_message = Some("same".to_string());
        left.unlocked_signers
            .insert(account.id.clone(), signer_for_test("left"));

        let mut right = test_store(&test_dir);
        right.accounts.push(account.clone());
        right.active_account_id = Some(account.id.clone());
        right.notice_message = Some("same".to_string());
        right
            .unlocked_signers
            .insert(account.id.clone(), signer_for_test("right"));

        assert!(left == right);

        right
            .unlocked_signers
            .insert("other.json".to_string(), signer_for_test("other"));

        assert!(left != right);
    }

    #[test]
    fn select_active_account_updates_selection_and_notice() {
        let test_dir = unique_test_dir("select-active-valid");
        let mut store = test_store(&test_dir);
        store
            .accounts
            .push(account_entry(&test_dir, "one.json", "One", "addr-1"));
        store
            .accounts
            .push(account_entry(&test_dir, "two.json", "Two", "addr-2"));

        select_active_account(&mut store, "two.json");

        assert_eq!(store.active_account_id.as_deref(), Some("two.json"));
        assert_eq!(store.notice_message.as_deref(), Some("Selected Two."));
        assert_eq!(store.error_message, None);
    }

    #[test]
    fn select_active_account_rejects_unknown_id() {
        let test_dir = unique_test_dir("select-active-missing");
        let mut store = test_store(&test_dir);
        store
            .accounts
            .push(account_entry(&test_dir, "one.json", "One", "addr-1"));
        store.active_account_id = Some("one.json".to_string());

        select_active_account(&mut store, "missing.json");

        assert_eq!(store.active_account_id.as_deref(), Some("one.json"));
        assert_eq!(
            store.error_message.as_deref(),
            Some("That account is no longer available.")
        );
        assert_eq!(store.notice_message, None);
    }

    #[test]
    fn apply_unlock_result_success_inserts_signer_and_notice() {
        let test_dir = unique_test_dir("unlock-result-ok");
        let mut store = test_store(&test_dir);
        store.error_message = Some("old error".to_string());

        apply_unlock_result(
            &mut store,
            Ok((
                signer_for_test("ok"),
                "one.json".to_string(),
                "One".to_string(),
            )),
        );

        assert!(store.is_account_unlocked("one.json"));
        assert_eq!(store.notice_message.as_deref(), Some("Unlocked One."));
        assert_eq!(store.error_message, None);
    }

    #[test]
    fn apply_unlock_result_error_sets_error_and_clears_notice() {
        let test_dir = unique_test_dir("unlock-result-err");
        let mut store = test_store(&test_dir);
        store.notice_message = Some("old notice".to_string());

        apply_unlock_result(&mut store, Err("bad password".to_string()));

        assert_eq!(store.notice_message, None);
        assert_eq!(store.error_message.as_deref(), Some("bad password"));
    }

    #[test]
    fn lock_account_updates_notice_for_locked_and_already_locked_states() {
        let test_dir = unique_test_dir("lock-account");
        let mut store = test_store(&test_dir);
        store
            .accounts
            .push(account_entry(&test_dir, "one.json", "One", "addr-1"));
        store
            .unlocked_signers
            .insert("one.json".to_string(), signer_for_test("one"));

        lock_account(&mut store, "one.json");

        assert!(!store.is_account_unlocked("one.json"));
        assert_eq!(store.notice_message.as_deref(), Some("Locked One."));
        assert_eq!(store.error_message, None);

        lock_account(&mut store, "one.json");

        assert_eq!(
            store.notice_message.as_deref(),
            Some("One is already locked.")
        );
    }

    #[test]
    fn delete_account_removes_file_signer_and_reselects_active_account() {
        let test_dir = unique_test_dir("delete-account");
        fs::create_dir_all(&test_dir).unwrap();

        let mut store = test_store(&test_dir);
        let first = account_entry(&test_dir, "a.json", "A", "addr-a");
        let second = account_entry(&test_dir, "b.json", "B", "addr-b");
        fs::write(&first.path, "{}\n").unwrap();
        fs::write(&second.path, "{}\n").unwrap();
        store.accounts.push(first.clone());
        store.accounts.push(second.clone());
        store.active_account_id = Some(first.id.clone());
        store
            .unlocked_signers
            .insert(first.id.clone(), signer_for_test("a"));

        delete_account(&mut store, &first.id);

        assert!(!first.path.exists());
        assert_eq!(store.accounts.len(), 1);
        assert_eq!(store.accounts[0].id, second.id);
        assert_eq!(store.accounts[0].name, second.name);
        assert_eq!(store.accounts[0].address, second.address);
        assert!(!store.is_account_unlocked(&first.id));
        assert_eq!(store.active_account_id.as_deref(), Some(second.id.as_str()));
        assert_eq!(store.notice_message.as_deref(), Some("Deleted A."));
        assert_eq!(store.error_message, None);

        fs::remove_dir_all(&test_dir).unwrap();
    }

    #[test]
    fn delete_account_reports_missing_account() {
        let test_dir = unique_test_dir("delete-missing");
        let mut store = test_store(&test_dir);
        store.notice_message = Some("old notice".to_string());

        delete_account(&mut store, "missing.json");

        assert_eq!(store.notice_message, None);
        assert_eq!(
            store.error_message.as_deref(),
            Some("That account is no longer available.")
        );
    }

    #[test]
    fn load_accounts_from_directory_skips_invalid_files_and_sorts_accounts() {
        let test_dir = unique_test_dir("load-accounts");
        fs::create_dir_all(&test_dir).unwrap();

        write_account_file(
            &test_dir.join("b.json"),
            "5GrwvaEF5zXb26Fz9rcQpDWS57CtERHpNehXCPcNoHGKutQY",
            "beta",
            "encoded-b",
            1,
        );
        write_account_file(
            &test_dir.join("a.json"),
            "5FHneW46xGXgs5mUiveU4sbTyGBzmst1J9B3vFJQ9KQp7q9d",
            "Alpha",
            "encoded-a",
            2,
        );
        write_account_file(
            &test_dir.join("blank.json"),
            "1234567890abcdef",
            "   ",
            "encoded-c",
            3,
        );
        fs::write(test_dir.join("broken.json"), "{not json").unwrap();
        fs::write(test_dir.join("note.txt"), "ignore me").unwrap();

        let (accounts, skipped_files) = load_accounts_from_directory(&test_dir);

        assert_eq!(skipped_files, 1);
        assert_eq!(accounts.len(), 3);
        assert_eq!(accounts[0].id, "blank.json");
        assert_eq!(accounts[1].id, "a.json");
        assert_eq!(accounts[2].id, "b.json");
        assert_eq!(accounts[0].name, "123456...abcdef");

        fs::remove_dir_all(&test_dir).unwrap();
    }

    #[test]
    fn load_accounts_from_directory_returns_empty_when_directory_is_missing() {
        let test_dir = unique_test_dir("load-accounts-missing");

        let (accounts, skipped_files) = load_accounts_from_directory(&test_dir);

        assert!(accounts.is_empty());
        assert_eq!(skipped_files, 0);
    }

    #[test]
    fn slugify_normalizes_names_and_has_fallback() {
        assert_eq!(slugify("  My__Test Account---01  "), "my-test-account-01");
        assert_eq!(slugify("---___   !!!"), "account");
        assert_eq!(slugify("CamelCase"), "camelcase");
    }

    #[test]
    fn short_address_preserves_short_values_and_truncates_long_values() {
        assert_eq!(short_address("12345678901234"), "12345678901234");
        assert_eq!(short_address("1234567890abcdef"), "123456...abcdef");
    }

    fn unique_test_dir(label: &str) -> PathBuf {
        let unique = format!(
            "acuity-dioxus-{label}-{}-{}",
            process::id(),
            unix_timestamp_millis()
        );
        env::temp_dir().join(unique)
    }
}

//! macOS key storage adapter: seals saved history with AES-256-GCM under a
//! key kept in the login Keychain.

use repasta::store::{Sealer, StoreError};
use ring::aead::{AES_256_GCM, Aad, LessSafeKey, NONCE_LEN, Nonce, UnboundKey};
use ring::rand::{SecureRandom, SystemRandom};
use security_framework::passwords::{get_generic_password, set_generic_password};

const SERVICE: &str = "io.repasta.desktop";
const ACCOUNT: &str = "history-key";
const KEY_LEN: usize = 32;
/// `errSecItemNotFound`.
const ITEM_NOT_FOUND: i32 = -25300;
/// Binds sealed data to its purpose.
const AAD: &[u8] = b"repasta history v1";

pub struct KeychainSealer {
    key: LessSafeKey,
    random: SystemRandom,
}

impl KeychainSealer {
    /// Uses the Keychain key, creating it when `create` is set and none
    /// exists. Without `create`, a missing key is [`StoreError::Locked`], so
    /// existing encrypted history is never paired with a fresh key.
    pub fn open(create: bool) -> Result<Self, StoreError> {
        let random = SystemRandom::new();
        let bytes = match get_generic_password(SERVICE, ACCOUNT) {
            Ok(bytes) => bytes,
            Err(error) if error.code() == ITEM_NOT_FOUND && create => {
                let mut bytes = vec![0; KEY_LEN];
                random
                    .fill(&mut bytes)
                    .map_err(|_| StoreError::EncryptionUnavailable)?;
                set_generic_password(SERVICE, ACCOUNT, &bytes).map_err(|_| StoreError::Locked)?;
                bytes
            }
            // Not found, denied, or the Keychain is locked.
            Err(_) => return Err(StoreError::Locked),
        };
        let key = UnboundKey::new(&AES_256_GCM, &bytes).map_err(|_| StoreError::Locked)?;
        Ok(Self {
            key: LessSafeKey::new(key),
            random,
        })
    }
}

impl Sealer for KeychainSealer {
    /// Output: a random nonce, then ciphertext with its tag.
    fn seal(&self, plain: &[u8]) -> Result<Vec<u8>, StoreError> {
        let mut nonce = [0; NONCE_LEN];
        self.random
            .fill(&mut nonce)
            .map_err(|_| StoreError::EncryptionUnavailable)?;
        let mut body = plain.to_vec();
        self.key
            .seal_in_place_append_tag(
                Nonce::assume_unique_for_key(nonce),
                Aad::from(AAD),
                &mut body,
            )
            .map_err(|_| StoreError::EncryptionUnavailable)?;
        Ok([nonce.as_slice(), &body].concat())
    }

    fn open(&self, sealed: &[u8]) -> Result<Vec<u8>, StoreError> {
        let (nonce, body) = sealed
            .split_first_chunk::<NONCE_LEN>()
            .ok_or(StoreError::Unreadable)?;
        let mut body = body.to_vec();
        let plain = self
            .key
            .open_in_place(
                Nonce::assume_unique_for_key(*nonce),
                Aad::from(AAD),
                &mut body,
            )
            .map_err(|_| StoreError::Unreadable)?;
        Ok(plain.to_vec())
    }
}

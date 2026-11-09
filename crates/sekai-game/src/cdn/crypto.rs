//! Secrets, and the encrypted bodies they open.

use std::path::PathBuf;

use aes::Aes128;
use aes::cipher::block_padding::Pkcs7;
use aes::cipher::{BlockModeDecrypt, BlockModeEncrypt, KeyIvInit};
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::SourceError;
use super::region::SecretNames;

type Decryptor = cbc::Decryptor<Aes128>;
type Encryptor = cbc::Encryptor<Aes128>;

/// Where secret values are read from: an environment variable with the name of the secret, or a
/// file with that name in a directory outside the repository.
#[derive(Debug, Clone, Default)]
pub struct Secrets {
    directory: Option<PathBuf>,
}

impl Secrets {
    /// Secrets from the environment, and then from the files in `directory`.
    pub fn new(directory: Option<PathBuf>) -> Self {
        Self { directory }
    }

    /// The text of the secret `name`, without surrounding whitespace.
    pub(super) fn text(&self, name: &str) -> Result<String, SourceError> {
        let missing = || SourceError::new("secret", format!("the secret {name} is not set"));
        let text = match std::env::var(name) {
            Ok(text) => text,
            Err(_) => {
                let directory = self.directory.as_ref().ok_or_else(missing)?;
                std::fs::read_to_string(directory.join(name)).map_err(|_| missing())?
            }
        };
        Ok(text.trim().to_owned())
    }

    /// The file a new secret `name` is written to. The secret must not be set yet.
    pub(super) fn vacant(&self, name: &str) -> Result<PathBuf, SourceError> {
        if self.text(name).is_ok() {
            return Err(SourceError::new(
                "secret",
                format!("the secret {name} is already set"),
            ));
        }
        self.directory
            .as_ref()
            .map(|directory| directory.join(name))
            .ok_or_else(|| SourceError::new("secret", "no directory of secrets is given"))
    }

    fn read(&self, name: &str) -> Result<Vec<u8>, SourceError> {
        parse_hex(&self.text(name)?).ok_or_else(|| {
            SourceError::new("secret", format!("the secret {name} is not hexadecimal"))
        })
    }

    /// The key of encrypted HCA audio, when `names` name one: one to sixteen hexadecimal digits.
    pub fn hca_key(&self, names: &SecretNames) -> Result<Option<u64>, SourceError> {
        let Some(name) = &names.hca else {
            return Ok(None);
        };
        let text = self.text(name)?;
        if text.is_empty() || text.len() > 16 || !text.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(SourceError::new(
                "secret",
                format!("the secret {name} is not a 64-bit hexadecimal number"),
            ));
        }
        Ok(u64::from_str_radix(&text, 16).ok())
    }
}

fn parse_hex(text: &str) -> Option<Vec<u8>> {
    if !text.is_ascii() || !text.len().is_multiple_of(2) {
        return None;
    }
    text.as_bytes()
        .chunks(2)
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok())
        .collect()
}

/// The key that opens manifests and interface responses.
pub(super) struct Keys {
    key: Vec<u8>,
    iv: Vec<u8>,
}

impl Keys {
    pub(super) fn read(names: &SecretNames, secrets: &Secrets) -> Result<Self, SourceError> {
        let keys = Self {
            key: secrets.read(&names.key)?,
            iv: secrets.read(&names.iv)?,
        };
        Decryptor::new_from_slices(&keys.key, &keys.iv).map_err(|_| {
            SourceError::new(
                "secret",
                format!("{} and {} must be 16 bytes each", names.key, names.iv),
            )
        })?;
        Ok(keys)
    }

    /// Writes `document` as MessagePack and encrypts it.
    pub(super) fn encode<T: Serialize>(&self, document: &T) -> Result<Vec<u8>, SourceError> {
        let mut buffer =
            rmp_serde::to_vec_named(document).map_err(|e| SourceError::new("decode", e))?;
        let length = buffer.len();
        buffer.resize(length + 16, 0);
        let written = Encryptor::new_from_slices(&self.key, &self.iv)
            .map_err(|_| SourceError::new("decode", "the key has the wrong length"))?
            .encrypt_padded::<Pkcs7>(&mut buffer, length)
            .map_err(|_| SourceError::new("decode", "the document does not fit its buffer"))?
            .len();
        buffer.truncate(written);
        Ok(buffer)
    }

    /// Decrypts `body` and reads the MessagePack document inside.
    pub(super) fn decode<T: DeserializeOwned>(&self, body: &[u8]) -> Result<T, SourceError> {
        let bad = |what: &str| SourceError::new("decode", what);
        let cipher = Decryptor::new_from_slices(&self.key, &self.iv)
            .map_err(|_| bad("the key has the wrong length"))?;
        let mut buffer = body.to_vec();
        let plain = cipher
            .decrypt_padded::<Pkcs7>(&mut buffer)
            .map_err(|_| bad("the body is not encrypted with the configured key"))?;
        rmp_serde::from_slice(plain).map_err(|e| SourceError::new("decode", e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hexadecimal_is_read_in_pairs() {
        assert_eq!(parse_hex("00ff10"), Some(vec![0, 255, 16]));
        assert_eq!(parse_hex("0"), None);
        assert_eq!(parse_hex("zz"), None);
    }

    #[test]
    fn a_missing_secret_is_named() {
        let error = Secrets::default().read("SEKAI_TEST_NOT_SET").unwrap_err();
        assert_eq!(error.code, "secret");
        assert!(error.message.contains("SEKAI_TEST_NOT_SET"));
    }

    #[test]
    fn a_bad_value_is_not_repeated_in_the_error() {
        let directory = std::env::temp_dir().join(format!("sekai-secret-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join("SEKAI_TEST_BAD"), "not-hex-value").unwrap();
        let error = Secrets::new(Some(directory.clone()))
            .read("SEKAI_TEST_BAD")
            .unwrap_err();
        assert!(!error.message.contains("not-hex-value"));
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn the_hca_key_is_a_hexadecimal_number() {
        let directory = std::env::temp_dir().join(format!("sekai-hca-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(
            directory.join("SEKAI_TEST_HCA"),
            " 0123456789ABCDE
",
        )
        .unwrap();
        std::fs::write(directory.join("SEKAI_TEST_HCA_LONG"), "0123456789abcdef0").unwrap();
        std::fs::write(directory.join("SEKAI_TEST_HCA_BAD"), "0x12").unwrap();
        let secrets = Secrets::new(Some(directory.clone()));
        let names = |hca: Option<&str>| SecretNames {
            key: String::new(),
            iv: String::new(),
            hca: hca.map(str::to_owned),
            account: None,
        };
        assert_eq!(secrets.hca_key(&names(None)).unwrap(), None);
        assert_eq!(
            secrets.hca_key(&names(Some("SEKAI_TEST_HCA"))).unwrap(),
            Some(0x0123456789ABCDE)
        );
        for name in ["SEKAI_TEST_HCA_LONG", "SEKAI_TEST_HCA_BAD"] {
            let error = secrets.hca_key(&names(Some(name))).unwrap_err();
            assert_eq!(error.code, "secret");
            assert!(error.message.contains(name));
            assert!(!error.message.contains("0x12"));
        }
        let error = secrets
            .hca_key(&names(Some("SEKAI_TEST_HCA_NOT_SET")))
            .unwrap_err();
        assert!(error.message.contains("is not set"));
        std::fs::remove_dir_all(directory).unwrap();
    }
}

use aes_gcm::{
    Aes256Gcm, Nonce as AesNonce,
    aead::{Aead as _, KeyInit as _, Payload},
};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_types::{
    JsonSchema, Value, async_trait, bail,
    json::json,
    rand::{TryRngCore, rngs::OsRng},
};
use serde::{Deserialize, Serialize};
use zeroize::Zeroize;

use flow_like_catalog_std_support::json::normalize_json_value;

const KEY_LEN: usize = 32;
const AES_GCM_NONCE_LEN: usize = 12;
const XCHACHA20_NONCE_LEN: usize = 24;
const ENCRYPTED_BYTES_VERSION: u8 = 2;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
pub enum EncryptionAlgorithm {
    #[serde(rename = "aes-256-gcm")]
    Aes256Gcm,
    #[serde(rename = "xchacha20-poly1305")]
    XChaCha20Poly1305,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct EncryptedBytes {
    pub version: u8,
    pub algorithm: EncryptionAlgorithm,
    pub nonce: Vec<u8>,
    pub associated_data: Value,
    pub ciphertext: Vec<u8>,
}

#[crate::register_node]
#[derive(Default)]
pub struct GenerateEncryptionKeyNode;

impl GenerateEncryptionKeyNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for GenerateEncryptionKeyNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "crypto_generate_key",
            "Generate Encryption Key",
            "Generates a 256-bit symmetric key for AES-256-GCM and XChaCha20-Poly1305.",
            "Utils/Crypto",
        );
        node.set_flowscript_name("crypto", "generateKey");
        node.add_icon("/flow/icons/key.svg");
        set_crypto_scores(&mut node);

        node.add_input_pin(
            "exec_in",
            "Execute",
            "Generate a new random key",
            VariableType::Execution,
        );
        node.add_output_pin(
            "exec_out",
            "Done",
            "Fires after the key is generated",
            VariableType::Execution,
        );
        node.add_output_pin(
            "key",
            "Key",
            "Random 32-byte symmetric key",
            VariableType::Byte,
        )
        .set_value_type(ValueType::Array)
        .set_options(PinOptions::new().set_sensitive(true).build());

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        let key = random_vec(KEY_LEN)?;
        context.set_pin_value("key", json!(key)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct AesEncryptBytesNode;

impl AesEncryptBytesNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for AesEncryptBytesNode {
    fn get_node(&self) -> Node {
        let mut node = encrypt_node(
            "crypto_aes_encrypt_bytes",
            "AES-256-GCM Encrypt",
            "Encrypts bytes with AES-256-GCM. A fresh nonce is generated internally for every encryption.",
        );
        node.set_flowscript_name("crypto", "aesEncryptBytes");
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        run_encrypt(context, EncryptionAlgorithm::Aes256Gcm).await
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct AesDecryptBytesNode;

impl AesDecryptBytesNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for AesDecryptBytesNode {
    fn get_node(&self) -> Node {
        let mut node = decrypt_node(
            "crypto_aes_decrypt_bytes",
            "AES-256-GCM Decrypt",
            "Decrypts an AES-256-GCM encrypted payload and verifies its authentication tag.",
        );
        node.set_flowscript_name("crypto", "aesDecryptBytes");
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        run_decrypt(context, EncryptionAlgorithm::Aes256Gcm).await
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct AesEncryptValueNode;

impl AesEncryptValueNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for AesEncryptValueNode {
    fn get_node(&self) -> Node {
        let mut node = encrypt_struct_node(
            "crypto_aes_encrypt_value",
            "AES-256-GCM Encrypt Value",
            "Serializes and encrypts a struct with AES-256-GCM. A fresh nonce is generated internally for every encryption.",
        );
        node.set_flowscript_name("crypto", "aesEncryptValue");
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        run_encrypt_struct(context, EncryptionAlgorithm::Aes256Gcm).await
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct AesDecryptValueNode;

impl AesDecryptValueNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for AesDecryptValueNode {
    fn get_node(&self) -> Node {
        let mut node = decrypt_struct_node(
            "crypto_aes_decrypt_value",
            "AES-256-GCM Decrypt Value",
            "Decrypts an AES-256-GCM payload and parses the plaintext as a struct.",
        );
        node.set_flowscript_name("crypto", "aesDecryptValue");
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        run_decrypt_struct(context, EncryptionAlgorithm::Aes256Gcm).await
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct XChaCha20EncryptBytesNode;

impl XChaCha20EncryptBytesNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for XChaCha20EncryptBytesNode {
    fn get_node(&self) -> Node {
        let mut node = encrypt_node(
            "crypto_xchacha20_encrypt_bytes",
            "XChaCha20-Poly1305 Encrypt",
            "Encrypts bytes with XChaCha20-Poly1305. A fresh 192-bit nonce is generated internally for every encryption.",
        );
        node.set_flowscript_name("crypto", "xchacha20EncryptBytes");
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        run_encrypt(context, EncryptionAlgorithm::XChaCha20Poly1305).await
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct XChaCha20DecryptBytesNode;

impl XChaCha20DecryptBytesNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for XChaCha20DecryptBytesNode {
    fn get_node(&self) -> Node {
        let mut node = decrypt_node(
            "crypto_xchacha20_decrypt_bytes",
            "XChaCha20-Poly1305 Decrypt",
            "Decrypts an XChaCha20-Poly1305 encrypted payload and verifies its authentication tag.",
        );
        node.set_flowscript_name("crypto", "xchacha20DecryptBytes");
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        run_decrypt(context, EncryptionAlgorithm::XChaCha20Poly1305).await
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct XChaCha20EncryptValueNode;

impl XChaCha20EncryptValueNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for XChaCha20EncryptValueNode {
    fn get_node(&self) -> Node {
        let mut node = encrypt_struct_node(
            "crypto_xchacha20_encrypt_value",
            "XChaCha20-Poly1305 Encrypt Value",
            "Serializes and encrypts a struct with XChaCha20-Poly1305. A fresh 192-bit nonce is generated internally for every encryption.",
        );
        node.set_flowscript_name("crypto", "xchacha20EncryptValue");
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        run_encrypt_struct(context, EncryptionAlgorithm::XChaCha20Poly1305).await
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct XChaCha20DecryptValueNode;

impl XChaCha20DecryptValueNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for XChaCha20DecryptValueNode {
    fn get_node(&self) -> Node {
        let mut node = decrypt_struct_node(
            "crypto_xchacha20_decrypt_value",
            "XChaCha20-Poly1305 Decrypt Value",
            "Decrypts an XChaCha20-Poly1305 payload and parses the plaintext as a struct.",
        );
        node.set_flowscript_name("crypto", "xchacha20DecryptValue");
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        run_decrypt_struct(context, EncryptionAlgorithm::XChaCha20Poly1305).await
    }
}

fn encrypt_node(name: &str, friendly_name: &str, description: &str) -> Node {
    let mut node = Node::new(name, friendly_name, description, "Utils/Crypto");
    node.add_icon("/flow/icons/lock.svg");
    set_crypto_scores(&mut node);

    node.add_input_pin(
        "exec_in",
        "Execute",
        "Encrypt the plaintext",
        VariableType::Execution,
    );
    node.add_input_pin("key", "Key", "32-byte symmetric key", VariableType::Byte)
        .set_value_type(ValueType::Array)
        .set_options(PinOptions::new().set_sensitive(true).build());
    node.add_input_pin(
        "plaintext",
        "Plaintext",
        "Bytes to encrypt",
        VariableType::Byte,
    )
    .set_value_type(ValueType::Array)
    .set_options(PinOptions::new().set_sensitive(true).build());
    node.add_input_pin(
        "associated_data",
        "Associated Data",
        "Optional authenticated metadata stored alongside the ciphertext",
        VariableType::Struct,
    )
    .set_default_value(Some(json!({})))
    .set_open_schema();

    node.add_output_pin(
        "exec_out",
        "Done",
        "Fires after encryption succeeds",
        VariableType::Execution,
    );
    node.add_output_pin(
        "encrypted",
        "Encrypted",
        "Authenticated encrypted payload with algorithm and generated nonce",
        VariableType::Struct,
    )
    .set_schema::<EncryptedBytes>()
    .set_options(PinOptions::new().set_enforce_schema(true).build());

    node
}

fn encrypt_struct_node(name: &str, friendly_name: &str, description: &str) -> Node {
    let mut node = Node::new(name, friendly_name, description, "Utils/Crypto");
    node.add_icon("/flow/icons/lock.svg");
    set_crypto_scores(&mut node);

    node.add_input_pin(
        "exec_in",
        "Execute",
        "Encrypt the struct",
        VariableType::Execution,
    );
    node.add_input_pin("key", "Key", "32-byte symmetric key", VariableType::Byte)
        .set_value_type(ValueType::Array)
        .set_options(PinOptions::new().set_sensitive(true).build());
    node.add_input_pin("value", "Value", "Struct to encrypt", VariableType::Struct)
        .set_open_schema()
        .set_options(PinOptions::new().set_sensitive(true).build());
    node.add_input_pin(
        "associated_data",
        "Associated Data",
        "Optional authenticated metadata stored alongside the ciphertext",
        VariableType::Struct,
    )
    .set_default_value(Some(json!({})))
    .set_open_schema();

    node.add_output_pin(
        "exec_out",
        "Done",
        "Fires after encryption succeeds",
        VariableType::Execution,
    );
    node.add_output_pin(
        "encrypted",
        "Encrypted",
        "Authenticated encrypted payload with algorithm and generated nonce",
        VariableType::Struct,
    )
    .set_schema::<EncryptedBytes>()
    .set_options(PinOptions::new().set_enforce_schema(true).build());

    node
}

fn decrypt_node(name: &str, friendly_name: &str, description: &str) -> Node {
    let mut node = Node::new(name, friendly_name, description, "Utils/Crypto");
    node.add_icon("/flow/icons/unlock.svg");
    set_crypto_scores(&mut node);

    node.add_input_pin(
        "exec_in",
        "Execute",
        "Decrypt the encrypted payload",
        VariableType::Execution,
    );
    node.add_input_pin("key", "Key", "32-byte symmetric key", VariableType::Byte)
        .set_value_type(ValueType::Array)
        .set_options(PinOptions::new().set_sensitive(true).build());
    node.add_input_pin(
        "encrypted",
        "Encrypted",
        "Authenticated encrypted payload",
        VariableType::Struct,
    )
    .set_schema::<EncryptedBytes>()
    .set_options(PinOptions::new().set_enforce_schema(true).build());

    node.add_output_pin(
        "exec_out",
        "Done",
        "Fires after decryption succeeds",
        VariableType::Execution,
    );
    node.add_output_pin(
        "plaintext",
        "Plaintext",
        "Decrypted bytes",
        VariableType::Byte,
    )
    .set_value_type(ValueType::Array)
    .set_options(PinOptions::new().set_sensitive(true).build());

    node
}

fn decrypt_struct_node(name: &str, friendly_name: &str, description: &str) -> Node {
    let mut node = Node::new(name, friendly_name, description, "Utils/Crypto");
    node.add_icon("/flow/icons/unlock.svg");
    set_crypto_scores(&mut node);

    node.add_input_pin(
        "exec_in",
        "Execute",
        "Decrypt the encrypted payload",
        VariableType::Execution,
    );
    node.add_input_pin("key", "Key", "32-byte symmetric key", VariableType::Byte)
        .set_value_type(ValueType::Array)
        .set_options(PinOptions::new().set_sensitive(true).build());
    node.add_input_pin(
        "encrypted",
        "Encrypted",
        "Authenticated encrypted payload",
        VariableType::Struct,
    )
    .set_schema::<EncryptedBytes>()
    .set_options(PinOptions::new().set_enforce_schema(true).build());

    node.add_output_pin(
        "exec_out",
        "Done",
        "Fires after decryption succeeds",
        VariableType::Execution,
    );
    node.add_output_pin("value", "Value", "Decrypted struct", VariableType::Struct)
        .set_open_schema()
        .set_options(PinOptions::new().set_sensitive(true).build());

    node
}

async fn run_encrypt(
    context: &mut ExecutionContext,
    algorithm: EncryptionAlgorithm,
) -> flow_like_types::Result<()> {
    context.deactivate_exec_pin("exec_out").await?;
    let mut key: Vec<u8> = context.evaluate_pin("key").await?;
    let mut key_array = key_from_pin(&mut key)?;

    let mut plaintext: Vec<u8> = match context.evaluate_pin("plaintext").await {
        Ok(value) => value,
        Err(err) => {
            key_array.zeroize();
            key.zeroize();
            return Err(err);
        }
    };
    let associated_data: Value = match context.evaluate_pin("associated_data").await {
        Ok(value) => value,
        Err(err) => {
            key_array.zeroize();
            key.zeroize();
            plaintext.zeroize();
            return Err(err);
        }
    };

    let result = encrypt_bytes(algorithm, &key_array, &plaintext, associated_data);
    key_array.zeroize();
    key.zeroize();
    plaintext.zeroize();

    context.set_pin_value("encrypted", json!(result?)).await?;
    context.activate_exec_pin("exec_out").await?;
    Ok(())
}

async fn run_encrypt_struct(
    context: &mut ExecutionContext,
    algorithm: EncryptionAlgorithm,
) -> flow_like_types::Result<()> {
    context.deactivate_exec_pin("exec_out").await?;
    let mut key: Vec<u8> = context.evaluate_pin("key").await?;
    let mut key_array = key_from_pin(&mut key)?;

    let value: Value = match context.evaluate_pin("value").await {
        Ok(value) => value,
        Err(err) => {
            key_array.zeroize();
            key.zeroize();
            return Err(err);
        }
    };
    let associated_data: Value = match context.evaluate_pin("associated_data").await {
        Ok(value) => value,
        Err(err) => {
            key_array.zeroize();
            key.zeroize();
            return Err(err);
        }
    };

    let mut plaintext = match serialize_struct_value(value) {
        Ok(value) => value,
        Err(err) => {
            key_array.zeroize();
            key.zeroize();
            return Err(err);
        }
    };
    let result = encrypt_bytes(algorithm, &key_array, &plaintext, associated_data);
    key_array.zeroize();
    key.zeroize();
    plaintext.zeroize();

    context.set_pin_value("encrypted", json!(result?)).await?;
    context.activate_exec_pin("exec_out").await?;
    Ok(())
}

async fn run_decrypt(
    context: &mut ExecutionContext,
    algorithm: EncryptionAlgorithm,
) -> flow_like_types::Result<()> {
    context.deactivate_exec_pin("exec_out").await?;
    let mut key: Vec<u8> = context.evaluate_pin("key").await?;
    let mut key_array = key_from_pin(&mut key)?;

    let encrypted: EncryptedBytes = match context.evaluate_pin("encrypted").await {
        Ok(value) => value,
        Err(err) => {
            key_array.zeroize();
            key.zeroize();
            return Err(err);
        }
    };

    let result = decrypt_bytes(algorithm, &key_array, &encrypted);
    key_array.zeroize();
    key.zeroize();

    context.set_pin_value("plaintext", json!(result?)).await?;
    context.activate_exec_pin("exec_out").await?;
    Ok(())
}

async fn run_decrypt_struct(
    context: &mut ExecutionContext,
    algorithm: EncryptionAlgorithm,
) -> flow_like_types::Result<()> {
    context.deactivate_exec_pin("exec_out").await?;
    let mut key: Vec<u8> = context.evaluate_pin("key").await?;
    let mut key_array = key_from_pin(&mut key)?;

    let encrypted: EncryptedBytes = match context.evaluate_pin("encrypted").await {
        Ok(value) => value,
        Err(err) => {
            key_array.zeroize();
            key.zeroize();
            return Err(err);
        }
    };

    let mut plaintext = match decrypt_bytes(algorithm, &key_array, &encrypted) {
        Ok(value) => value,
        Err(err) => {
            key_array.zeroize();
            key.zeroize();
            return Err(err);
        }
    };
    key_array.zeroize();
    key.zeroize();

    let value = deserialize_struct_value(&plaintext);
    plaintext.zeroize();

    context.set_pin_value("value", value?).await?;
    context.activate_exec_pin("exec_out").await?;
    Ok(())
}

fn encrypt_bytes(
    algorithm: EncryptionAlgorithm,
    key: &[u8; KEY_LEN],
    plaintext: &[u8],
    associated_data: Value,
) -> flow_like_types::Result<EncryptedBytes> {
    let associated_data = normalize_json_value(associated_data);
    let associated_data_bytes = serialize_struct_ref(&associated_data)?;
    let (nonce, ciphertext) = match algorithm {
        EncryptionAlgorithm::Aes256Gcm => {
            let nonce = random_vec(AES_GCM_NONCE_LEN)?;
            let ciphertext = aes_encrypt(key, &nonce, plaintext, &associated_data_bytes)?;
            (nonce, ciphertext)
        }
        EncryptionAlgorithm::XChaCha20Poly1305 => {
            let nonce = random_vec(XCHACHA20_NONCE_LEN)?;
            let ciphertext = xchacha20_encrypt(key, &nonce, plaintext, &associated_data_bytes)?;
            (nonce, ciphertext)
        }
    };

    Ok(EncryptedBytes {
        version: ENCRYPTED_BYTES_VERSION,
        algorithm,
        nonce,
        associated_data,
        ciphertext,
    })
}

fn decrypt_bytes(
    expected_algorithm: EncryptionAlgorithm,
    key: &[u8; KEY_LEN],
    encrypted: &EncryptedBytes,
) -> flow_like_types::Result<Vec<u8>> {
    if encrypted.version != ENCRYPTED_BYTES_VERSION {
        bail!(
            "Unsupported encrypted payload version: {}",
            encrypted.version
        );
    }
    if encrypted.algorithm != expected_algorithm {
        bail!(
            "Encrypted payload uses {:?}, but this node expects {:?}",
            encrypted.algorithm,
            expected_algorithm
        );
    }

    match encrypted.algorithm {
        EncryptionAlgorithm::Aes256Gcm => {
            ensure_nonce_len(&encrypted.nonce, AES_GCM_NONCE_LEN)?;
            let associated_data = serialize_struct_ref(&encrypted.associated_data)?;
            aes_decrypt(
                key,
                &encrypted.nonce,
                &encrypted.ciphertext,
                &associated_data,
            )
        }
        EncryptionAlgorithm::XChaCha20Poly1305 => {
            ensure_nonce_len(&encrypted.nonce, XCHACHA20_NONCE_LEN)?;
            let associated_data = serialize_struct_ref(&encrypted.associated_data)?;
            xchacha20_decrypt(
                key,
                &encrypted.nonce,
                &encrypted.ciphertext,
                &associated_data,
            )
        }
    }
}

fn serialize_struct_value(value: Value) -> flow_like_types::Result<Vec<u8>> {
    let normalized = normalize_json_value(value);
    serialize_struct_ref(&normalized)
}

fn serialize_struct_ref(value: &Value) -> flow_like_types::Result<Vec<u8>> {
    let normalized = normalize_json_value(value.clone());
    Ok(flow_like_types::json::to_vec(&normalized)?)
}

fn deserialize_struct_value(bytes: &[u8]) -> flow_like_types::Result<Value> {
    let value: Value = flow_like_types::json::from_slice(bytes)?;
    Ok(normalize_json_value(value))
}

fn aes_encrypt(
    key: &[u8; KEY_LEN],
    nonce: &[u8],
    plaintext: &[u8],
    associated_data: &[u8],
) -> flow_like_types::Result<Vec<u8>> {
    let cipher = Aes256Gcm::new_from_slice(key)
        .map_err(|_| flow_like_types::anyhow!("Invalid AES-256-GCM key"))?;
    cipher
        .encrypt(
            AesNonce::from_slice(nonce),
            Payload {
                msg: plaintext,
                aad: associated_data,
            },
        )
        .map_err(|_| flow_like_types::anyhow!("AES-256-GCM encryption failed"))
}

fn aes_decrypt(
    key: &[u8; KEY_LEN],
    nonce: &[u8],
    ciphertext: &[u8],
    associated_data: &[u8],
) -> flow_like_types::Result<Vec<u8>> {
    let cipher = Aes256Gcm::new_from_slice(key)
        .map_err(|_| flow_like_types::anyhow!("Invalid AES-256-GCM key"))?;
    cipher
        .decrypt(
            AesNonce::from_slice(nonce),
            Payload {
                msg: ciphertext,
                aad: associated_data,
            },
        )
        .map_err(|_| {
            flow_like_types::anyhow!(
                "AES-256-GCM decryption failed: wrong key, corrupted ciphertext, or altered associated data"
            )
        })
}

fn xchacha20_encrypt(
    key: &[u8; KEY_LEN],
    nonce: &[u8],
    plaintext: &[u8],
    associated_data: &[u8],
) -> flow_like_types::Result<Vec<u8>> {
    let cipher = XChaCha20Poly1305::new_from_slice(key)
        .map_err(|_| flow_like_types::anyhow!("Invalid XChaCha20-Poly1305 key"))?;
    cipher
        .encrypt(
            XNonce::from_slice(nonce),
            Payload {
                msg: plaintext,
                aad: associated_data,
            },
        )
        .map_err(|_| flow_like_types::anyhow!("XChaCha20-Poly1305 encryption failed"))
}

fn xchacha20_decrypt(
    key: &[u8; KEY_LEN],
    nonce: &[u8],
    ciphertext: &[u8],
    associated_data: &[u8],
) -> flow_like_types::Result<Vec<u8>> {
    let cipher = XChaCha20Poly1305::new_from_slice(key)
        .map_err(|_| flow_like_types::anyhow!("Invalid XChaCha20-Poly1305 key"))?;
    cipher
        .decrypt(
            XNonce::from_slice(nonce),
            Payload {
                msg: ciphertext,
                aad: associated_data,
            },
        )
        .map_err(|_| {
            flow_like_types::anyhow!(
                "XChaCha20-Poly1305 decryption failed: wrong key, corrupted ciphertext, or altered associated data"
            )
        })
}

fn key_from_pin(key: &mut [u8]) -> flow_like_types::Result<[u8; KEY_LEN]> {
    if key.len() != KEY_LEN {
        key.zeroize();
        bail!(
            "Encryption key must be exactly {} bytes for AES-256-GCM and XChaCha20-Poly1305",
            KEY_LEN
        );
    }

    let mut key_array = [0u8; KEY_LEN];
    key_array.copy_from_slice(key);
    Ok(key_array)
}

fn ensure_nonce_len(nonce: &[u8], expected: usize) -> flow_like_types::Result<()> {
    if nonce.len() != expected {
        bail!(
            "Invalid nonce length: expected {} bytes, got {}",
            expected,
            nonce.len()
        );
    }
    Ok(())
}

fn random_vec(len: usize) -> flow_like_types::Result<Vec<u8>> {
    let mut bytes = vec![0u8; len];
    let mut rng = OsRng;
    rng.try_fill_bytes(&mut bytes)?;
    Ok(bytes)
}

fn set_crypto_scores(node: &mut Node) {
    node.set_scores(
        NodeScores::new()
            .set_privacy(10)
            .set_security(10)
            .set_performance(8)
            .set_governance(8)
            .set_reliability(9)
            .set_cost(10)
            .build(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_keys_are_32_bytes() {
        assert_eq!(random_vec(KEY_LEN).unwrap().len(), KEY_LEN);
    }

    #[test]
    fn aes_roundtrip_and_nonce_randomization() {
        let key = [7u8; KEY_LEN];
        let plaintext = b"flow-like secret bytes";
        let aad = json!({"node_id": "123"});

        let encrypted =
            encrypt_bytes(EncryptionAlgorithm::Aes256Gcm, &key, plaintext, aad.clone()).unwrap();
        let encrypted_again =
            encrypt_bytes(EncryptionAlgorithm::Aes256Gcm, &key, plaintext, aad).unwrap();

        assert_eq!(encrypted.nonce.len(), AES_GCM_NONCE_LEN);
        assert_ne!(encrypted.nonce, encrypted_again.nonce);
        assert_ne!(encrypted.ciphertext, plaintext);
        assert_eq!(
            decrypt_bytes(EncryptionAlgorithm::Aes256Gcm, &key, &encrypted).unwrap(),
            plaintext
        );
    }

    #[test]
    fn xchacha20_roundtrip_and_nonce_randomization() {
        let key = [9u8; KEY_LEN];
        let plaintext = b"flow-like secret bytes";

        let encrypted = encrypt_bytes(
            EncryptionAlgorithm::XChaCha20Poly1305,
            &key,
            plaintext,
            json!({}),
        )
        .unwrap();
        let encrypted_again = encrypt_bytes(
            EncryptionAlgorithm::XChaCha20Poly1305,
            &key,
            plaintext,
            json!({}),
        )
        .unwrap();

        assert_eq!(encrypted.nonce.len(), XCHACHA20_NONCE_LEN);
        assert_ne!(encrypted.nonce, encrypted_again.nonce);
        assert_ne!(encrypted.ciphertext, plaintext);
        assert_eq!(
            decrypt_bytes(EncryptionAlgorithm::XChaCha20Poly1305, &key, &encrypted).unwrap(),
            plaintext
        );
    }

    #[test]
    fn decrypt_rejects_wrong_algorithm_node() {
        let key = [3u8; KEY_LEN];
        let encrypted =
            encrypt_bytes(EncryptionAlgorithm::Aes256Gcm, &key, b"secret", json!({})).unwrap();

        assert!(decrypt_bytes(EncryptionAlgorithm::XChaCha20Poly1305, &key, &encrypted).is_err());
    }

    #[test]
    fn decrypt_rejects_altered_associated_data() {
        let key = [4u8; KEY_LEN];
        let mut encrypted = encrypt_bytes(
            EncryptionAlgorithm::XChaCha20Poly1305,
            &key,
            b"secret",
            json!({"context": "original"}),
        )
        .unwrap();

        encrypted.associated_data = json!({"context": "other"});

        assert!(decrypt_bytes(EncryptionAlgorithm::XChaCha20Poly1305, &key, &encrypted).is_err());
    }

    #[test]
    fn struct_payload_roundtrip_uses_canonical_serialization() {
        let key = [5u8; KEY_LEN];
        let value = json!({
            "z": [3, 2, 1],
            "a": {
                "b": true,
                "a": "first"
            }
        });
        let associated_data = json!({
            "purpose": "unit-test",
            "version": 1
        });

        let plaintext = serialize_struct_value(value.clone()).unwrap();
        let encrypted = encrypt_bytes(
            EncryptionAlgorithm::Aes256Gcm,
            &key,
            &plaintext,
            associated_data.clone(),
        )
        .unwrap();
        let decrypted_plaintext =
            decrypt_bytes(EncryptionAlgorithm::Aes256Gcm, &key, &encrypted).unwrap();
        let decrypted_value = deserialize_struct_value(&decrypted_plaintext).unwrap();

        assert_eq!(
            encrypted.associated_data,
            normalize_json_value(associated_data)
        );
        assert_eq!(decrypted_value, normalize_json_value(value));
    }
}

use hmac::{Hmac, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

pub fn sign_hmac_sha256_hex(secret_key: &str, payload: &str) -> String {
    let mut mac = HmacSha256::new_from_slice(secret_key.as_bytes()).expect("invalid hmac key");
    mac.update(payload.as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

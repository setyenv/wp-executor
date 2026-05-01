use hmac::{Hmac, Mac};
use sha2::Sha256;

/// HMAC-SHA256 of `body` using `bearer_token` as key. Returns the
/// `sha256=<hex>` string the upstream expects in the `X-PFW-Signature`
/// header. See RemoteContractHelper::auth.hmac.
pub fn sign_body(bearer_token: &str, body: &[u8]) -> String {
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(bearer_token.as_bytes())
        .expect("HMAC accepts any key length");
    mac.update(body);
    let digest = mac.finalize().into_bytes();
    format!("sha256={}", hex::encode(digest))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signature_format_is_sha256_prefix_plus_hex() {
        let sig = sign_body("token", b"hello");
        assert!(sig.starts_with("sha256="));
        let hex = &sig["sha256=".len()..];
        assert_eq!(hex.len(), 64); // 32 bytes hex-encoded
    }

    #[test]
    fn known_vector_matches_php_hash_hmac() {
        // Cross-verified against PHP: hash_hmac('sha256', 'hello', 'token').
        // This guarantees byte-for-byte parity with the upstream's
        // RemoteWorkerHelper::sign_body() so the signature header validates.
        let sig = sign_body("token", b"hello");
        assert_eq!(
            sig,
            "sha256=df3178e409a68446314d5be83b911b78dc6fa272a556429fd9d2092575dcf174"
        );
    }

    #[test]
    fn empty_body_signs_deterministically() {
        let sig1 = sign_body("k", b"");
        let sig2 = sign_body("k", b"");
        assert_eq!(sig1, sig2);
    }
}

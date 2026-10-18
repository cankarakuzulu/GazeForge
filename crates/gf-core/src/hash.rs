use serde_json::Value;
use sha2::{Digest, Sha256};

pub const PREFIX: &str = "sha256:";

pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

pub fn reference(hex: &str) -> String {
    format!("{PREFIX}{hex}")
}

pub fn parse_reference(text: &str) -> Result<String, String> {
    let hex = text.strip_prefix(PREFIX).ok_or_else(|| format!("{text} is not a {PREFIX}<64 hex> reference"))?;
    if hex.len() != 64 || !hex.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
        return Err(format!("{text} is not a {PREFIX}<64 lowercase hex> reference"));
    }
    Ok(hex.to_string())
}

pub fn canonical_bytes(value: &Value) -> Vec<u8> {
    serde_json::to_vec(value).expect("a JSON value always serialises")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_matches_the_known_vector() {
        assert_eq!(sha256_hex(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }

    #[test]
    fn references_round_trip_and_bad_ones_are_refused() {
        let hex = sha256_hex(b"");
        assert_eq!(parse_reference(&reference(&hex)).unwrap(), hex);
        assert!(parse_reference(&hex).is_err());
        assert!(parse_reference("sha256:ABC").is_err());
        assert!(parse_reference(&format!("sha256:{}", hex.to_uppercase())).is_err());
    }

    #[test]
    fn canonical_bytes_sort_keys() {
        let value: Value = serde_json::from_str(r#"{"b":1,"a":{"d":2,"c":3}}"#).unwrap();
        assert_eq!(canonical_bytes(&value), br#"{"a":{"c":3,"d":2},"b":1}"#);
    }
}

//! Pack an action plus arguments into Telegram's 64-byte callback data,
//! optionally HMAC-signed so users cannot forge button payloads.

use crate::crypto::{ct_eq, hex, hmac_sha256};
use crate::error::{Error, Result};

/// Telegram's limit for callback data.
pub const CALLBACK_DATA_MAX_BYTES: usize = 64;
const HMAC_BYTES: usize = 12;

fn payload(action: &str, args: &[&str]) -> Result<String> {
    if action.is_empty() {
        return Err(Error::CallbackInvalid);
    }
    if action.contains(':') || args.iter().any(|a| a.contains(':')) {
        return Err(Error::CallbackColon);
    }
    let mut out = String::from(action);
    for a in args {
        out.push(':');
        out.push_str(a);
    }
    Ok(out)
}

fn mac(secret: &[u8], payload: &str) -> String {
    let sum = hmac_sha256(secret, payload.as_bytes());
    hex(&sum[..HMAC_BYTES])
}

/// `action:arg1:arg2`, at most 64 bytes.
pub fn pack_callback(action: &str, args: &[&str]) -> Result<String> {
    let p = payload(action, args)?;
    if p.len() > CALLBACK_DATA_MAX_BYTES {
        return Err(Error::CallbackTooLong);
    }
    Ok(p)
}

/// Inverse of [`pack_callback`].
pub fn unpack_callback(data: &str) -> Result<(String, Vec<String>)> {
    if data.is_empty() {
        return Err(Error::CallbackInvalid);
    }
    let mut parts = data.split(':');
    let action = parts.next().unwrap_or("");
    if action.is_empty() {
        return Err(Error::CallbackInvalid);
    }
    Ok((action.to_string(), parts.map(str::to_string).collect()))
}

/// `action:arg1:...:mac` where `mac` is a truncated HMAC-SHA256 of the rest.
pub fn pack_callback_signed(secret: &[u8], action: &str, args: &[&str]) -> Result<String> {
    if secret.is_empty() {
        return Err(Error::NoCallbackSecret);
    }
    let p = payload(action, args)?;
    let packed = format!("{p}:{}", mac(secret, &p));
    if packed.len() > CALLBACK_DATA_MAX_BYTES {
        return Err(Error::CallbackTooLong);
    }
    Ok(packed)
}

/// Verify and unpack data produced by [`pack_callback_signed`].
pub fn unpack_callback_signed(secret: &[u8], data: &str) -> Result<(String, Vec<String>)> {
    if secret.is_empty() {
        return Err(Error::NoCallbackSecret);
    }
    if data.is_empty() {
        return Err(Error::CallbackInvalid);
    }
    let idx = match data.rfind(':') {
        Some(i) if i > 0 && i < data.len() - 1 => i,
        _ => return Err(Error::CallbackBadHmac),
    };
    let (p, got) = (&data[..idx], &data[idx + 1..]);
    if !ct_eq(got.as_bytes(), mac(secret, p).as_bytes()) {
        return Err(Error::CallbackBadHmac);
    }
    unpack_callback(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain() {
        let d = pack_callback("play", &["skip", "7"]).unwrap();
        assert_eq!(d, "play:skip:7");
        assert_eq!(unpack_callback(&d).unwrap(), ("play".into(), vec!["skip".into(), "7".into()]));
        assert_eq!(unpack_callback("x").unwrap(), ("x".into(), vec![]));
        assert_eq!(pack_callback("a:b", &[]), Err(Error::CallbackColon));
        assert_eq!(pack_callback("a", &["x:y"]), Err(Error::CallbackColon));
        assert_eq!(pack_callback("", &[]), Err(Error::CallbackInvalid));
        assert_eq!(pack_callback("a", &[&"x".repeat(70)]), Err(Error::CallbackTooLong));
    }

    #[test]
    fn signed() {
        let s = b"secret";
        let d = pack_callback_signed(s, "ban", &["42"]).unwrap();
        assert!(d.len() <= 64);
        assert_eq!(unpack_callback_signed(s, &d).unwrap(), ("ban".into(), vec!["42".into()]));
        assert_eq!(unpack_callback_signed(b"other", &d), Err(Error::CallbackBadHmac));
        let forged = d.replacen("42", "43", 1);
        assert_eq!(unpack_callback_signed(s, &forged), Err(Error::CallbackBadHmac));
        assert_eq!(unpack_callback_signed(b"", &d), Err(Error::NoCallbackSecret));
        assert_eq!(unpack_callback_signed(s, "nomac:"), Err(Error::CallbackBadHmac));
    }
}

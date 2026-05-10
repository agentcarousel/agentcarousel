//! Lowercase hex encoding for digest outputs (`GenericArray` is not `LowerHex`).
use std::fmt::Write as _;

pub(crate) fn hex_lower(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        write!(&mut s, "{:02x}", b).expect("write to String cannot fail");
    }
    s
}

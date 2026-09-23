//! Percent-escaping for the Assuan wire format.
//!
//! Three distinct rules, and mixing them up corrupts data: command arguments
//! decode `%XX` only (`+` is literal), `D` lines escape `%`/CR/LF, and
//! `INQUIRE` payloads additionally use `+` for space.

/// Invalid escapes pass through literally, as libassuan's `strcpy_escaped` does.
pub fn decode_argument(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            match (hex_val(bytes[i + 1]), hex_val(bytes[i + 2])) {
                (Some(hi), Some(lo)) => {
                    out.push((hi << 4) | lo);
                    i += 3;
                    continue;
                }
                _ => {
                    out.push(bytes[i]);
                    i += 1;
                    continue;
                }
            }
        }
        out.push(bytes[i]);
        i += 1;
    }

    // Lossy rather than dropping the line if a client sends non-UTF-8.
    String::from_utf8_lossy(&out).into_owned()
}

/// Returns bytes, not a `String`: building one by pushing `b as char` would
/// reinterpret each byte as a codepoint and mangle non-ASCII passphrases.
pub fn encode_data(input: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(input.len());
    for &b in input {
        match b {
            b'%' => out.extend_from_slice(b"%25"),
            b'\r' => out.extend_from_slice(b"%0D"),
            b'\n' => out.extend_from_slice(b"%0A"),
            _ => out.push(b),
        }
    }
    out
}

/// Space is encoded as `+`, so `+` itself must be escaped.
pub fn encode_inquire(input: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(input.len());
    for &b in input {
        if b < b' ' || b == b'+' || b == b'%' {
            out.extend_from_slice(format!("%{b:02X}").as_bytes());
        } else if b == b' ' {
            out.push(b'+');
        } else {
            out.push(b);
        }
    }
    out
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_percent_escapes() {
        assert_eq!(decode_argument("a%20b"), "a b");
        assert_eq!(decode_argument("100%25"), "100%");
        assert_eq!(decode_argument("line%0Abreak"), "line\nbreak");
    }

    #[test]
    fn plus_is_literal_in_command_arguments() {
        // Unlike inquire payloads, `+` carries no special meaning here.
        assert_eq!(decode_argument("a+b"), "a+b");
    }

    #[test]
    fn malformed_escapes_pass_through() {
        assert_eq!(decode_argument("100%"), "100%");
        assert_eq!(decode_argument("%zz"), "%zz");
        assert_eq!(decode_argument("%2"), "%2");
    }

    #[test]
    fn data_escaping_protects_line_framing() {
        assert_eq!(encode_data(b"ab"), b"ab");
        assert_eq!(encode_data(b"a%b"), b"a%25b");
        assert_eq!(encode_data(b"a\nb"), b"a%0Ab");
        assert_eq!(encode_data(b"a\rb"), b"a%0Db");
    }

    #[test]
    fn data_escaping_leaves_spaces_and_plus_alone() {
        // A passphrase containing a space must survive verbatim on a D line.
        assert_eq!(encode_data(b"a b+c"), b"a b+c");
    }

    #[test]
    fn data_escaping_preserves_multibyte_utf8() {
        // Regression: escaping used to widen each byte into a codepoint,
        // turning "ü" (0xC3 0xBC) into "Ã¼" on the wire.
        assert_eq!(encode_data("ü".as_bytes()), vec![0xC3, 0xBC]);
    }

    #[test]
    fn inquire_encoding_uses_plus_for_space() {
        assert_eq!(encode_inquire(b"a b"), b"a+b");
        assert_eq!(encode_inquire(b"a+b"), b"a%2Bb");
        assert_eq!(encode_inquire(b"a%b"), b"a%25b");
        assert_eq!(encode_inquire(b"a\nb"), b"a%0Ab");
    }

    #[test]
    fn round_trips_through_decode() {
        for original in ["hunter2", "with space", "100% sure", "tab\there", "ünïcødé"] {
            let encoded = encode_data(original.as_bytes());
            let as_text = String::from_utf8(encoded).expect("escaped form stays utf-8");
            assert_eq!(decode_argument(&as_text), original);
        }
    }
}

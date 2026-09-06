pub fn string_to_bool(str:&str) -> bool {
    match str.to_lowercase().trim() {
        "y" |
        "yes"|
        "true" => true,
        "n" |
        "no" |
        "false" => false,
        _ => false,
    }
}

pub fn generate_escape_string(s: &str) -> String {
    let mut result = String::new();
    for c in s.chars() {
        match c {
            '\n' => result.push_str("\\n"),
            '\r' => result.push_str("\\r"),
            '\t' => result.push_str("\\t"),
            '\\' => result.push_str("\\\\"),
            '\"' => result.push_str("\\\""),
            '\'' => result.push_str("\\'"),
            _ => result.push(c),
        }
    }
    result
}

/// GNU ELF version hash function used for strings like "GLIBC_2.2.5".
pub fn vna_hash(version: &str) -> u32 {
    let mut h: u32 = 0;
    for b in version.bytes() {
        h = h.wrapping_shl(4).wrapping_add(b as u32);
        let g = h & 0xF0000000;
        if g != 0 {
            h ^= g.wrapping_shr(24);
        }
        h &= !g;
    }

    h
}

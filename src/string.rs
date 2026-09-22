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

/// Sanitizes a string to be a valid symbol by replacing non-alphanumeric characters with underscores.
/// If the resulting string is empty, it returns "sym".
/// If the resulting string starts with a digit, it prepends an underscore.
pub fn sanitize_symbol(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for c in input.chars() {
        if c.is_ascii_alphanumeric() || c == '_' {
            out.push(c);
        } else {
            out.push('_');
        }
    }

    if out.is_empty() {
        return "sym".to_string();
    }

    if out.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        format!("_{}", out)
    } else {
        out
    }
}

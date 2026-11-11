//! Looking at bytes.

// @listing hex-dump
/// `bytes` as text: sixteen to a line, in hexadecimal and as characters. A run of
/// identical lines is printed once, followed by `*`.
pub fn hex_dump(bytes: &[u8]) -> String {
    let mut out = String::new();
    let mut previous: Option<&[u8]> = None;
    let mut skipping = false;
    for (i, row) in bytes.chunks(16).enumerate() {
        if previous == Some(row) {
            if !skipping {
                out.push_str("*\n");
            }
            skipping = true;
            continue;
        }
        (previous, skipping) = (Some(row), false);
        let hex: Vec<String> = row.iter().map(|b| format!("{b:02x}")).collect();
        let text: String = row.iter().map(|&b| printable(b)).collect();
        out.push_str(&format!("{:04x}  {:<47}  |{text}|\n", i * 16, hex.join(" ")));
    }
    out
}

fn printable(b: u8) -> char {
    if b == b' ' || b.is_ascii_graphic() { b as char } else { '.' }
}
// @end

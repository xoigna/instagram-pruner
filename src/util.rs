pub fn truncate(s: &str, max: usize) -> String {
    let chars: String = s
        .chars()
        .filter(|c| !c.is_control() && *c != '\u{200b}')
        .collect();
    if chars.chars().count() <= max {
        chars
    } else {
        let mut out: String = chars.chars().take(max).collect();
        out.push('\u{2026}');
        out
    }
}

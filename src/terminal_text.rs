/// Removes C0, DEL, and C1 controls before external text reaches the terminal.
pub(crate) fn sanitize(text: &str) -> String {
    text.chars().filter(|c| !c.is_control()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removes_all_control_characters_and_preserves_unicode() {
        let controls: String = (0..=0x9f)
            .filter_map(char::from_u32)
            .filter(|c| c.is_control())
            .collect();
        assert_eq!(
            sanitize(&format!("日本語 café 🎵{controls}")),
            "日本語 café 🎵"
        );
    }

    #[test]
    fn disarms_osc_and_csi_sequences() {
        assert_eq!(
            sanitize("Song\x1b]52;c;VEVTVA==\x07\x1b[2J\u{9b}2J"),
            "Song]52;c;VEVTVA==[2J2J"
        );
    }
}

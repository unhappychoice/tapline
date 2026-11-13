use super::Screen;

/// Applies every complete sequence in `screen.pending` and returns how many
/// bytes were consumed; an incomplete trailing sequence is kept for later.
pub(super) fn feed(screen: &mut Screen) -> usize {
    let bytes = std::mem::take(&mut screen.pending);
    let mut pos = 0;
    while let Some(len) = step(screen, &bytes[pos..]) {
        pos += len;
    }
    screen.pending = bytes;
    pos
}

fn step(screen: &mut Screen, bytes: &[u8]) -> Option<usize> {
    match bytes.first()? {
        0x1b => escape(screen, bytes),
        b'\r' => {
            screen.cursor.0 = 0;
            Some(1)
        }
        b'\n' => {
            screen.cursor.1 = screen.cursor.1.saturating_add(1);
            Some(1)
        }
        b if b.is_ascii_control() => Some(1),
        _ => text(screen, bytes),
    }
}

fn text(screen: &mut Screen, bytes: &[u8]) -> Option<usize> {
    let len = utf8_len(bytes[0]);
    let chunk = bytes.get(..len)?;
    std::str::from_utf8(chunk)
        .map_or(Some('\u{FFFD}'), |s| s.chars().next())
        .into_iter()
        .for_each(|ch| screen.put(ch));
    Some(len)
}

fn utf8_len(lead: u8) -> usize {
    match lead {
        0xf0.. => 4,
        0xe0.. => 3,
        0xc0.. => 2,
        _ => 1,
    }
}

fn escape(screen: &mut Screen, bytes: &[u8]) -> Option<usize> {
    if *bytes.get(1)? != b'[' {
        return Some(2);
    }
    let end = bytes[2..].iter().position(|b| (0x40..=0x7e).contains(b))? + 2;
    let params = std::str::from_utf8(&bytes[2..end]).unwrap_or("");
    csi(screen, params, bytes[end]);
    Some(end + 1)
}

fn csi(screen: &mut Screen, params: &str, command: u8) {
    if params.starts_with('?') {
        return;
    }
    match command {
        b'H' => screen.cursor = cursor_target(params),
        b'm' => screen.style.apply_sgr(params),
        b'K' if params == "2" => screen.clear_row(screen.cursor.1, 0),
        b'K' => screen.clear_row(screen.cursor.1, screen.cursor.0),
        b'J' if params == "2" => screen.clear_all(),
        _ => {}
    }
}

fn cursor_target(params: &str) -> (u16, u16) {
    let mut parts = params
        .split(';')
        .map(|p| p.parse::<u16>().unwrap_or(1).max(1) - 1);
    let row = parts.next().unwrap_or(0);
    let col = parts.next().unwrap_or(0);
    (col, row)
}

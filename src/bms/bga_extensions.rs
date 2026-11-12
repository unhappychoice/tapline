use super::*;
use crate::bga::{Crop, Effect, EffectEvent, Extensions, Switch};

pub(super) fn absorb(ext: &mut Extensions, cmd: &str, val: &str, base: u32) -> bool {
    let up = cmd.to_ascii_uppercase();
    if up == "POORBGA" {
        if let Ok(mode @ 0..=2) = val.trim().parse() {
            ext.poor_mode = mode;
        }
        return true;
    }
    let Some((prefix, id)) = ["SWBGA", "ARGB", "BGA"].iter().find_map(|prefix| {
        up.strip_prefix(prefix)
            .and_then(|_| parse_id(cmd.get(prefix.len()..)?, base))
            .map(|id| (*prefix, id))
    }) else {
        return false;
    };
    match prefix {
        "ARGB" => {
            if let Some(color) = color(val) {
                ext.argb.insert(id, color);
            }
        }
        "BGA" => {
            if let Some(crop) = crop(val, base) {
                ext.crops.insert(id, crop);
            }
        }
        _ => {
            if let Some(switch) = switch(val, base) {
                ext.switches.insert(id, switch);
            }
        }
    }
    true
}

fn color(val: &str) -> Option<[u8; 4]> {
    val.split(',')
        .map(|v| v.trim().parse().ok())
        .collect::<Option<Vec<_>>>()?
        .try_into()
        .ok()
}

fn crop(val: &str, base: u32) -> Option<Crop> {
    let mut parts = val.split_whitespace();
    let image = parse_id(parts.next()?, base)?;
    let rect: [i32; 6] = parts
        .map(|v| v.parse().ok())
        .collect::<Option<Vec<_>>>()?
        .try_into()
        .ok()?;
    (rect[2] > rect[0].max(0) && rect[3] > rect[1].max(0)).then_some(Crop { image, rect })
}

fn switch(val: &str, base: u32) -> Option<Switch> {
    let mut words = val.split_whitespace();
    let parts: Vec<_> = words.next()?.split(':').collect();
    if parts.len() != 5 {
        return None;
    }
    let frame_ms = parts[0].parse().ok().filter(|v| *v > 0)?;
    let channel = parts[2].to_ascii_uppercase();
    if !matches!(channel.as_bytes(), [b'1' | b'2', b'1'..=b'9']) {
        return None;
    }
    let looping = match parts[3] {
        "0" => false,
        "1" => true,
        _ => return None,
    };
    let pattern_text = words.next()?;
    if pattern_text.len() % 2 != 0 || !pattern_text.is_ascii() || words.next().is_some() {
        return None;
    }
    let pattern = pattern_text
        .as_bytes()
        .chunks(2)
        .map(|v| parse_id(std::str::from_utf8(v).ok()?, base))
        .collect::<Option<Vec<_>>>()?;
    Some(Switch {
        frame_ms,
        duration_ms: parts[1].parse().ok()?,
        channel,
        lane: None,
        looping,
        argb: color(parts[4])?,
        pattern,
    })
}

pub(super) fn collect(
    mut ext: Extensions,
    raw: &[(u32, String, String)],
    timeline: &Timeline,
    base: u32,
) -> Extensions {
    for (measure, ch, data) in raw {
        let layers = [
            BgaLayer::Base,
            BgaLayer::Overlay,
            BgaLayer::Overlay2,
            BgaLayer::Poor,
        ];
        let opacity = ["0B", "0C", "0D", "0E"].iter().position(|v| v == ch);
        let color = ["A1", "A2", "A3", "A4"].iter().position(|v| v == ch);
        if opacity.is_none() && color.is_none() && ch != "A5" {
            continue;
        }
        let slots = if opacity.is_some() {
            parse_slots_hex(data)
        } else {
            parse_slots(data, base)
        };
        ext.events.extend(
            slots
                .iter()
                .enumerate()
                .filter(|(_, v)| **v != 0)
                .map(|(i, &id)| EffectEvent {
                    time_ms: timeline.time_at(*measure, i as f64 / slots.len() as f64),
                    effect: if let Some(index) = opacity {
                        Effect::Opacity(layers[index], id as u8)
                    } else if let Some(index) = color {
                        Effect::Color(layers[index], id)
                    } else {
                        Effect::Switch(id)
                    },
                }),
        );
    }
    ext.events.sort_by(|a, b| a.time_ms.total_cmp(&b.time_ms));
    ext
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn malformed_definitions_do_not_replace_valid_values() {
        let mut ext = Extensions::default();
        for line in [
            "ARGB01 255,1,2,3",
            "ARGB01 256,1,2,3",
            "ARGB02 1,2",
            "BGA01 01 0 0 64 64 -4 -4",
            "BGA01 01 0 0 0 0 0 0",
            "SWBGA01 100:0:11:1:255,0,0,0 0001",
            "SWBGA01 0:0:11:1:255,0,0,0 01",
            "SWBGA02 100:0:11:1:255,0,0,0 0!",
            "POORBGA 2",
            "POORBGA 3",
        ] {
            let (cmd, val) = split_cmd_value(line);
            assert!(absorb(&mut ext, cmd, val, 36));
        }
        assert_eq!(ext.argb.len(), 1);
        assert_eq!(ext.argb[&1], [255, 1, 2, 3]);
        assert_eq!(ext.crops[&1].rect, [0, 0, 64, 64, -4, -4]);
        assert_eq!(ext.switches.len(), 1);
        assert_eq!(ext.switches[&1].pattern, [0, 1]);
        assert_eq!(ext.poor_mode, 2);
    }

    #[test]
    fn extension_ids_follow_base_62_case() {
        let mut ext = Extensions::default();
        absorb(&mut ext, "argb0a", "255,0,0,0", 62);
        absorb(&mut ext, "ARGB0A", "255,1,2,3", 62);
        assert_eq!(ext.argb[&36], [255, 0, 0, 0]);
        assert_eq!(ext.argb[&10], [255, 1, 2, 3]);
    }
}
#[cfg(test)]
mod lane_tests {
    use super::super::bga_switch_lane;

    fn switch_lane(channel: &str, lanes: usize) -> Option<usize> {
        let (p1, p2) = match lanes {
            10 | 12 | 14 | 16 => (lanes / 2, Some(lanes / 2)),
            _ => (lanes, None),
        };
        bga_switch_lane(channel, p1, p2, lanes == 9)
    }

    #[test]
    fn switch_channels_follow_playable_layouts() {
        for (lanes, p1, p2) in [
            (6, 5, None),
            (8, 7, None),
            (12, 5, Some(11)),
            (16, 7, Some(15)),
        ] {
            assert_eq!(switch_lane("16", lanes), Some(p1));
            assert_eq!(switch_lane("26", lanes), p2);
            assert_eq!(switch_lane("17", lanes), None);
            assert_eq!(switch_lane("27", lanes), None);
        }
        for lanes in [5, 7, 10, 14] {
            assert_eq!(switch_lane("16", lanes), None);
            assert_eq!(switch_lane("26", lanes), None);
        }
        assert_eq!(switch_lane("18", 6), None);
        assert_eq!(switch_lane("28", 12), None);
        assert_eq!(switch_lane("29", 16), Some(14));
        assert_eq!(switch_lane("29", 14), Some(13));
        assert_eq!(switch_lane("25", 9), Some(8));
        for key in 1..=5 {
            assert_eq!(switch_lane(&format!("1{key}"), 10), Some(key - 1));
            assert_eq!(switch_lane(&format!("2{key}"), 10), Some(key + 4));
        }
        for channel in ["17", "18", "19", "27", "28", "29"] {
            assert_eq!(switch_lane(channel, 10), None);
        }
    }
}

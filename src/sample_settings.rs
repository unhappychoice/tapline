#[derive(Debug, Clone, Default, PartialEq)]
pub struct SampleSettings {
    pub pan: i32,
    pub attenuation: i32,
    pub frequency: Option<u32>,
    pub pitch: Option<u8>,
    pub volume: Option<u32>,
    pub length: Option<u32>,
}

impl SampleSettings {
    pub fn parse_exwav(value: &str) -> Option<(Self, &str)> {
        let (flags, mut rest) = token(value)?;
        let mut settings = Self::default();
        for flag in flags.to_ascii_lowercase().chars() {
            let (value, tail) = token(rest)?;
            let value = value.parse::<i32>().ok()?;
            match flag {
                'p' if (-10000..=10000).contains(&value) => settings.pan = value,
                'v' if (-10000..=0).contains(&value) => settings.attenuation = value,
                'f' if (100..=100000).contains(&value) => settings.frequency = Some(value as u32),
                _ => return None,
            }
            rest = tail;
        }
        (!rest.is_empty()).then_some((settings, rest))
    }

    pub fn gains(&self) -> (f32, f32) {
        // DirectSound pan and volume are measured in hundredths of a decibel.
        let gain = |attenuation: i32| {
            if attenuation <= -10000 {
                0.0
            } else {
                10.0_f32.powf(attenuation as f32 / 2000.0)
            }
        };
        let volume = gain(self.attenuation) * self.volume.unwrap_or(100) as f32 / 100.0;
        (
            volume * gain(-self.pan.max(0)),
            volume * gain(self.pan.min(0)),
        )
    }
}

fn token(value: &str) -> Option<(&str, &str)> {
    let value = value.trim();
    let end = value.find(char::is_whitespace)?;
    Some((&value[..end], value[end..].trim_start()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_preserve_gain_and_invalid_definitions_are_rejected() {
        assert_eq!(SampleSettings::default().gains(), (1.0, 1.0));
        [
            "p 10001 a.wav",
            "v 1 a.wav",
            "f 99 a.wav",
            "x 0 a.wav",
            "p nope a.wav",
            "p 0",
        ]
        .into_iter()
        .for_each(|value| {
            assert!(SampleSettings::parse_exwav(value).is_none(), "{value}");
        });
    }
}

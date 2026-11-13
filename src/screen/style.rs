/// One SGR color, kept `Copy` so cells stay cheap to compare and clone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Paint {
    #[default]
    Default,
    Basic(u8),
    Indexed(u8),
    Rgb(u8, u8, u8),
}

/// Terminal text style as tracked from SGR sequences.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct Style {
    fg: Paint,
    bg: Paint,
    attrs: u16,
}

impl Style {
    pub(crate) const DEFAULT: Self = Self {
        fg: Paint::Default,
        bg: Paint::Default,
        attrs: 0,
    };

    pub(crate) fn apply_sgr(&mut self, params: &str) {
        let codes: Vec<u16> = params.split(';').map(|p| p.parse().unwrap_or(0)).collect();
        let mut rest = codes.as_slice();
        while let Some((&code, tail)) = rest.split_first() {
            rest = self.apply_code(code, tail);
        }
    }

    fn apply_code<'a>(&mut self, code: u16, tail: &'a [u16]) -> &'a [u16] {
        match code {
            0 => *self = Self::default(),
            1..=9 => self.attrs |= 1 << code,
            22 => self.attrs &= !0b110,
            23..=29 => self.attrs &= !(1 << (code - 20)),
            30..=37 | 90..=97 => self.fg = Paint::Basic(code as u8),
            40..=47 | 100..=107 => self.bg = Paint::Basic(code as u8),
            39 => self.fg = Paint::Default,
            49 => self.bg = Paint::Default,
            38 | 48 => return self.apply_extended(code == 38, tail),
            _ => {}
        }
        tail
    }

    fn apply_extended<'a>(&mut self, fg: bool, tail: &'a [u16]) -> &'a [u16] {
        let (paint, rest) = match tail {
            [5, n, rest @ ..] => (Paint::Indexed(*n as u8), rest),
            [2, r, g, b, rest @ ..] => (Paint::Rgb(*r as u8, *g as u8, *b as u8), rest),
            _ => return &[],
        };
        if fg {
            self.fg = paint;
        } else {
            self.bg = paint;
        }
        rest
    }

    /// Full SGR sequence that recreates this style from a reset state.
    pub(crate) fn to_sgr(self) -> String {
        let attrs = (1..=9).filter(|bit| self.attrs & (1 << bit) != 0);
        let params: Vec<String> = std::iter::once("0".to_string())
            .chain(attrs.map(|bit| bit.to_string()))
            .chain(paint_params(self.fg, 30))
            .chain(paint_params(self.bg, 40))
            .collect();
        format!("\x1b[{}m", params.join(";"))
    }
}

fn paint_params(paint: Paint, base: u8) -> Option<String> {
    match paint {
        Paint::Default => None,
        Paint::Basic(code) => Some(code.to_string()),
        Paint::Indexed(n) => Some(format!("{};5;{n}", base + 8)),
        Paint::Rgb(r, g, b) => Some(format!("{};2;{r};{g};{b}", base + 8)),
    }
}

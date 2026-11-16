use super::{panel_width, ImageBank};
use crate::chart::Chart;

pub struct Presentation {
    pub bank: ImageBank,
    loaded: bool,
}

impl Presentation {
    pub fn new(chart: &Chart) -> Self {
        Self {
            bank: ImageBank::load(&Default::default(), &chart.bga, &chart.bga_extensions),
            loaded: false,
        }
    }

    pub fn prepare(&mut self, chart: &Chart, left_margin: u16, height: u16) {
        if !self.loaded && panel_width(left_margin, height).is_some() {
            self.bank = ImageBank::load(&chart.bmp_paths, &chart.bga, &chart.bga_extensions);
            self.bank.prepare_crops(chart);
            self.loaded = true;
        }
    }
}

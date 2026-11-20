use super::ImageBank;
use crate::chart::Chart;
use std::sync::mpsc::{self, Receiver, TryRecvError};

pub struct Presentation {
    pub bank: ImageBank,
    preparation: Preparation,
}

enum Preparation {
    Deferred,
    Loading(Receiver<ImageBank>),
    Finished,
}

impl Presentation {
    pub fn new(chart: &Chart, visible: bool) -> Self {
        if visible {
            return Self {
                bank: Self::load_assets(chart),
                preparation: Preparation::Finished,
            };
        }
        Self {
            bank: ImageBank::load(&Default::default(), &chart.bga, &chart.bga_extensions),
            preparation: Preparation::Deferred,
        }
    }

    pub fn prepare_deferred(
        &mut self,
        chart: &Chart,
        visible: bool,
        load: impl FnOnce(&Chart) -> ImageBank + Send + 'static,
    ) {
        match &self.preparation {
            Preparation::Deferred if visible => self.start_loading(chart, load),
            Preparation::Loading(receiver) => match receiver.try_recv() {
                Ok(bank) => {
                    self.bank = bank;
                    self.preparation = Preparation::Finished;
                }
                Err(TryRecvError::Disconnected) => self.preparation = Preparation::Finished,
                Err(TryRecvError::Empty) => {}
            },
            _ => {}
        }
    }

    pub fn load_assets(chart: &Chart) -> ImageBank {
        ImageBank::load(&chart.bmp_paths, &chart.bga, &chart.bga_extensions)
    }

    fn start_loading(
        &mut self,
        chart: &Chart,
        load: impl FnOnce(&Chart) -> ImageBank + Send + 'static,
    ) {
        let assets = Chart {
            bmp_paths: chart.bmp_paths.clone(),
            bga: chart.bga.clone(),
            bga_extensions: chart.bga_extensions.clone(),
            ..Chart::default()
        };
        let (sender, receiver) = mpsc::channel();
        let worker = std::thread::Builder::new()
            .name("bga-loader".into())
            .spawn(move || {
                let _ = sender.send(load(&assets));
            });
        self.preparation = if worker.is_ok() {
            Preparation::Loading(receiver)
        } else {
            Preparation::Finished
        };
    }
}

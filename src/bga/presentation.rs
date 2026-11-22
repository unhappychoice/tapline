use super::{bitmaps, ImageBank};
use crate::chart::Chart;
use std::{
    sync::mpsc::{self, Receiver, TryRecvError},
    sync::Arc,
};

pub struct Presentation {
    pub bank: ImageBank,
    preparation: Preparation<ImageBank>,
}

pub(crate) struct ChartImages<'a> {
    chart: &'a Chart,
    preparation: Preparation<Arc<bitmaps::Bitmaps>>,
}

enum Preparation<T> {
    Deferred,
    Loading(Receiver<T>),
    Ready(T),
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
            bank: ChartImages::new(chart).bank(None),
            preparation: Preparation::Deferred,
        }
    }

    pub fn prepare_deferred(
        &mut self,
        chart: &Chart,
        visible: bool,
        load: impl FnOnce(&Chart) -> ImageBank + Send + 'static,
    ) {
        if matches!(self.preparation, Preparation::Deferred) && visible {
            self.preparation = Preparation::start(chart, load);
            return;
        }
        if let Some(bank) = self.preparation.poll() {
            self.bank = bank;
        }
    }

    pub fn load_assets(chart: &Chart) -> ImageBank {
        ImageBank::load(&chart.bmp_paths, &chart.bga, &chart.bga_extensions)
    }

    pub(crate) fn with_cached_images(images: &mut ChartImages<'_>, visible: bool) -> Self {
        if visible && matches!(images.preparation, Preparation::Deferred) {
            images.preparation = Preparation::Ready(load_bitmaps(images.chart));
        }
        let bitmaps = visible.then(|| images.ready()).flatten();
        Self {
            preparation: if bitmaps.is_some() {
                Preparation::Finished
            } else {
                Preparation::Deferred
            },
            bank: images.bank(bitmaps),
        }
    }

    pub(crate) fn prepare_cached_images(&mut self, images: &mut ChartImages<'_>, visible: bool) {
        self.prepare_with(images, visible, load_bitmaps);
    }

    pub(crate) fn prepare_with(
        &mut self,
        images: &mut ChartImages<'_>,
        visible: bool,
        load: impl FnOnce(&Chart) -> Arc<bitmaps::Bitmaps> + Send + 'static,
    ) {
        if !matches!(self.preparation, Preparation::Deferred) || !visible {
            return;
        }
        if matches!(images.preparation, Preparation::Deferred) {
            images.preparation = Preparation::start(images.chart, load);
        }
        if let Some(bitmaps) = images.ready() {
            self.bank = images.bank(Some(bitmaps));
            self.preparation = Preparation::Finished;
        }
    }
}

impl<'a> ChartImages<'a> {
    pub(crate) fn new(chart: &'a Chart) -> Self {
        Self {
            chart,
            preparation: Preparation::Deferred,
        }
    }

    fn bank(&self, bitmaps: Option<Arc<bitmaps::Bitmaps>>) -> ImageBank {
        let paths = Default::default();
        ImageBank::with_bitmaps(
            bitmaps.as_ref().map_or(&paths, |_| &self.chart.bmp_paths),
            &self.chart.bga,
            &self.chart.bga_extensions,
            bitmaps.unwrap_or_default(),
        )
    }

    fn ready(&mut self) -> Option<Arc<bitmaps::Bitmaps>> {
        if let Some(bitmaps) = self.preparation.poll() {
            self.preparation = Preparation::Ready(bitmaps);
        }
        match &self.preparation {
            Preparation::Ready(bitmaps) => Some(Arc::clone(bitmaps)),
            _ => None,
        }
    }
}

impl<T: Send + 'static> Preparation<T> {
    fn poll(&mut self) -> Option<T> {
        let Self::Loading(receiver) = self else {
            return None;
        };
        let result = receiver.try_recv();
        if matches!(result, Err(TryRecvError::Empty)) {
            return None;
        }
        *self = Self::Finished;
        result.ok()
    }

    fn start(chart: &Chart, load: impl FnOnce(&Chart) -> T + Send + 'static) -> Self {
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
        if worker.is_ok() {
            Self::Loading(receiver)
        } else {
            Self::Finished
        }
    }
}

pub(crate) fn load_bitmaps(chart: &Chart) -> Arc<bitmaps::Bitmaps> {
    Arc::new(bitmaps::load(
        &chart.bmp_paths,
        &chart.bga,
        &chart.bga_extensions,
    ))
}

use super::{bitmaps, ImageBank};
use crate::chart::Chart;
use std::{
    sync::mpsc::{self, Receiver, TryRecvError},
    sync::Arc,
};

pub struct Presentation {
    pub bank: ImageBank,
    prepared: bool,
}

pub(crate) struct ChartImages<'a> {
    chart: &'a Chart,
    preparation: Preparation,
}

enum Preparation {
    Deferred,
    Loading(Receiver<Arc<bitmaps::Bitmaps>>),
    Ready(Arc<bitmaps::Bitmaps>),
    Failed,
}

impl Presentation {
    pub fn load_assets(chart: &Chart) -> ImageBank {
        ImageBank::load(&chart.bmp_paths, &chart.bga, &chart.bga_extensions)
    }

    pub(crate) fn new(images: &mut ChartImages<'_>, visible: bool) -> Self {
        if visible && matches!(images.preparation, Preparation::Deferred) {
            images.preparation = Preparation::Ready(load_bitmaps(images.chart));
        }
        let bitmaps = visible.then(|| images.ready()).flatten();
        Self {
            prepared: bitmaps.is_some(),
            bank: images.bank(bitmaps),
        }
    }

    pub(crate) fn prepare_deferred(&mut self, images: &mut ChartImages<'_>, visible: bool) {
        self.prepare_with(images, visible, load_bitmaps);
    }

    pub(crate) fn prepare_with(
        &mut self,
        images: &mut ChartImages<'_>,
        visible: bool,
        load: impl FnOnce(&Chart) -> Arc<bitmaps::Bitmaps> + Send + 'static,
    ) {
        if self.prepared || !visible {
            return;
        }
        if matches!(images.preparation, Preparation::Deferred) {
            images.start_loading(load);
        }
        if let Some(bitmaps) = images.ready() {
            self.bank = images.bank(Some(bitmaps));
            self.prepared = true;
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
        if let Preparation::Loading(receiver) = &self.preparation {
            self.preparation = match receiver.try_recv() {
                Ok(bitmaps) => Preparation::Ready(bitmaps),
                Err(TryRecvError::Disconnected) => Preparation::Failed,
                Err(TryRecvError::Empty) => return None,
            };
        }
        match &self.preparation {
            Preparation::Ready(bitmaps) => Some(Arc::clone(bitmaps)),
            _ => None,
        }
    }

    fn start_loading(
        &mut self,
        load: impl FnOnce(&Chart) -> Arc<bitmaps::Bitmaps> + Send + 'static,
    ) {
        let assets = Chart {
            bmp_paths: self.chart.bmp_paths.clone(),
            bga: self.chart.bga.clone(),
            bga_extensions: self.chart.bga_extensions.clone(),
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
            Preparation::Failed
        };
    }
}

pub(crate) fn load_bitmaps(chart: &Chart) -> Arc<bitmaps::Bitmaps> {
    Arc::new(bitmaps::load(
        &chart.bmp_paths,
        &chart.bga,
        &chart.bga_extensions,
    ))
}

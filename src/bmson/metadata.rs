use super::model::Info;
use crate::bms::ChartMeta;
use anyhow::Result;
use std::path::Path;

pub(super) fn convert(path: &Path, info: &Info) -> Result<ChartMeta> {
    let mode = info.mode_hint.as_deref().unwrap_or("beat-7k");
    let lane_count = match mode {
        "beat-7k" | "generic-7keys" => 7,
        "beat-14k" | "generic-14keys" => 14,
        "beat-5k" | "popn-5k" | "generic-5keys" => 5,
        "generic-4keys" => 4,
        _ => anyhow::bail!("Unsupported bmson mode: {mode}"),
    };
    Ok(ChartMeta {
        path: path.into(),
        random_seed: 0,
        preview: None,
        title: info.title.clone(),
        subtitle: info.subtitle.clone(),
        artist: info.artist.clone(),
        subartist: info.subartists.join(", "),
        genre: info.genre.clone(),
        bpm: info.init_bpm,
        playlevel: info.level,
        difficulty: info.chart_name.as_deref().and_then(|name| {
            ["BEGINNER", "NORMAL", "HYPER", "ANOTHER", "INSANE"]
                .iter()
                .position(|n| name.eq_ignore_ascii_case(n))
                .map(|i| i as u8 + 1)
        }),
        lane_count,
        banner: info.banner_image.clone().unwrap_or_default(),
        comment: String::new(),
    })
}

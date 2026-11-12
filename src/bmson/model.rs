use serde::Deserialize;

#[derive(Deserialize)]
pub struct Bmson {
    pub version: String,
    pub info: Info,
    #[serde(default)]
    pub bpm_events: Vec<Bpm>,
    #[serde(default)]
    pub stop_events: Vec<Stop>,
    pub sound_channels: Vec<SoundChannel>,
    #[serde(default)]
    pub bga: Bga,
}

#[derive(Deserialize)]
pub struct Info {
    pub title: String,
    #[serde(default)]
    pub subtitle: String,
    pub artist: String,
    #[serde(default)]
    pub subartists: Vec<String>,
    #[serde(default)]
    pub genre: String,
    pub init_bpm: f64,
    pub resolution: Option<u32>,
    pub mode_hint: Option<String>,
    pub level: Option<u8>,
    pub chart_name: Option<String>,
    pub eyecatch_image: Option<String>,
    pub banner_image: Option<String>,
}

#[derive(Deserialize)]
pub struct Bpm {
    pub y: u32,
    pub bpm: f64,
}

#[derive(Deserialize)]
pub struct Stop {
    pub y: u32,
    pub duration: u32,
}

#[derive(Deserialize)]
pub struct SoundChannel {
    pub name: String,
    pub notes: Vec<SoundNote>,
}

#[derive(Deserialize)]
pub struct SoundNote {
    pub x: Option<u32>,
    pub y: u32,
    #[serde(default)]
    pub l: u32,
    #[serde(default)]
    pub c: bool,
}

#[derive(Default, Deserialize)]
#[serde(default)]
pub struct Bga {
    pub bga_header: Vec<Image>,
    pub bga_events: Vec<ImageEvent>,
    pub layer_events: Vec<ImageEvent>,
    pub poor_events: Vec<ImageEvent>,
}

#[derive(Deserialize)]
pub struct Image {
    pub id: u32,
    pub name: String,
}

#[derive(Deserialize)]
pub struct ImageEvent {
    pub y: u32,
    pub id: u32,
}

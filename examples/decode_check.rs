use std::env;
use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;

use rodio::{Decoder, Source};

fn main() {
    for arg in env::args().skip(1) {
        let path = PathBuf::from(&arg);
        let f = match File::open(&path) {
            Ok(f) => BufReader::new(f),
            Err(e) => {
                println!("OPEN-ERR {} :: {}", arg, e);
                continue;
            }
        };
        match Decoder::new(f) {
            Ok(dec) => {
                let ch = dec.channels();
                let sr = dec.sample_rate();
                let dur = dec.total_duration();
                let n: usize = dec.convert_samples::<f32>().count();
                println!("OK {arg} ch={ch} sr={sr} dur={dur:?} samples={n}");
            }
            Err(e) => println!("DECODE-ERR {} :: {}", arg, e),
        }
    }
}

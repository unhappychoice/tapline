use std::env;
use std::path::PathBuf;
use tapline::bms;

fn main() {
    let mut ok = 0usize;
    let mut fail = 0usize;
    for arg in env::args().skip(1) {
        let path = PathBuf::from(&arg);
        print!("{arg:52} ");
        match bms::load(&path, 0.0) {
            Ok(c) => {
                ok += 1;
                println!(
                    "OK  title={:?} bpm={:.0} lanes={} notes={} p2={} mines={} bgm={} bga={} dur={:.0}ms",
                    c.title,
                    c.bpm,
                    c.lane_count,
                    c.notes.len(),
                    c.p2_notes.len(),
                    c.mines.len(),
                    c.bgm.len(),
                    c.bga.len(),
                    c.duration_ms,
                );
            }
            Err(e) => {
                fail += 1;
                println!("ERR {e}");
            }
        }
    }
    eprintln!("summary: {ok} ok, {fail} fail");
}

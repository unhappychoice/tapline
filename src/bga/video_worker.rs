use super::video::{Start, Video};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Condvar, Mutex},
    thread::{self, JoinHandle},
};

#[derive(Default)]
struct Mailbox {
    pending: Option<(Option<PathBuf>, Start)>,
    ready: Option<Video>,
    closed: bool,
}

pub(super) struct Worker {
    shared: Arc<(Mutex<Mailbox>, Condvar)>,
    thread: Option<JoinHandle<()>>,
}

impl Worker {
    pub fn start<D>(
        open: impl FnMut(&Path, Start) -> Option<(D, Video)> + Send + 'static,
    ) -> Option<Self> {
        let shared = Arc::new((Mutex::default(), Condvar::new()));
        let worker = Arc::clone(&shared);
        let thread = thread::Builder::new()
            .name("bga-video".into())
            .spawn(move || run(&worker, open))
            .ok()?;
        Some(Self {
            shared,
            thread: Some(thread),
        })
    }

    pub fn request(&self, path: Option<PathBuf>, start: Start) {
        let (mailbox, wake) = &*self.shared;
        let mut state = mailbox.lock().unwrap();
        state.pending = Some((path, start));
        state.ready = None;
        wake.notify_one();
    }

    pub fn take_video(&self) -> Option<Video> {
        let (mailbox, _) = &*self.shared;
        mailbox.lock().unwrap().ready.take()
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        let (mailbox, wake) = &*self.shared;
        mailbox.lock().unwrap().closed = true;
        wake.notify_one();
        // The bank is dropped after gameplay, so finish reaping before the session exits.
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn run<D>(
    (mailbox, wake): &(Mutex<Mailbox>, Condvar),
    mut open: impl FnMut(&Path, Start) -> Option<(D, Video)>,
) {
    let mut decoder = None;
    let mut state = mailbox.lock().unwrap();
    loop {
        state = wake
            .wait_while(state, |state| !state.closed && state.pending.is_none())
            .unwrap();
        if state.closed {
            break;
        }

        // Leave the request in the mailbox during cleanup so newer events can replace it.
        drop(state);
        drop(decoder.take());
        state = mailbox.lock().unwrap();
        if state.closed {
            break;
        }
        let (path, start) = state.pending.take().unwrap();
        drop(state);

        let opened = path.as_deref().and_then(|path| open(path, start));
        state = mailbox.lock().unwrap();
        if let Some((process, video)) = opened {
            decoder = Some(process);
            if !state.closed && state.pending.is_none() {
                state.ready = Some(video);
            }
        }
    }
}

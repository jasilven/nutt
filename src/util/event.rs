use std::io;
use std::sync::mpsc;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::thread;
use std::time::Duration;

use termion::event::Key;
use termion::input::TermRead;

pub enum Event<I> {
    Input(I),
    Tick,
}

/// A small event handler that wrap termion input and tick events. Each event
/// type is handled in its own thread and returned to a common `Receiver`
pub struct Events {
    rx: mpsc::Receiver<Event<Key>>,
    input_handle: thread::JoinHandle<()>,
    ignore_exit_key: Arc<AtomicBool>,
    ignore_compose_key: Arc<AtomicBool>,
    tick_handle: thread::JoinHandle<()>,
    exit: Arc<AtomicBool>,
}

#[derive(Debug, Clone, Copy)]
pub struct Config {
    pub exit_key: Key,
    pub compose_key: Key,
    pub tick_rate: Duration,
}

impl Default for Config {
    fn default() -> Config {
        Config {
            exit_key: Key::Char('q'),
            compose_key: Key::Char('e'),
            tick_rate: Duration::from_millis(250),
        }
    }
}

impl Events {
    pub fn new() -> Events {
        Events::with_config(Config::default())
    }

    pub fn with_config(config: Config) -> Events {
        let (tx, rx) = mpsc::channel();
        let ignore_exit_key = Arc::new(AtomicBool::new(false));
        let ignore_compose_key = Arc::new(AtomicBool::new(false));
        let exit = Arc::new(AtomicBool::new(false));
        let input_handle = {
            let tx = tx.clone();
            let ignore_exit_key = ignore_exit_key.clone();
            let ignore_compose_key = ignore_compose_key.clone();
            let exit = exit.clone();
            thread::spawn(move || {
                let stdin = io::stdin();
                for evt in stdin.keys() {
                    match evt {
                        Ok(key) => {
                            if let Err(_) = tx.send(Event::Input(key)) {
                                break;
                            }
                            if !ignore_exit_key.load(Ordering::Relaxed) && key == config.exit_key {
                                break;
                            }
                            if !ignore_compose_key.load(Ordering::Relaxed)
                                && key == config.compose_key
                            {
                                break;
                            }
                        }
                        Err(_) => {}
                    }
                }
                exit.store(true, Ordering::Relaxed);
            })
        };
        let tick_handle = {
            let tx = tx.clone();
            let is_exit = exit.clone();
            thread::spawn(move || {
                let tx = tx.clone();
                loop {
                    if is_exit.load(Ordering::Relaxed) {
                        break;
                    }
                    tx.send(Event::Tick).unwrap();
                    thread::sleep(config.tick_rate);
                }
            })
        };
        Events {
            rx,
            ignore_exit_key,
            ignore_compose_key,
            input_handle,
            tick_handle,
            exit,
        }
    }

    pub fn next(&self) -> Result<Event<Key>, mpsc::RecvError> {
        self.rx.recv()
    }
    pub fn disable_exit_key(&mut self) {
        self.ignore_exit_key.store(true, Ordering::Relaxed);
    }

    pub fn enable_exit_key(&mut self) {
        self.ignore_exit_key.store(false, Ordering::Relaxed);
    }
}

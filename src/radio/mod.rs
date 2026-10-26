use std::sync::OnceLock;

use serde_json::{Value, json};
use tokio::sync::mpsc;

use crate::store::{radio_store, settings_store};
use dab::DabTuner;
use fm::FmTuner;

pub mod channels;
pub mod dab;
pub mod fm;
pub mod hardware;
pub mod wradio_backend;

/// Poked by the tuners whenever something the UI shows changed on its own (RDS text, DAB label)
pub type Notifier = mpsc::UnboundedSender<()>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Fm,
    Dab,
}

impl Mode {
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "fm" => Some(Self::Fm),
            "dab" => Some(Self::Dab),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Fm => "fm",
            Self::Dab => "dab",
        }
    }
}

// Work the UI hands to the radio service. They run one at a time, in the order they were sent.
pub enum Command {
    Refresh,
    // Starts the tuner for the mode and stops the other one
    Activate(Mode),
    // Releases the hardware, e.g. when the radio page is left
    Deactivate,
    FmToggle,
    FmStep { direction: i32, fast: bool },
    FmSetFavorite(usize),
    FmRecallFavorite(usize),
    DabScan,
    DabTogglePlay,
    DabTune { id: u32, frequency: u32 },
    DabSetFavorite(usize),
    DabRecallFavorite(usize),
}

static COMMANDS: OnceLock<mpsc::UnboundedSender<Command>> = OnceLock::new();

// Never blocks
pub fn send(command: Command) {
    let Some(commands) = COMMANDS.get() else {
        eprintln!("radio service not started");
        return;
    };
    if commands.send(command).is_err() {
        eprintln!("radio service stopped");
    }
}

// Call once at startup, after the settings are loaded
pub fn start() {
    let (tx, rx) = mpsc::unbounded_channel();
    COMMANDS.set(tx).unwrap_or_else(|_| panic!("radio::start called twice"));
    crate::runtime::spawn(run(rx));
}

async fn run(mut commands: mpsc::UnboundedReceiver<Command>) {
    let (notify, mut changes) = mpsc::unbounded_channel();
    let mut service = RadioService::new(notify, &settings_store::snapshot());

    loop {
        tokio::select! {
            command = commands.recv() => {
                let Some(command) = command else { break };
                service.busy = true;
                service.publish();
                service.handle(command).await;
                service.busy = false;
            }
            Some(()) = changes.recv() => while changes.try_recv().is_ok() {},
        }
        service.publish();
    }

    service.release().await;
}

struct RadioService {
    mode: Mode,
    // Whether the radio page is open, i.e. whether a tuner should be running
    active: bool,
    busy: bool,
    scanning: bool,
    error: Option<String>,
    fm: FmTuner,
    dab: DabTuner,
}

impl RadioService {
    fn new(notify: Notifier, saved: &settings_store::AppSettings) -> Self {
        Self {
            mode: Mode::parse(&saved.radio_mode).unwrap_or(Mode::Fm),
            active: false,
            busy: false,
            scanning: false,
            error: None,
            fm: FmTuner::new(notify.clone(), saved.fm_frequency, &saved.fm_favorites),
            dab: DabTuner::new(
                notify,
                saved.dab_stations.clone(),
                saved.dab_favorites.clone(),
                saved.dab_last.clone(),
            ),
        }
    }

    async fn handle(&mut self, command: Command) {
        self.error = None;
        let result = match command {
            Command::Refresh => Ok(()),
            Command::Activate(mode) => self.activate(mode).await,
            Command::Deactivate => {
                self.release().await;
                Ok(())
            }
            Command::FmToggle => self.fm_toggle().await,
            Command::FmStep { direction, fast } => {
                self.fm.step(direction, fast).await;
                Ok(())
            }
            Command::FmSetFavorite(slot) => self.fm.set_favorite(slot),
            Command::FmRecallFavorite(slot) => self.fm.recall_favorite(slot).await,
            Command::DabScan => self.dab_scan().await,
            Command::DabTogglePlay => self.dab_toggle_play().await,
            Command::DabTune { id, frequency } => match self.dab.find(id, frequency) {
                Some(station) => self.dab.tune(station).await,
                None => Err("Unknown DAB station".into()),
            },
            Command::DabSetFavorite(slot) => self.dab.set_favorite(slot),
            Command::DabRecallFavorite(slot) => self.dab.recall_favorite(slot).await,
        };
        if let Err(e) = result {
            eprintln!("radio: {e}");
            self.error = Some(e);
        }
        self.persist();
    }

    async fn activate(&mut self, mode: Mode) -> Result<(), String> {
        self.mode = mode;
        self.active = true;
        match mode {
            Mode::Fm => {
                self.log_dab_stop().await;
                if !hardware::get().fm {
                    return Err("No FM-capable tuner is plugged in".into());
                }
                self.fm.start(self.fm.frequency()).await
            }
            Mode::Dab => {
                self.fm.stop().await;
                self.dab.start(hardware::get()).await
            }
        }
    }

    async fn release(&mut self) {
        self.active = false;
        self.fm.stop().await;
        self.log_dab_stop().await;
    }

    async fn log_dab_stop(&mut self) {
        if let Err(e) = self.dab.stop().await {
            eprintln!("radio: stopping DAB: {e}");
        }
    }

    async fn fm_toggle(&mut self) -> Result<(), String> {
        if self.fm.running() {
            self.fm.stop().await;
            Ok(())
        } else {
            self.fm.start(self.fm.frequency()).await
        }
    }

    async fn dab_scan(&mut self) -> Result<(), String> {
        self.scanning = true;
        self.publish();
        let result = self
            .dab
            .scan(&mut |progress| radio_store::set_scan_text(progress.describe()))
            .await;
        self.scanning = false;
        radio_store::set_scan_text(String::new());
        result
    }

    async fn dab_toggle_play(&mut self) -> Result<(), String> {
        match self.dab.current().cloned() {
            Some(station) if !self.dab.playing() => self.dab.tune(station).await,
            _ => self.dab.stop_playback().await,
        }
    }

    fn persist(&self) {
        let mode = self.mode.name().to_string();
        let frequency = self.fm.frequency();
        let fm_favorites = self.fm.favorites().to_vec();
        let stations = self.dab.stations().to_vec();
        let dab_favorites = self.dab.favorites().to_vec();
        let last = self.dab.current().cloned();
        settings_store::update(move |s| {
            s.radio_mode = mode;
            s.fm_frequency = frequency;
            s.fm_favorites = fm_favorites;
            s.dab_stations = stations;
            s.dab_favorites = dab_favorites;
            s.dab_last = last;
        });
    }

    fn publish(&self) {
        radio_store::set_state(self.state());
    }

    fn state(&self) -> Value {
        let station = self.fm.station();
        json!({
            "mode": self.mode.name(),
            "busy": self.busy,
            "error": self.error,
            "fm": {
                "running": self.fm.running(),
                "frequency": self.fm.frequency(),
                "stationName": station.as_ref().and_then(|s| s.name.clone()),
                "stationText": station.as_ref().and_then(|s| s.text.clone()),
                "favorites": self.fm.favorites(),
            },
            "dab": {
                "running": self.dab.running(),
                "playing": self.dab.playing(),
                "scanning": self.scanning,
                "stations": self.dab.stations(),
                "current": self.dab.current(),
                "dynamicLabel": self.dab.dynamic_label(),
                "favorites": self.dab.favorites(),
            },
        })
    }
}


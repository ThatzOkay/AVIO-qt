use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::OnceLock;

use serde_json::{Value, json};
use tokio::sync::mpsc;

use crate::store::{radio_store, settings_store};
use dab::{DabStation, DabTuner};
use slide_cache::file_url;
use fm::FmTuner;

pub mod channels;
pub mod dab;
pub mod fm;
pub mod hardware;
pub mod slide_cache;
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
    // Writes the station list into this folder / replaces it with the list in this file
    DabExportList(PathBuf),
    DabImportList(PathBuf),
}

impl Command {
    /// Whether this should cancel a tune that's still retrying: the user stopped, picked another
    /// station or went elsewhere.
    fn interrupts_tune(&self) -> bool {
        matches!(
            self,
            Self::DabTune { .. }
                | Self::DabTogglePlay
                | Self::DabRecallFavorite(_)
                | Self::DabScan
                | Self::Deactivate
                | Self::Activate(Mode::Fm)
        )
    }

    /// Whether this can be served right away while a tune is retrying, without touching it.
    fn runs_during_tune(&self) -> bool {
        matches!(
            self,
            Self::Refresh
                | Self::Activate(Mode::Dab)
                | Self::DabSetFavorite(_)
                | Self::DabExportList(_)
                | Self::DabImportList(_)
        )
    }
}

// What the service listens on besides the command it's running
struct Io {
    commands: mpsc::UnboundedReceiver<Command>,
    changes: mpsc::UnboundedReceiver<()>,
    // Commands that arrived during a tune and have to wait their turn
    backlog: VecDeque<Command>,
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

async fn run(commands: mpsc::UnboundedReceiver<Command>) {
    let (notify, changes) = mpsc::unbounded_channel();
    let mut service = RadioService::new(notify, &settings_store::snapshot());
    let mut io = Io { commands, changes, backlog: VecDeque::new() };
    // A command that cancelled a tune, to run next
    let mut interrupt = None;

    loop {
        let command = match io.backlog.pop_front().or_else(|| interrupt.take()) {
            Some(command) => command,
            None => tokio::select! {
                command = io.commands.recv() => match command {
                    Some(command) => command,
                    None => break,
                },
                Some(()) = io.changes.recv() => {
                    while io.changes.try_recv().is_ok() {}
                    service.publish();
                    continue;
                }
            },
        };
        service.busy = true;
        service.publish();
        interrupt = service.handle(command, &mut io).await;
        service.busy = false;
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
    // What the last export/import did, for the settings page
    notice: Option<String>,
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
            notice: None,
            fm: FmTuner::new(notify.clone(), saved.fm_frequency, &saved.fm_favorites),
            dab: DabTuner::new(
                notify,
                saved.dab_stations.clone(),
                saved.dab_favorites.clone(),
                saved.dab_last.clone(),
            ),
        }
    }

    // Returns the command that cancelled a tune started by this one, if any
    async fn handle(&mut self, command: Command, io: &mut Io) -> Option<Command> {
        self.error = None;
        self.notice = None;
        let mut interrupt = None;
        let result = match command {
            Command::Refresh => Ok(()),
            Command::Activate(mode) => self.activate(mode, io, &mut interrupt).await,
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
            Command::DabTogglePlay => self.dab_toggle_play(io, &mut interrupt).await,
            Command::DabTune { id, frequency } => match self.dab.find(id, frequency) {
                Some(station) => self.dab_tune(station, io, &mut interrupt).await,
                None => Err("Unknown DAB station".into()),
            },
            Command::DabSetFavorite(slot) => self.dab.set_favorite(slot),
            Command::DabRecallFavorite(slot) => match self.dab.favorite(slot) {
                Ok(station) => self.dab_tune(station, io, &mut interrupt).await,
                Err(e) => Err(e),
            },
            Command::DabExportList(folder) => self.dab.export_list(&folder).await.map(|notice| self.notice = Some(notice)),
            Command::DabImportList(file) => self.dab.import_list(&file).await.map(|notice| self.notice = Some(notice)),
        };
        if let Err(e) = result {
            eprintln!("radio: {e}");
            self.error = Some(e);
        }
        self.persist();
        interrupt
    }

    async fn activate(
        &mut self,
        mode: Mode,
        io: &mut Io,
        interrupt: &mut Option<Command>,
    ) -> Result<(), String> {
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
                // Only when the tuner was just opened (the page was entered), so switching tabs
                // doesn't restart a station the user stopped
                let opened = !self.dab.running();
                self.dab.start(hardware::get()).await?;
                match self.dab.current().cloned() {
                    Some(last) if opened && !self.dab.playing() => self.dab_tune(last, io, interrupt).await,
                    _ => Ok(()),
                }
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

    async fn dab_toggle_play(&mut self, io: &mut Io, interrupt: &mut Option<Command>) -> Result<(), String> {
        // Stop covers a tune that's still retrying too (it was cancelled to get here)
        if self.dab.playing() || self.dab.tuning().is_some() {
            return self.dab.stop_playback().await;
        }
        match self.dab.current().cloned() {
            Some(station) => self.dab_tune(station, io, interrupt).await,
            None => Ok(()),
        }
    }

    /// Tunes `station`, retrying until it plays. Meanwhile commands keep being served: stopping or
    /// picking another station drops the tune (and is handed back through `interrupt` to run
    /// next), the rest either run now or wait in the backlog.
    async fn dab_tune(
        &mut self,
        station: DabStation,
        io: &mut Io,
        interrupt: &mut Option<Command>,
    ) -> Result<(), String> {
        let job = self.dab.begin_tune(station.clone())?;
        self.publish();

        let mut tune = Box::pin(job.run(&station));
        let result = loop {
            tokio::select! {
                result = &mut tune => break Some(result),
                command = io.commands.recv() => match command {
                    None => break None,
                    Some(command) if command.interrupts_tune() => {
                        *interrupt = Some(command);
                        break None;
                    }
                    Some(command) if command.runs_during_tune() => {
                        self.handle_inline(command).await;
                        self.publish();
                    }
                    Some(command) => io.backlog.push_back(command),
                },
                Some(()) = io.changes.recv() => {
                    while io.changes.try_recv().is_ok() {}
                    self.publish();
                }
            }
        };
        drop(tune);

        match result {
            Some(result) => {
                self.dab.finish_tune(station, &result);
                result
            }
            // Cancelled: whoever cancelled it deals with `tuning`, which is how a stop knows
            // there was something to stop
            None => Ok(()),
        }
    }

    /// The commands `Command::runs_during_tune` lets through
    async fn handle_inline(&mut self, command: Command) {
        self.error = None;
        self.notice = None;
        let result = match command {
            Command::DabSetFavorite(slot) => self.dab.set_favorite(slot),
            Command::DabExportList(folder) => self.dab.export_list(&folder).await.map(|notice| self.notice = Some(notice)),
            Command::DabImportList(file) => self.dab.import_list(&file).await.map(|notice| self.notice = Some(notice)),
            _ => Ok(()),
        };
        if let Err(e) = result {
            eprintln!("radio: {e}");
            self.error = Some(e);
        }
        self.persist();
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
            // A tune that's retrying isn't busy: the UI has to stay usable to cancel it
            "busy": self.busy && self.dab.tuning().is_none(),
            "error": self.error,
            "notice": self.notice,
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
                "tuning": self.dab.tuning(),
                "tuningStatus": self.dab.tuning().and_then(|_| self.dab.tune_status()),
                "signalBars": self.dab.signal_bars(),
                "audioReady": self.dab.audio_ready(),
                "audioError": self.dab.audio_error(),
                "scanning": self.scanning,
                "stations": stations_json(self.dab.stations()),
                "current": self.dab.current(),
                "dynamicLabel": self.dab.dynamic_label(),
                "slide": self.dab.slide().map(|path| file_url(&path)),
                "favorites": self.dab.favorites(),
            },
        })
    }
}

// The station list for the UI: each station with its cached slideshow picture, if it has one
fn stations_json(stations: &[DabStation]) -> Value {
    let slides = slide_cache::all();
    Value::Array(
        stations
            .iter()
            .map(|station| {
                let mut value = json!(station);
                value["slide"] = json!(slides.get(&station.id).map(|path| file_url(path)));
                value
            })
            .collect(),
    )
}

use qtbridge::{QmlObject, qobject};
use serde_json::Value;

use super::Remote;
use crate::radio::{self, Command, Mode};

static REMOTE: Remote = Remote::new();

// Everything the radio page shows lives in `state` (see `RadioService::state`), pushed whole by
// the radio service; the slots below just hand commands to it.
#[derive(Default)]
pub struct RadioStore {
    state: Value,
    scan_text: String,
}

#[qobject(Singleton)]
impl RadioStore {
    qproperty!("state", Read = state, Notify = state_changed);
    qproperty!("scanText", Read = scan_text, Notify = scan_text_changed);

    #[qsignal]
    fn state_changed(&mut self);
    #[qsignal]
    fn scan_text_changed(&mut self);

    pub fn state(&self) -> Value {
        self.state.clone()
    }

    pub fn scan_text(&self) -> String {
        self.scan_text.clone()
    }

    // Call once from QML on startup: registers for updates and asks for the current state
    #[qslot]
    fn init(&self) {
        REMOTE.set(self.get_qml_method_invoker());
        radio::send(Command::Refresh);
    }

    // "fm" or "dab"
    #[qslot]
    fn activate(&self, mode: String) {
        match Mode::parse(&mode) {
            Some(mode) => radio::send(Command::Activate(mode)),
            None => eprintln!("radio: unknown mode {mode}"),
        }
    }

    #[qslot]
    fn deactivate(&self) {
        radio::send(Command::Deactivate);
    }

    #[qslot]
    fn fm_toggle(&self) {
        radio::send(Command::FmToggle);
    }

    #[qslot]
    fn fm_step(&self, direction: i32, fast: bool) {
        radio::send(Command::FmStep { direction, fast });
    }

    #[qslot]
    fn fm_set_favorite(&self, slot: u32) {
        radio::send(Command::FmSetFavorite(slot as usize));
    }

    #[qslot]
    fn fm_recall_favorite(&self, slot: u32) {
        radio::send(Command::FmRecallFavorite(slot as usize));
    }

    #[qslot]
    fn dab_scan(&self) {
        radio::send(Command::DabScan);
    }

    #[qslot]
    fn dab_toggle_play(&self) {
        radio::send(Command::DabTogglePlay);
    }

    #[qslot]
    fn dab_tune(&self, id: u32, frequency: u32) {
        radio::send(Command::DabTune { id, frequency });
    }

    #[qslot]
    fn dab_set_favorite(&self, slot: u32) {
        radio::send(Command::DabSetFavorite(slot as usize));
    }

    #[qslot]
    fn dab_recall_favorite(&self, slot: u32) {
        radio::send(Command::DabRecallFavorite(slot as usize));
    }

    #[qslot]
    fn dab_export_list(&self, folder: String) {
        radio::send(Command::DabExportList(folder.into()));
    }

    #[qslot]
    fn dab_import_list(&self, file: String) {
        radio::send(Command::DabImportList(file.into()));
    }

    #[qslot]
    fn state_set(&mut self, state: Value) {
        self.state = state;
        self.state_changed();
    }

    #[qslot]
    fn scan_text_set(&mut self, text: String) {
        self.scan_text = text;
        self.scan_text_changed();
    }
}

// Safe from any thread. Dropped if QML hasn't called RadioStore.init() yet.
pub fn set_state(state: Value) {
    REMOTE.with(|invoker| {
        qtbridge::invoke_method!(invoker, "state_set", state);
    });
}

pub fn set_scan_text(text: String) {
    REMOTE.with(|invoker| {
        qtbridge::invoke_method!(invoker, "scan_text_set", text);
    });
}

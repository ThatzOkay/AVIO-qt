use qtbridge::{QmlObject, qobject};

use super::Remote;
use crate::runtime;
use crate::usb::udev_rule::{self, RuleStatus};

static REMOTE: Remote = Remote::new();

// State of the USB permission (udev) rule, for the dialog QML shows about it:
// "ok" nothing to ask, "missing" / "outdated" waiting for the user to accept the install,
// "installing" pkexec is running, "failed" it was cancelled or errored, "installed" restart needed
pub struct UdevStore {
    state: String,
    rule_file: String,
}

impl Default for UdevStore {
    fn default() -> Self {
        Self {
            state: "ok".to_string(),
            rule_file: udev_rule::rule_file().to_string(),
        }
    }
}

#[qobject(Singleton)]
impl UdevStore {
    qproperty!("state", Read = state, Notify = state_changed);
    qproperty!("ruleFile", Read = rule_file, Notify = rule_file_changed);

    #[qsignal]
    fn state_changed(&mut self);
    #[qsignal]
    fn rule_file_changed(&mut self);

    pub fn state(&self) -> String {
        self.state.clone()
    }

    pub fn rule_file(&self) -> String {
        self.rule_file.clone()
    }

    // Call once from QML on startup: registers for replies and checks the installed rule
    #[qslot]
    fn init(&self) {
        REMOTE.set(self.get_qml_method_invoker());
        runtime::spawn(async {
            let state = match tokio::task::spawn_blocking(udev_rule::rule_status).await {
                Ok(RuleStatus::Missing) => "missing",
                Ok(RuleStatus::Outdated) => "outdated",
                _ => "ok",
            };
            // Nothing to ask the user if the rule can't be installed from here (an image
            // without polkit, not running as root): it has to ship in the image
            if state != "ok" && !udev_rule::can_install() {
                eprintln!("udev rule is {state} and can't be installed from here (no root, no pkexec)");
                return;
            }
            set_state(state);
        });
    }

    // The user accepted the install (or retry) prompt
    #[qslot]
    fn install(&mut self) {
        self.state = "installing".to_string();
        self.state_changed();
        runtime::spawn(async {
            match udev_rule::install_rule().await {
                Ok(()) => set_state("installed"),
                Err(e) => {
                    eprintln!("Failed to install udev rule: {:?}", e);
                    set_state("failed");
                }
            }
        });
    }

    #[qslot]
    fn skip(&mut self) {
        self.state = "ok".to_string();
        self.state_changed();
    }

    #[qslot]
    fn restart(&self) {
        udev_rule::restart_app();
    }

    #[qslot]
    fn state_set(&mut self, state: String) {
        self.state = state;
        self.state_changed();
    }
}

fn set_state(state: &str) {
    let state = state.to_string();
    REMOTE.with(|invoker| {
        qtbridge::invoke_method!(invoker, "state_set", state);
    });
}

use std::{sync::Arc, thread::sleep, time::Duration, vec};

use regex::Regex;
use regex_split::RegexSplit;
use tokio::io::{AsyncBufReadExt, BufReader};

use crate::audio::gstreamer::{enum_command, launch_command};

pub struct AudioDeviceMonitorHandle {
    stop_tx: tokio::sync::oneshot::Sender<()>,
}

impl AudioDeviceMonitorHandle {
    pub fn stop(self) {
        let _ = self.stop_tx.send(());
    }
}

pub enum AudioDeviceType {
    Sink,
    Source,
}

#[derive(serde::Serialize)]
pub struct AudioDevice {
    pub id: String,
    pub name: String,
    pub is_default: bool,
    pub offline: Option<bool>,
}

const ENUMERATE_TIMEOUT_MS: u64 = 4000;

pub async fn list_audio_devices(kind: AudioDeviceType) -> Result<Vec<AudioDevice>, String> {
    let (bin, envs) = enum_command("gst-device-monitor-1.0");

    let filter = match kind {
        AudioDeviceType::Sink => "Audio/Sink",
        AudioDeviceType::Source => "Audio/Source",
    };

    let mut child = tokio::process::Command::new(bin)
        .arg(filter)
        .envs(envs)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;

    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let mut stdout_lines = BufReader::new(stdout).lines();
    let mut stderr_lines = BufReader::new(stderr).lines();
    let mut output = String::new();
    let mut err_output = String::new();

    let deadline = tokio::time::Instant::now() + Duration::from_millis(ENUMERATE_TIMEOUT_MS);
    loop {
        tokio::select! {
            res = tokio::time::timeout_at(deadline, stdout_lines.next_line()) => {
                match res {
                    Ok(Ok(Some(line))) => {
                        output.push_str(&line);
                        output.push('\n');
                        if output.len() > 1024 * 1024 {
                            let _ = child.kill().await;
                            return Err("Output too large".to_string());
                        }
                    }
                    _ => break,
                }
            }
            res = stderr_lines.next_line() => {
                if let Ok(Some(line)) = res {
                    err_output.push_str(&line);
                    err_output.push('\n');
                }
            }
        }
    }
    let _ = child.kill().await;

    Ok(parse_device_monitor_output(&output, kind))
}

pub fn parse_device_monitor_output(output: &str, kind: AudioDeviceType) -> Vec<AudioDevice> {
    let mut devices = vec![];
    let re = Regex::new(r"(?m)^\s*Device found:\s*$").unwrap();
    let blocks: Vec<&str> = re.split_inclusive(output).collect();
    let default_re = Regex::new(r"(?im)^\s*default:\s*true\s*$").unwrap();

    for block in blocks {
        if block.trim().is_empty() {
            continue;
        }

        let cls = match_prop(block, "class");
        if cls.is_none() {
            continue;
        }
        let expected_class = match kind {
            AudioDeviceType::Sink => "Audio/Sink",
            AudioDeviceType::Source => "Audio/Source",
        };
        if !cls.unwrap().contains(expected_class) {
            continue;
        }

        match kind {
            AudioDeviceType::Sink => {}
            AudioDeviceType::Source => {
                if match_prop(block, "device.class") == Some("monitor".to_owned()) {
                    continue;
                }
            }
        }

        let id = id_from_launch_line(block)
            .or_else(|| match_prop(block, "unique-id"))
            .or_else(|| match_prop(block, "device.name"))
            .or_else(|| match_prop(block, "node.name"))
            .or_else(|| match_prop(block, "alsa.card_name"))
            .or_else(|| match_prop(block, "name"));

        if id.is_none() {
            continue;
        }

        let name = match_prop(block, "name")
            .or_else(|| match_prop(block, "device.description"))
            .or_else(|| match_prop(block, "node.description"))
            .or_else(|| match_prop(block, "alsa.card_name"))
            .or_else(|| id.clone());

        if name.is_none() {
            continue;
        }

        let is_default = default_re.is_match(block)
            || match_prop(block, "is-default") == Some("true".to_owned())
            || match_prop(block, "is-default") == Some("true (gboolean)".to_owned())
            || match_prop(block, "node.is-default") == Some("true".to_owned());

        let id = id.unwrap();

        let device = AudioDevice {
            id,
            name: name.unwrap(),
            is_default,
            offline: None,
        };

        devices.push(device);
    }

    devices
}

// gst-device-monitor prints a sample launch line per device, e.g.
//   gst-launch-1.0 ... ! osxaudiosink device=53
//   gst-launch-1.0 ... ! 'pulsesink device=alsa_output.platform-fef00700.hdmi.hdmi-stereo'
//   gst-launch-1.0 ... ! wasapisink device-name=\{0.0.0.00000000\}.\{abc-def\}
// We pull whatever follows device= / device-name= and stop at the first
// unquoted whitespace, end of line, or closing quote.
fn id_from_launch_line(block: &str) -> Option<String> {
    let launch_re = Regex::new(r"gst-launch-1\.0[^\n]*").unwrap();
    let line = launch_re.find(block)?.as_str();

    let id_re =
        Regex::new(r#"\b(?:unique-id|device-name|device)=(?:"([^"]*)"|'([^']*)'|([^\s'"]+))"#)
            .unwrap();
    let caps = id_re.captures(line)?;

    caps.get(1)
        .or(caps.get(2))
        .or(caps.get(3))
        .map(|m| m.as_str().to_string())
}

fn match_prop(block: &str, key: &str) -> Option<String> {
    let escaped = regex::escape(key);
    let re = Regex::new(&format!(r"(?m)^\s*{}\s*[:=]\s*(.+?)\s*$", escaped)).unwrap();
    let caps = re.captures(block)?;
    let val = caps.get(1)?.as_str();
    let val = val
        .strip_prefix('"')
        .and_then(|v| v.strip_suffix('"'))
        .unwrap_or(val);
    Some(val.to_string())
}

const TOPOLOGY_EVENT_RE: &str = r"(?m)^\s*Device\s+\S+\s*:\s*$";
const DEBOUNCE_MS: u64 = 250;
const RESTART_DELAY_MS: u64 = 2_000;

pub fn start_audio_device_monitor<F>(on_change: F) -> AudioDeviceMonitorHandle
where
    F: Fn() + Send + Sync + 'static,
{
    let (stop_tx, mut stop_rx) = tokio::sync::oneshot::channel::<()>();

    let (bin, envs) = launch_command("gst-device-monitor-1.0");

    let on_change: Arc<F> = Arc::new(on_change);

    crate::runtime::spawn(async move {
        let topology_re = Regex::new(TOPOLOGY_EVENT_RE).unwrap();
        let mut debounce: Option<tokio::task::JoinHandle<()>> = None;

        loop {
            let mut child = match tokio::process::Command::new(&bin)
                .args(["-f", "Audio/Sink", "Audio/Source"])
                .envs(envs.clone())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::null())
                .stdin(std::process::Stdio::null())
                .spawn()
            {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("[AudioDeviceMonitor] spawn failed: {e}");
                    sleep(Duration::from_millis(RESTART_DELAY_MS));
                    continue;
                }
            };

            let stdout = child.stdout.take().unwrap();
            let mut lines = BufReader::new(stdout).lines();
            let on_change = Arc::clone(&on_change);

            loop {
                tokio::select! {
                    _ = &mut stop_rx => {
                        let _ = child.kill().await;
                        return;
                    }
                    line = lines.next_line() => {
                        match line {
                            Ok(Some(line)) => {
                                if topology_re.is_match(&(line.clone() + "\n")) {
                                    if let Some(h) = debounce.take() { h.abort(); }
                                    let on_change = Arc::clone(&on_change);
                                    debounce = Some(crate::runtime::spawn(async move {
                                        sleep(Duration::from_millis(DEBOUNCE_MS));
                                        on_change();
                                    }));
                                }
                            }
                            _ => break,
                        }
                    }
                }
            }

            let _ = child.wait().await;
            sleep(Duration::from_millis(RESTART_DELAY_MS));
        }
    });

    AudioDeviceMonitorHandle { stop_tx }
}

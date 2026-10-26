use std::vec;

use tokio::io::{AsyncBufReadExt, BufReader};

use crate::{
    audio::gstreamer::{audio_device_prop, audio_source_element},
    shared::audio_decode::AudioFormat,
};

struct Microphone {
    process: Option<tokio::process::Child>,
    stdin: Option<tokio::process::ChildStdin>,
    current_decode_type: i32,
    bytes_read: usize,
    chunk_seq: u64,
    device: Option<String>,
}

impl Microphone {
    pub fn new() -> Self {
        Microphone {
            process: None,
            stdin: None,
            current_decode_type: 5, // 5 = PCM signed 16-bit little-endian
            bytes_read: 0,
            chunk_seq: 0,
            device: None,
        }
    }

    fn set_device(&mut self, device: Option<String>) {
        self.device = device;
    }

    pub async fn start(&mut self, decode_type: Option<i32>) {
        let decode_type = decode_type.unwrap_or(5);

        let format = resolve_format(decode_type);
        self.current_decode_type = decode_type;

        let (cmd, env) = crate::audio::gstreamer::launch_command("gst-launch-1.0");

        let mut args: Vec<String> = vec![];

        let mut source_args = vec![audio_source_element().to_string()];

        if self.device.is_some() {
            let device = self.device.clone().unwrap();
            let device_args = format!("{}={}", audio_device_prop(), device);
            source_args.push(device_args);
        }

        args.push("-q".to_string());
        args.extend(source_args);
        args.push("!".to_string());
        args.push("queue".to_string());
        args.push("max-size-time=20000000".to_string());
        args.push("max-size-bytes=0".to_string());
        args.push("max-size-buffers=0".to_string());
        args.push("leaky=downstream".to_string());
        args.push("!".to_string());
        args.push("audioconvert".to_string());
        args.push("!".to_string());
        args.push("audioresample".to_string());
        args.push("!".to_string());
        args.push(format!(
            "{},rate={},channels={}",
            to_gst_raw_format(&format),
            format.frequency,
            format.channels
        ));
        args.push("!".to_string());
        args.push("fdsink".to_string());
        args.push("fd=1".to_string());


        self.bytes_read = 0;
        self.chunk_seq = 0;

        let mut command = tokio::process::Command::new(&cmd);
        command
            .args(&args)
            .envs(env)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);

        let mut child = match command.spawn() {
            Ok(c) => c,
            Err(e) => {
                eprintln!("[AudioOutput] Failed to spawn: {e}");
                return;
            }
        };

        let stdin = child.stdin.take();
        let stderr = child.stderr.take();

        self.process = Some(child);
        self.stdin = stdin;

        if let Some(stderr) = stderr {
            crate::runtime::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    eprintln!("[AudioOutput] STDERR: {}", line.trim());
                }
            });
        }
    }

    pub async fn stop(&mut self) {
        if let Some(mut child) = self.process.take() {
            let result = child.kill().await;

            if result.is_err() {
                eprintln!("[Microphone] Failed to kill process: {:?}", result);
            }
        }
        self.cleanup();
    }

    pub fn is_capturing(&self) -> bool {
        self.process.is_some()
    }

    fn cleanup(&mut self) {
        self.process = None;
        self.stdin = None;
        self.bytes_read = 0;
        self.chunk_seq = 0;
    }
}

fn resolve_format(decode_type: i32) -> AudioFormat {
    crate::shared::audio_decode::decode_type_map(decode_type).unwrap_or(AudioFormat {
        frequency: 16000,
        channels: 1,
        bit_depth: 16,
        format: Some("s16le".to_string()),
        mime_type: None,
    })
}

fn to_gst_raw_format(format: &AudioFormat) -> String {
    let raw = format
        .format
        .clone()
        .unwrap_or("s16le".to_string())
        .to_lowercase();

    if raw == "s16le" || raw == "s16_le" {
        return "S16LE".to_string();
    }

    raw.to_uppercase()
}

pub fn get_sys_default_prettty_name() -> String {
    "system default".to_string()
}

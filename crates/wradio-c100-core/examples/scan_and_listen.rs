//! Headless CLI example against the core crate directly (no Tauri/webview).
//!
//! Two modes:
//! - `--scan`: sweeps the full Band III channel table (skipping the 3
//!   intermediate/N frequencies, [`tables::DAB_N_FREQUENCIES_IDX`]),
//!   briefly collecting the FIC on every channel that locks and upserting
//!   every service found into the persisted station store (`store.rs`),
//!   then exits. Simplified from the reference's own `scanNext()`: that
//!   function only skips an N-frequency index when the *previous*
//!   channel's ensemble collection fully finished
//!   (`if(m_ensembleCollectFinished)` gating the skip check) -- a quirk of
//!   its threaded scan-command-queue state machine that doesn't fit this
//!   synchronous polling loop and isn't obviously intentional behavior to
//!   preserve (an N-frequency isn't a real standalone channel regardless
//!   of what happened on the channel before it), so here every N-frequency
//!   index is skipped unconditionally instead.
//! - Default (no `--scan`): tunes to one frequency (with auto-retry, see
//!   [`Tuner::tune_and_wait_for_lock`]) -- defaulting to the last-played
//!   station's frequency from the store if `--frequency-khz` wasn't given
//!   (auto-tune on startup) -- collects the FIC into an ensemble/service
//!   database, selects a service (by `--service-name` substring match if
//!   given, else the first DAB+ audio service found), and decodes its MSC
//!   data (Firecode sync + Reed-Solomon + AU splitting) and metadata
//!   (X-PAD/DLS/MOT/slideshow), printing per-superframe stats and signal
//!   bars. Both modes reconnect automatically
//!   ([`Tuner::reconnect`]) if the device disconnects mid-run.
//!
//! A real native playback path (decode + `cpal` output) is a later stage
//! of the port (this crate exposes raw AAC access units, not decoded PCM
//! -- see `dabplus.rs`'s own doc comment on why); this file is kept as
//! the running example those and other later stages build directly on
//! top of, per the port plan. In the meantime, `--dump-aac -` writes
//! ADTS-framed audio to stdout (every other line this example prints
//! goes to stderr precisely so this is safe), which an external player
//! can decode for actual live listening, e.g.:
//! `cargo run --example scan_and_listen -- --dump-aac - 2>status.log | ffplay -nodisp -autoexit -i -`.

use std::collections::BTreeSet;
use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;

use clap::Parser;
use wradio_c100_core::dabplus::{self, DabPlusDecoder};
use wradio_c100_core::ensemble::Ensemble;
use wradio_c100_core::fic;
use wradio_c100_core::pad;
use wradio_c100_core::service::ServiceComponentKind;
use wradio_c100_core::store::{StationRef, Store};
use wradio_c100_core::tables;
use wradio_c100_core::{Error, Tuner};

fn default_store_path() -> PathBuf {
    match std::env::var_os("HOME") {
        Some(home) => PathBuf::from(home).join(".local/share/wradio-c100/stations.json"),
        None => PathBuf::from("wradio-c100-stations.json"),
    }
}

fn format_bars(bars: Option<u8>) -> String {
    bars.map(|l| l.to_string()).unwrap_or_else(|| "-".into())
}

#[derive(Parser)]
struct Args {
    /// Sweep the full Band III channel table and exit, instead of tuning
    /// to (and decoding) a single frequency.
    #[arg(long)]
    scan: bool,

    /// Band III channel frequency to tune to, in kHz (e.g. 178352 for 5C).
    /// Ignored with `--scan`. Defaults to the last-played station's
    /// frequency if the station store has one, otherwise the first
    /// channel in the table.
    #[arg(long)]
    frequency_khz: Option<u32>,

    /// Select a service by a case-insensitive substring of its label,
    /// instead of the first DAB+ audio service found. Ignored with
    /// `--scan`.
    #[arg(long)]
    service_name: Option<String>,

    /// Path to the persisted station database (scan results/favorites/
    /// presets, see `store.rs`). Defaults to
    /// `~/.local/share/wradio-c100/stations.json`.
    #[arg(long)]
    store_path: Option<PathBuf>,

    /// How long to keep polling the FIC per channel before giving up
    /// (either after full ensemble collection or this timeout), in
    /// seconds.
    #[arg(long, default_value_t = 15)]
    fic_timeout_secs: u64,

    /// How long to poll MSC data for the selected service, once its
    /// subchannel is open, in seconds. Ignored with `--scan`.
    #[arg(long, default_value_t = 15)]
    msc_timeout_secs: u64,

    /// How many tune-and-wait-for-lock attempts to make per frequency
    /// (each with its own lock timeout) before giving up on it.
    #[arg(long, default_value_t = 3)]
    tune_attempts: u32,

    /// Write decoded audio, ADTS-framed as raw AAC, to this path -- or to
    /// stdout if the path is exactly `-`, e.g. for live playback piped
    /// straight into a player:
    /// `scan_and_listen --dump-aac - 2>/dev/null | ffplay -nodisp -autoexit -i -`.
    /// (All of this example's own status output goes to stderr precisely
    /// so stdout is safe to pipe like this.) Not a permanent playback path
    /// -- this crate doesn't decode AAC to PCM itself, see `dabplus.rs`'s
    /// own doc comment on why; ADTS framing is reconstructed here from
    /// `DabPlusOutput::access_unit_lengths` purely for this dump. Ignored
    /// with `--scan`.
    #[arg(long)]
    dump_aac: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let args = Args::parse();
    let store_path = args.store_path.clone().unwrap_or_else(default_store_path);
    let mut store = Store::open(&store_path);
    eprintln!("Station store: {}", store_path.display());

    eprintln!("Opening wRadio C100 device...");
    let mut tuner = Tuner::open().await?;

    eprintln!("Powering up and initializing tuner chip...");
    loop {
        if tuner.initialize().await? {
            break;
        }
        eprintln!("Power-up not yet successful, retrying...");
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    eprintln!("Tuner initialized.");

    if args.scan {
        run_scan(
            &mut tuner,
            &mut store,
            Duration::from_secs(args.fic_timeout_secs),
        )
        .await?;
        store.save()?;
        eprintln!(
            "Scan complete. {} station(s) known.",
            store.stations().len()
        );
        return Ok(());
    }

    // Auto-tune on startup: retune to the last-played station's frequency
    // if the caller didn't ask for a specific one and the store has one.
    let khz = args.frequency_khz.unwrap_or_else(|| {
        store
            .last_played()
            .and_then(|station| store.station(station))
            .and_then(|record| record.frequencies_khz.first().copied())
            .unwrap_or_else(|| Tuner::channel_table_khz()[0])
    });

    eprintln!(
        "Tuning to {khz} kHz (up to {} attempt(s))...",
        args.tune_attempts
    );
    let locked = tuner
        .tune_and_wait_for_lock(khz, Duration::from_secs(10), args.tune_attempts)
        .await?;
    let quality = tuner.signal_quality().await?;
    eprintln!(
        "lock: {} signal={}",
        if locked { "achieved" } else { "never achieved" },
        format_bars(quality.signal_bars)
    );

    if !locked {
        eprintln!("Never achieved channel lock -- skipping FIC read.");
        return Ok(());
    }

    eprintln!("Polling FIC (up to {}s)...", args.fic_timeout_secs);
    let mut ensemble = Ensemble::new();
    ensemble.set_frequency_khz(khz);
    let mut printed_services: BTreeSet<u32> = BTreeSet::new();
    let mut printed_ensemble_label = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(args.fic_timeout_secs);

    while tokio::time::Instant::now() < deadline {
        let fic_bytes = match tuner.poll_fic().await {
            Ok(bytes) => bytes,
            Err(Error::DeviceDisconnected) => {
                reconnect_with_message(&mut tuner).await?;
                continue;
            }
            Err(e) => return Err(e.into()),
        };
        if !fic_bytes.is_empty() {
            let events = fic::parse_fic(&fic_bytes);
            ensemble.apply(&events);

            if !printed_ensemble_label && !ensemble.ensemble_label().is_empty() {
                eprintln!(
                    "Ensemble: {:04X} \"{}\" (ECC {:02X})",
                    ensemble.ensemble_id(),
                    ensemble.ensemble_label(),
                    ensemble.ensemble_ecc()
                );
                printed_ensemble_label = true;
            }

            for service in ensemble.services() {
                store.upsert_station(
                    StationRef {
                        ensemble_id: ensemble.ensemble_id(),
                        service_id: service.service_id,
                    },
                    &service.label,
                    service.programme_type_code,
                    ensemble.service_has_dab_plus_audio(service),
                    khz,
                );

                if printed_services.insert(service.service_id) {
                    eprintln!(
                        "  Service {:06X}: \"{}\" ({})",
                        service.service_id,
                        service.label,
                        service.programme_type_full_name()
                    );
                }
            }

            if ensemble.is_collect_finished() {
                eprintln!("Ensemble collection finished.");
                break;
            }
        }
    }

    if !ensemble.ensemble_label().is_empty() || !printed_services.is_empty() {
        eprintln!(
            "Done. {} service(s) found in ensemble \"{}\".",
            printed_services.len(),
            ensemble.ensemble_label()
        );
    } else {
        eprintln!("Done. No FIC data decoded within the timeout.");
    }

    // Select a service: by `--service-name` substring match if given
    // (closing the original README's "Support finding the service ID by
    // name from command line" TODO), else the first service with a DAB+
    // (ascty 0x3F) audio stream component whose subchannel placement/
    // bitrate is fully resolved (FIG 0/1 has been seen for it).
    let name_filter = args.service_name.as_deref().map(str::to_lowercase);
    let mut candidate: Option<(u32, String, u8, u16)> = None;
    'outer: for service in ensemble.services() {
        if let Some(name) = &name_filter {
            if !service.label.to_lowercase().contains(name.as_str()) {
                continue;
            }
        }
        for &key in &service.component_keys {
            let Some(component) = ensemble.component(key) else {
                continue;
            };
            if let ServiceComponentKind::StreamAudio {
                audio_service_component_type,
            } = component.kind
            {
                if audio_service_component_type == dabplus::ASCTY_DAB_PLUS
                    && component.subchannel_id != 0xFF
                    && component.subchannel_bitrate > 0
                {
                    candidate = Some((
                        service.service_id,
                        service.label.clone(),
                        component.subchannel_id,
                        component.subchannel_bitrate,
                    ));
                    break 'outer;
                }
            }
        }
    }

    let Some((service_id, label, subchan_id, bitrate_kbits)) = candidate else {
        match &args.service_name {
            Some(name) => eprintln!("No DAB+ audio service matching \"{name}\" found."),
            None => eprintln!("No DAB+ audio service found to decode."),
        }
        store.save()?;
        return Ok(());
    };

    store.set_last_played(StationRef {
        ensemble_id: ensemble.ensemble_id(),
        service_id,
    });
    store.save()?;

    eprintln!(
        "Opening subchannel {subchan_id:#04x} for \"{}\" ({bitrate_kbits} kbit/s)...",
        label.trim()
    );
    tuner.open_subchannel(subchan_id).await?;
    let mut decoder = DabPlusDecoder::new(bitrate_kbits);
    let mut pad_decoder = pad::PadDecoder::new();
    let mut last_dls_text = String::new();
    let mut current_service_id = service_id;
    let mut aac_dump: Option<Box<dyn Write>> = match &args.dump_aac {
        Some(path) if path.as_os_str() == "-" => {
            eprintln!("Dumping decoded audio (ADTS AAC) to stdout...");
            Some(Box::new(std::io::BufWriter::new(std::io::stdout())))
        }
        Some(path) => {
            eprintln!("Dumping decoded audio (ADTS AAC) to {}...", path.display());
            Some(Box::new(std::io::BufWriter::new(std::fs::File::create(
                path,
            )?)))
        }
        None => None,
    };

    eprintln!(
        "Polling MSC (up to {}s). While running, type on stdin: 'list' (services on this ensemble), \
         'play <name substring>' (switch service on the fly -- no retune needed, same ensemble), \
         or 'quit'.",
        args.msc_timeout_secs
    );
    let mut superframe_count = 0u32;
    let msc_deadline = tokio::time::Instant::now() + Duration::from_secs(args.msc_timeout_secs);
    let mut last_bars_print = tokio::time::Instant::now();

    // Read stdin commands on a dedicated task, decoupled from the MSC
    // polling loop below via a channel. This matters for correctness, not
    // just style: `Hal::read_register`/`read_msc_data`/etc. are each a
    // `bulk_write` (send a request) immediately followed by a `bulk_read`
    // (fetch its reply) -- two separate `.await` points, not one atomic
    // USB transaction. Racing `tuner.poll_msc()` against stdin input in a
    // single `tokio::select!` (an earlier version of this loop did exactly
    // that) can cancel `poll_msc()`'s future between those two awaits the
    // moment a command line arrives -- the device has already queued its
    // reply, but nothing ever reads it out, so the *next* unrelated
    // register read (e.g. `close_subchannel`'s ack read) desyncs and reads
    // those stale bytes instead, surfacing as a fatal "short USB read"
    // error. Found live: typing `play 3FM` mid-poll crashed the session.
    // Keeping stdin on its own task and only checking for a pending
    // command via non-blocking `try_recv()` around a `poll_msc().await`
    // that's always allowed to run to completion avoids the race by
    // construction -- worst case a typed command waits one poll's worth
    // of latency (well under the OS's line-buffered stdin delay anyway).
    let (cmd_tx, mut cmd_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    tokio::spawn(async move {
        let mut stdin_lines =
            tokio::io::AsyncBufReadExt::lines(tokio::io::BufReader::new(tokio::io::stdin()));
        while let Ok(Some(line)) = stdin_lines.next_line().await {
            if cmd_tx.send(line).is_err() {
                break;
            }
        }
    });

    'session: while tokio::time::Instant::now() < msc_deadline {
        match cmd_rx.try_recv() {
            Ok(cmd) => {
                let cmd = cmd.trim();

                if cmd.is_empty() {
                    // no-op
                } else if cmd == "quit" || cmd == "q" || cmd == "exit" {
                    break 'session;
                } else if cmd == "list" {
                    for service in ensemble.services() {
                        eprintln!(
                            "  {}\"{}\" ({})",
                            if service.service_id == current_service_id {
                                "-> "
                            } else {
                                "   "
                            },
                            service.label.trim(),
                            service.programme_type_full_name()
                        );
                    }
                } else if let Some(name) = cmd.strip_prefix("play ") {
                    let name_lower = name.trim().to_lowercase();
                    let mut new_candidate: Option<(u32, String, u8, u16)> = None;
                    'find: for service in ensemble.services() {
                        if !service.label.to_lowercase().contains(&name_lower) {
                            continue;
                        }
                        for &key in &service.component_keys {
                            let Some(component) = ensemble.component(key) else {
                                continue;
                            };
                            if let ServiceComponentKind::StreamAudio {
                                audio_service_component_type,
                            } = component.kind
                            {
                                if audio_service_component_type == dabplus::ASCTY_DAB_PLUS
                                    && component.subchannel_id != 0xFF
                                    && component.subchannel_bitrate > 0
                                {
                                    new_candidate = Some((
                                        service.service_id,
                                        service.label.clone(),
                                        component.subchannel_id,
                                        component.subchannel_bitrate,
                                    ));
                                    break 'find;
                                }
                            }
                        }
                    }

                    match new_candidate {
                        Some((new_service_id, new_label, new_subchan_id, new_bitrate)) => {
                            eprintln!(
                                "Switching to \"{}\" (subchannel {new_subchan_id:#04x}, {new_bitrate} kbit/s)...",
                                new_label.trim()
                            );
                            // Same ensemble/frequency -- no retune needed,
                            // just close the old subchannel and open the
                            // new one, and reset the per-subchannel decode
                            // state (a fresh `DabPlusDecoder`/`PadDecoder`
                            // since Firecode sync and DLS/MOT state don't
                            // carry over to a different subchannel).
                            tuner.close_subchannel().await?;
                            tuner.open_subchannel(new_subchan_id).await?;
                            decoder = DabPlusDecoder::new(new_bitrate);
                            pad_decoder = pad::PadDecoder::new();
                            last_dls_text.clear();
                            current_service_id = new_service_id;
                            store.set_last_played(StationRef {
                                ensemble_id: ensemble.ensemble_id(),
                                service_id: current_service_id,
                            });
                            store.save()?;
                        }
                        None => eprintln!(
                            "No DAB+ audio service matching \"{name}\" found. Try 'list'."
                        ),
                    }
                } else {
                    eprintln!("Unknown command {cmd:?}. Try 'list', 'play <name>', or 'quit'.");
                }
            }
            Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {}
            Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                // stdin closed -- keep decoding, just stop looking for commands.
            }
        }

        // Always run this to completion (never raced against the command
        // check above) -- see the comment on `cmd_rx` above for why.
        let msc_bytes = match tuner.poll_msc().await {
            Ok(bytes) => bytes,
            Err(Error::DeviceDisconnected) => {
                // `Tuner::reconnect` restores the tuned frequency
                // and re-opens whichever subchannel was open when
                // it disconnected (it tracks both internally) --
                // nothing else to redo here.
                reconnect_with_message(&mut tuner).await?;
                continue;
            }
            Err(e) => return Err(e.into()),
        };

        if msc_bytes.is_empty() {
            if last_bars_print.elapsed() >= Duration::from_secs(5) {
                let quality = tuner.signal_quality().await?;
                eprintln!(
                    "  lock={} signal={}",
                    if quality.locked { "yes" } else { "no" },
                    format_bars(quality.signal_bars)
                );
                last_bars_print = tokio::time::Instant::now();
            }
            continue;
        }

        for output in decoder.push_msc_data(&msc_bytes) {
            superframe_count += 1;
            eprintln!(
                        "  Superframe #{superframe_count}: {} AAC bytes, {} PAD unit(s), {}ch {}Hz sbr={} ps={} rs_corrected={} rs_uncorrectable={}",
                        output.access_units.len(),
                        output.pad_units.len(),
                        output.channels,
                        output.sample_rate,
                        output.sbr_used,
                        output.ps_used,
                        output.rs_corrected_symbols,
                        output.rs_uncorrectable,
                    );

            for pad_bytes in &output.pad_units {
                let pad_out = pad_decoder.pad_data_input(pad_bytes);
                if let Some(update) = pad_out.dls_update {
                    if update.text != last_dls_text {
                        last_dls_text = update.text.clone();
                        eprintln!("    DLS: \"{}\"", update.text.trim());
                        for tag in &update.plus_tags {
                            eprintln!("      DL+ {:?}: \"{}\"", tag.content_type, tag.text);
                        }
                    }
                }
                for slide in &pad_out.slideshow_updates {
                    eprintln!(
                        "    Slideshow: \"{}\" ({}, {} bytes){}",
                        slide.content_name,
                        slide.mime_type,
                        slide.slideshow_data.len(),
                        if slide.is_categorized {
                            format!(", category {}/{}", slide.category_id, slide.slide_id)
                        } else {
                            String::new()
                        }
                    );
                }
            }

            if let Some(writer) = &mut aac_dump {
                // ADTS-wrapping lives in `wradio_c100_core::adts` now,
                // shared with `tauri-plugin-wradio-c100`'s native
                // playback path (its AAC decoder needs the same
                // container framing this dump does) -- this example no
                // longer keeps its own copy.
                let mut framed = Vec::new();
                wradio_c100_core::adts::wrap_superframe(&mut framed, &output);
                writer.write_all(&framed)?;
                // Flush every superframe instead of leaving it to
                // `BufWriter`'s default 8KB buffer: each superframe is
                // only ~1.2-1.9KB, so left unflushed it silently batches
                // 4-6 superframes' worth (roughly half a second of audio)
                // before releasing them to the pipe all at once. A
                // steady-rate consumer on the other end (e.g. `ffplay -i
                // -`) sees that as silence-then-a-burst rather than a
                // smooth stream, which is a textbook cause of audible
                // stutter/clicks -- reported live ("weird tst tsst" while
                // listening). Flushing here costs one extra syscall per
                // superframe (every ~120ms of audio) in exchange for
                // delivering each superframe to the pipe as soon as it's
                // decoded.
                writer.flush()?;
            }
        }
    }

    tuner.close_subchannel().await?;
    if let Some(mut writer) = aac_dump {
        writer.flush()?;
    }
    eprintln!("Done. Decoded {superframe_count} superframe(s).");

    // `std::process::exit` rather than falling off the end into an
    // implicit `Ok(())` return: the stdin-reading task spawned above (see
    // the comment on `cmd_rx`) is permanently parked in a blocking read
    // for its *next* line the moment this loop exits -- nothing will ever
    // arrive (a real terminal only EOFs on Ctrl-D, and my own fifo-based
    // test harness never EOFs at all), and that blocking read can't be
    // cancelled. `#[tokio::main]`'s generated wrapper drops the runtime
    // when `main` returns, and dropping a runtime blocks the calling
    // thread until every outstanding task -- including unkillable
    // blocking-pool ones -- finishes, so a plain `Ok(())` here hangs the
    // whole process forever after printing this exact "Done." line.
    // Reproduced live: typed `quit`, saw "Done. Decoded N superframe(s).",
    // then the process sat in `futex_wait` making no progress until
    // killed. All cleanup above (`close_subchannel`, flushing `aac_dump`)
    // already ran, so it's safe to terminate immediately instead of
    // waiting on a task that will never finish on its own.
    std::process::exit(0);
}

/// Waits for the device to reappear and re-initializes it, printing
/// progress -- call this when a `Tuner` method returns
/// `Error::DeviceDisconnected`.
async fn reconnect_with_message(tuner: &mut Tuner) -> anyhow::Result<()> {
    eprintln!("Device disconnected -- waiting for it to reappear...");
    tuner.reconnect(Duration::from_secs(2)).await?;
    eprintln!("Device reconnected and re-initialized.");
    Ok(())
}

/// Sweeps every Band III channel (skipping the 3 intermediate/N
/// frequencies -- see this file's own doc comment), briefly collecting
/// the FIC on any that lock and upserting every service found into
/// `store`.
async fn run_scan(
    tuner: &mut Tuner,
    store: &mut Store,
    per_channel_fic_timeout: Duration,
) -> anyhow::Result<()> {
    let channels = Tuner::channel_table_khz();
    let total = channels.len() - tables::DAB_N_FREQUENCIES_IDX.len();
    let mut scanned = 0usize;

    for (idx, &khz) in channels.iter().enumerate() {
        if tables::DAB_N_FREQUENCIES_IDX.contains(&idx) {
            continue;
        }
        scanned += 1;
        eprint!("[{scanned}/{total}] {khz} kHz... ");
        std::io::stdout().flush().ok();

        let locked = match tuner
            .tune_and_wait_for_lock(khz, Duration::from_secs(4), 1)
            .await
        {
            Ok(locked) => locked,
            Err(Error::DeviceDisconnected) => {
                eprintln!();
                reconnect_with_message(tuner).await?;
                false
            }
            Err(e) => return Err(e.into()),
        };

        if !locked {
            eprintln!("no signal");
            continue;
        }

        let quality = tuner.signal_quality().await?;
        let mut ensemble = Ensemble::new();
        ensemble.set_frequency_khz(khz);
        let deadline = tokio::time::Instant::now() + per_channel_fic_timeout;

        while tokio::time::Instant::now() < deadline {
            let fic_bytes = match tuner.poll_fic().await {
                Ok(bytes) => bytes,
                Err(Error::DeviceDisconnected) => {
                    eprintln!();
                    reconnect_with_message(tuner).await?;
                    continue;
                }
                Err(e) => return Err(e.into()),
            };
            if !fic_bytes.is_empty() {
                let events = fic::parse_fic(&fic_bytes);
                ensemble.apply(&events);
                if ensemble.is_collect_finished() {
                    break;
                }
            }
        }

        // `Store::upsert_station` silently ignores an unknown (0xFFFF)
        // ensemble id, so a channel that yielded FIG 0/2 (services) within
        // the timeout but not yet FIG 0/0 (the ensemble id itself) won't
        // get persisted this pass -- flagged below rather than left
        // silently unexplained.
        let ensemble_id_known = ensemble.ensemble_id() != 0xFFFF;
        for service in ensemble.services() {
            store.upsert_station(
                StationRef {
                    ensemble_id: ensemble.ensemble_id(),
                    service_id: service.service_id,
                },
                &service.label,
                service.programme_type_code,
                ensemble.service_has_dab_plus_audio(service),
                khz,
            );
        }

        eprintln!(
            "locked, signal={}, ensemble \"{}\", {} service(s){}",
            format_bars(quality.signal_bars),
            ensemble.ensemble_label().trim(),
            ensemble.services().count(),
            if ensemble_id_known {
                ""
            } else {
                " (ensemble id not yet resolved, not persisted this pass)"
            }
        );
    }

    Ok(())
}

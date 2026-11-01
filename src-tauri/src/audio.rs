//! Audio capture with cpal, streamed to FFmpeg over a local TCP socket.
//!
//! Each `AudioFeed` owns a thread that:
//! 1. opens the device (a microphone, or the default speaker in WASAPI loopback mode),
//! 2. waits for FFmpeg to connect to its localhost port,
//! 3. writes interleaved f32 samples paced by the wall clock.
//!
//! Pacing matters: WASAPI loopback delivers *nothing* while no sound is playing, which
//! would make the audio track shorter than the video and drift out of sync. The feed
//! fills such gaps with silence and drops samples if a device runs ahead of the clock.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::collections::VecDeque;
use std::io::{ErrorKind, Write};
use std::net::{Shutdown, TcpListener};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

#[derive(Clone, Debug)]
pub enum AudioKind {
    /// A microphone by device name (`None` = system default input).
    Microphone(Option<String>),
    /// Whatever is playing on the default output device (Windows WASAPI loopback).
    SystemLoopback,
}

pub struct AudioFeed {
    pub port: u16,
    pub sample_rate: u32,
    pub channels: u16,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl Drop for AudioFeed {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

pub fn input_device_names() -> Vec<String> {
    let host = cpal::default_host();
    match host.input_devices() {
        Ok(devices) => {
            let mut names: Vec<String> = devices.filter_map(|d| d.name().ok()).collect();
            names.dedup();
            names
        }
        Err(_) => Vec::new(),
    }
}

pub fn default_input_name() -> Option<String> {
    cpal::default_host().default_input_device().and_then(|d| d.name().ok())
}

/// System audio capture is implemented through WASAPI loopback, which only exists on Windows.
pub fn loopback_supported() -> bool {
    cfg!(target_os = "windows") && cpal::default_host().default_output_device().is_some()
}

fn find_device(kind: &AudioKind) -> Result<(cpal::Device, cpal::SupportedStreamConfig), String> {
    let host = cpal::default_host();
    match kind {
        AudioKind::Microphone(name) => {
            let named = name.as_ref().and_then(|wanted| {
                host.input_devices().ok()?.find(|d| d.name().map(|n| n == *wanted).unwrap_or(false))
            });
            let device = named
                .or_else(|| host.default_input_device())
                .or_else(|| host.input_devices().ok().and_then(|mut all| all.next()))
                .ok_or("No microphone was found.")?;
            let config = device
                .default_input_config()
                .map_err(|e| format!("The microphone could not be opened ({e})."))?;
            Ok((device, config))
        }
        AudioKind::SystemLoopback => {
            let device = host.default_output_device().ok_or("No speaker or headphone device was found.")?;
            let config = device
                .default_output_config()
                .map_err(|e| format!("The output device could not be opened ({e})."))?;
            Ok((device, config))
        }
    }
}

/// Opens the device and starts a feed. Returns once the device is producing audio
/// and the TCP port is ready for FFmpeg to connect to.
pub fn start_feed(kind: AudioKind) -> Result<AudioFeed, String> {
    let (_device, config) = find_device(&kind)?;
    let sample_rate = config.sample_rate().0;
    let channels = config.channels();

    let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;

    let stop = Arc::new(AtomicBool::new(false));
    let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();
    let thread_stop = stop.clone();
    let handle = thread::Builder::new()
        .name("tallycast-audio".into())
        .spawn(move || run_feed(kind, listener, sample_rate, channels, thread_stop, ready_tx))
        .map_err(|e| e.to_string())?;

    let mut feed = AudioFeed { port, sample_rate, channels, stop, handle: Some(handle) };
    match ready_rx.recv_timeout(Duration::from_secs(6)) {
        Ok(Ok(())) => Ok(feed),
        Ok(Err(e)) => Err(e), // `feed` drops here and joins the thread
        Err(_) => {
            // Detach instead of joining: the driver may be stuck inside the open call.
            feed.stop.store(true, Ordering::SeqCst);
            feed.handle.take();
            Err("The audio device did not respond.".into())
        }
    }
}

type SampleQueue = Arc<Mutex<VecDeque<f32>>>;

fn push_samples<I: Iterator<Item = f32>>(queue: &Mutex<VecDeque<f32>>, samples: I, max_len: usize) {
    if let Ok(mut q) = queue.lock() {
        q.extend(samples);
        if q.len() > max_len {
            let excess = q.len() - max_len;
            q.drain(..excess);
        }
    }
}

fn build_stream(kind: &AudioKind, queue: SampleQueue) -> Result<cpal::Stream, String> {
    let (device, supported) = find_device(kind)?;
    let format = supported.sample_format();
    let config: cpal::StreamConfig = supported.into();
    // Keep at most ~2 seconds buffered.
    let max_len = config.sample_rate.0 as usize * config.channels as usize * 2;
    let on_error = |e: cpal::StreamError| eprintln!("[tallycast] audio stream error: {e}");

    let stream = match format {
        cpal::SampleFormat::F32 => device.build_input_stream(
            &config,
            move |data: &[f32], _: &cpal::InputCallbackInfo| {
                push_samples(&queue, data.iter().copied(), max_len)
            },
            on_error,
            None,
        ),
        cpal::SampleFormat::I16 => device.build_input_stream(
            &config,
            move |data: &[i16], _: &cpal::InputCallbackInfo| {
                push_samples(&queue, data.iter().map(|s| *s as f32 / 32_768.0), max_len)
            },
            on_error,
            None,
        ),
        cpal::SampleFormat::U16 => device.build_input_stream(
            &config,
            move |data: &[u16], _: &cpal::InputCallbackInfo| {
                push_samples(&queue, data.iter().map(|s| (*s as f32 - 32_768.0) / 32_768.0), max_len)
            },
            on_error,
            None,
        ),
        cpal::SampleFormat::I32 => device.build_input_stream(
            &config,
            move |data: &[i32], _: &cpal::InputCallbackInfo| {
                push_samples(&queue, data.iter().map(|s| *s as f32 / 2_147_483_648.0), max_len)
            },
            on_error,
            None,
        ),
        other => return Err(format!("Unsupported audio sample format: {other:?}")),
    }
    .map_err(|e| format!("The audio device could not be opened ({e})."))?;
    Ok(stream)
}

fn run_feed(
    kind: AudioKind,
    listener: TcpListener,
    sample_rate: u32,
    channels: u16,
    stop: Arc<AtomicBool>,
    ready: mpsc::Sender<Result<(), String>>,
) {
    let queue: SampleQueue = Arc::new(Mutex::new(VecDeque::new()));

    // The cpal stream must live on this thread (it is !Send on some platforms).
    let stream = match build_stream(&kind, queue.clone()) {
        Ok(s) => s,
        Err(e) => {
            let _ = ready.send(Err(e));
            return;
        }
    };
    if let Err(e) = stream.play() {
        let _ = ready.send(Err(format!("The audio device could not start ({e}).")));
        return;
    }
    let _ = ready.send(Ok(()));

    // Wait for FFmpeg to connect.
    let wait_start = Instant::now();
    let mut socket = loop {
        if stop.load(Ordering::SeqCst) {
            return;
        }
        match listener.accept() {
            Ok((s, _)) => break s,
            Err(e) if e.kind() == ErrorKind::WouldBlock => {
                if wait_start.elapsed() > Duration::from_secs(30) {
                    eprintln!("[tallycast] FFmpeg never connected to the audio feed");
                    return;
                }
                thread::sleep(Duration::from_millis(5));
            }
            Err(e) => {
                eprintln!("[tallycast] audio feed accept failed: {e}");
                return;
            }
        }
    };
    let _ = socket.set_nonblocking(false);
    let _ = socket.set_nodelay(true);
    let _ = socket.set_write_timeout(Some(Duration::from_secs(3)));

    // Sample 0 of the stream corresponds to "now".
    if let Ok(mut q) = queue.lock() {
        q.clear();
    }
    let t0 = Instant::now();
    let ch = channels as u64;
    let rate = sample_rate as u64;
    let max_lead = rate / 5; // allow 200 ms ahead of the wall clock
    let max_lag = rate / 10; // pad with silence when 100 ms behind
    let mut written: u64 = 0; // frames
    let mut out: Vec<u8> = Vec::with_capacity((rate * ch * 4 / 10) as usize);

    while !stop.load(Ordering::SeqCst) {
        thread::sleep(Duration::from_millis(10));
        let target = (t0.elapsed().as_secs_f64() * sample_rate as f64) as u64;
        out.clear();

        if let Ok(mut q) = queue.lock() {
            let available = q.len() as u64 / ch;
            let budget = (target + max_lead).saturating_sub(written);
            let take = available.min(budget);
            for s in q.drain(..(take * ch) as usize) {
                out.extend_from_slice(&s.to_le_bytes());
            }
            written += take;
            // Device is running ahead of the clock: discard the surplus.
            if q.len() as u64 / ch > rate / 2 {
                q.clear();
            }
        }

        // Device delivered nothing for a while (e.g. silent loopback): fill with silence.
        if written + max_lag < target {
            let missing = target - written;
            out.resize(out.len() + (missing * ch * 4) as usize, 0);
            written = target;
        }

        if !out.is_empty() && socket.write_all(&out).is_err() {
            break; // FFmpeg closed the connection
        }
    }

    drop(stream);
    let _ = socket.shutdown(Shutdown::Both);
}

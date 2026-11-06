//! Native Windows audio: WASAPI loopback ("what you hear") + optional microphone,
//! mixed in real time and streamed as f32le / 48 kHz / stereo into FFmpeg's stdin.
//!
//! WASAPI loopback delivers *no packets* while nothing is playing. The pump therefore
//! runs on the wall clock: every 10 ms it writes exactly the number of frames that
//! elapsed, filling gaps with silence, which keeps audio locked to the video timeline.

use crate::config::AudioConfig;
use std::collections::VecDeque;
use std::io::Write;
use std::process::ChildStdin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use windows::core::PCWSTR;
use windows::Win32::Media::Audio::*;
use windows::Win32::System::Com::*;

const RATE: u32 = 48_000;
const CHANNELS: usize = 2;
/// Bound each source queue to 300 ms to cap latency if a device runs fast.
const MAX_QUEUE: usize = (RATE as usize * CHANNELS * 3) / 10;

pub struct AudioPump {
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

struct Source {
    client: IAudioClient,
    capture: IAudioCaptureClient,
    queue: VecDeque<f32>,
    volume: f32,
}

fn float_format() -> WAVEFORMATEX {
    WAVEFORMATEX {
        wFormatTag: 3, // WAVE_FORMAT_IEEE_FLOAT
        nChannels: CHANNELS as u16,
        nSamplesPerSec: RATE,
        nAvgBytesPerSec: RATE * 8,
        nBlockAlign: 8,
        wBitsPerSample: 32,
        cbSize: 0,
    }
}

unsafe fn open_source(device: &IMMDevice, loopback: bool, volume: f32) -> windows::core::Result<Source> {
    let client: IAudioClient = device.Activate(CLSCTX_ALL, None)?;
    let fmt = float_format();
    let mut flags = AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY;
    if loopback {
        flags |= AUDCLNT_STREAMFLAGS_LOOPBACK;
    }
    // 200 ms shared-mode buffer (100-ns units).
    client.Initialize(AUDCLNT_SHAREMODE_SHARED, flags, 2_000_000, 0, &fmt, None)?;
    let capture: IAudioCaptureClient = client.GetService()?;
    client.Start()?;
    Ok(Source { client, capture, queue: VecDeque::with_capacity(MAX_QUEUE), volume })
}

unsafe fn drain(src: &mut Source) -> windows::core::Result<()> {
    loop {
        let n = src.capture.GetNextPacketSize()?;
        if n == 0 {
            return Ok(());
        }
        let mut data: *mut u8 = std::ptr::null_mut();
        let mut frames = 0u32;
        let mut flags = 0u32;
        src.capture.GetBuffer(&mut data, &mut frames, &mut flags, None, None)?;
        let samples = frames as usize * CHANNELS;
        if flags & (AUDCLNT_BUFFERFLAGS_SILENT.0 as u32) != 0 || data.is_null() {
            src.queue.extend(std::iter::repeat_n(0.0f32, samples));
        } else {
            let slice = std::slice::from_raw_parts(data as *const f32, samples);
            src.queue.extend(slice.iter().copied());
        }
        src.capture.ReleaseBuffer(frames)?;
        while src.queue.len() > MAX_QUEUE {
            src.queue.pop_front();
        }
    }
}

impl AudioPump {
    pub fn start(sink: ChildStdin, cfg: &AudioConfig) -> std::io::Result<Self> {
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = stop.clone();
        let cfg = cfg.clone();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<(), String>>();
        let handle = std::thread::Builder::new().name("vinci-wasapi".into()).spawn(move || unsafe {
            if let Err(e) = Self::run(sink, &cfg, &stop2, &ready_tx) {
                let _ = ready_tx.send(Err(e.to_string()));
                log::error!("WASAPI audio stopped: {e}");
            }
            CoUninitialize();
        })?;
        match ready_rx.recv_timeout(Duration::from_secs(5)) {
            Ok(Ok(())) => Ok(Self { stop, handle: Some(handle) }),
            Ok(Err(e)) => Err(std::io::Error::other(e)),
            Err(_) => Err(std::io::Error::other("WASAPI initialisation timed out")),
        }
    }

    unsafe fn run(
        mut sink: ChildStdin,
        cfg: &AudioConfig,
        stop: &AtomicBool,
        ready: &std::sync::mpsc::Sender<Result<(), String>>,
    ) -> windows::core::Result<()> {
        CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
        let enumerator: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        let mut sources: Vec<Source> = Vec::new();
        if cfg.system {
            let dev = match &cfg.system_device {
                Some(id) => {
                    let w: Vec<u16> = id.encode_utf16().chain(Some(0)).collect();
                    enumerator.GetDevice(PCWSTR(w.as_ptr()))?
                }
                None => enumerator.GetDefaultAudioEndpoint(eRender, eConsole)?,
            };
            sources.push(open_source(&dev, true, cfg.system_volume)?);
        }
        if cfg.microphone {
            let dev = match &cfg.mic_device {
                Some(id) => {
                    let w: Vec<u16> = id.encode_utf16().chain(Some(0)).collect();
                    enumerator.GetDevice(PCWSTR(w.as_ptr()))
                }
                None => enumerator.GetDefaultAudioEndpoint(eCapture, eConsole),
            };
            match dev.and_then(|d| open_source(&d, false, cfg.mic_volume)) {
                Ok(s) => sources.push(s),
                Err(e) => log::warn!("microphone unavailable, continuing without it: {e}"),
            }
        }
        let _ = ready.send(Ok(()));

        let start = Instant::now();
        let mut written: u64 = 0;
        let mut out: Vec<f32> = Vec::with_capacity(RATE as usize / 10 * CHANNELS);
        let mut bytes: Vec<u8> = Vec::with_capacity(out.capacity() * 4);
        while !stop.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(10));
            for s in sources.iter_mut() {
                drain(s)?;
            }
            let due = (start.elapsed().as_secs_f64() * RATE as f64) as u64;
            let frames = due.saturating_sub(written) as usize;
            if frames < 240 {
                continue;
            }
            out.clear();
            out.resize(frames * CHANNELS, 0.0);
            for s in sources.iter_mut() {
                for v in out.iter_mut() {
                    match s.queue.pop_front() {
                        Some(x) => *v += x * s.volume,
                        None => break,
                    }
                }
            }
            bytes.clear();
            for v in &out {
                bytes.extend_from_slice(&v.clamp(-1.0, 1.0).to_le_bytes());
            }
            if sink.write_all(&bytes).is_err() {
                break; // FFmpeg exited
            }
            written += frames as u64;
        }
        for s in &sources {
            let _ = s.client.Stop();
        }
        Ok(())
    }

    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

impl Drop for AudioPump {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Active audio endpoints: `(id, friendly name, is_input)`.
pub fn list_devices() -> Vec<(String, String, bool)> {
    let mut out = Vec::new();
    unsafe {
        let init = CoInitializeEx(None, COINIT_MULTITHREADED);
        let res = (|| -> windows::core::Result<()> {
            let enumerator: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
            for (flow, is_input) in [(eRender, false), (eCapture, true)] {
                let coll = enumerator.EnumAudioEndpoints(flow, DEVICE_STATE_ACTIVE)?;
                for i in 0..coll.GetCount()? {
                    let dev = coll.Item(i)?;
                    let id = dev.GetId()?.to_string().unwrap_or_default();
                    let store = dev.OpenPropertyStore(STGM_READ)?;
                    let name = store
                        .GetValue(&windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName)
                        .map(|v| v.to_string())
                        .unwrap_or_else(|_| id.clone());
                    out.push((id, name, is_input));
                }
            }
            Ok(())
        })();
        if let Err(e) = res {
            log::warn!("audio device enumeration failed: {e}");
        }
        if init.is_ok() {
            CoUninitialize();
        }
    }
    out
}

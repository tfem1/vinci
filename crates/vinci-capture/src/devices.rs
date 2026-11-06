//! Audio device listing for the Capture page.

use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct AudioDevice {
    /// Value to store in `AudioConfig::{system,mic}_device`.
    pub id: String,
    pub name: String,
    /// `true` for microphones/inputs, `false` for outputs (loopback / monitor sources).
    pub input: bool,
}

pub fn list_audio_devices() -> Vec<AudioDevice> {
    #[cfg(windows)]
    {
        crate::wasapi::list_devices().into_iter().map(|(id, name, input)| AudioDevice { id, name, input }).collect()
    }
    #[cfg(target_os = "linux")]
    {
        pulse_sources()
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        Vec::new()
    }
}

/// PulseAudio / PipeWire-Pulse sources via `pactl list short sources`
/// (`<idx>\t<name>\t<driver>\t<format>\t<state>`). Monitor sources = outputs.
#[cfg(target_os = "linux")]
fn pulse_sources() -> Vec<AudioDevice> {
    let Some(pactl) = vinci_hw::process::which("pactl") else { return Vec::new() };
    let Ok((true, out, _)) = vinci_hw::process::run_capture(vinci_hw::process::command(pactl).args(["list", "short", "sources"])) else {
        return Vec::new();
    };
    parse_pactl_short(&out)
}

#[cfg(any(target_os = "linux", test))]
pub(crate) fn parse_pactl_short(out: &str) -> Vec<AudioDevice> {
    out.lines()
        .filter_map(|l| {
            let name = l.split('\t').nth(1)?.trim().to_string();
            if name.is_empty() {
                return None;
            }
            let input = !name.ends_with(".monitor");
            let pretty = name.trim_end_matches(".monitor").rsplit('.').next().unwrap_or(&name).replace(['_', '-'], " ");
            Some(AudioDevice { name: if input { pretty } else { format!("{pretty} (output)") }, id: name, input })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn pactl() {
        let s = "0\talsa_output.pci-0000_00_1f.3.analog-stereo.monitor\tPipeWire\ts32le 2ch 48000Hz\tSUSPENDED\n1\talsa_input.usb-Blue_Yeti.analog-stereo\tPipeWire\ts16le 2ch 48000Hz\tRUNNING\n";
        let d = super::parse_pactl_short(s);
        assert_eq!(d.len(), 2);
        assert!(!d[0].input && d[0].name.ends_with("(output)"));
        assert!(d[1].input);
        assert_eq!(d[1].id, "alsa_input.usb-Blue_Yeti.analog-stereo");
    }
}

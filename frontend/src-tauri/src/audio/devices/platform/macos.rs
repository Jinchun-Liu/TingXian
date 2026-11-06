use anyhow::Result;
use cidre::core_audio as ca;

use crate::audio::devices::configuration::{AudioDevice, DeviceType};

/// List macOS audio devices without opening AudioUnits.
pub fn configure_macos_audio() -> Result<Vec<AudioDevice>> {
    let mut devices: Vec<AudioDevice> = Vec::new();

    // cpal's input_devices/output_devices probe supported formats by opening
    // AudioUnits. A blocked driver then stalls both settings and Stop. Listing
    // only needs the device name and stream channels, not a render unit.
    for device in ca::System::devices().map_err(|e| anyhow::anyhow!("无法读取音频设备：{e:?}"))? {
        let Ok(name) = device.name().map(|value| value.to_string()) else {
            continue;
        };
        if name == "meetily-audio-tap"
            && device.class().is_ok_and(|class| class == ca::Class::AGGREGATE_DEVICE)
        {
            continue;
        }
        if device.input_stream_cfg().is_ok_and(|cfg| cfg.buffers().iter().any(|b| b.number_channels > 0)) {
            devices.push(AudioDevice::new(name.clone(), DeviceType::Input));
        }
        if device.output_stream_cfg().is_ok_and(|cfg| cfg.buffers().iter().any(|b| b.number_channels > 0)) {
            devices.push(AudioDevice::new(name, DeviceType::Output));
        }
    }

    Ok(devices)
}

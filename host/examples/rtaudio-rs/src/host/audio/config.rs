use crate::host::RtAudioHost;
use clack_extensions::audio_ports::{
    AudioPortFlags, AudioPortInfoBuffer, AudioPortType, PluginAudioPorts,
};
use clack_host::prelude::{
    ClapId, PluginAudioConfiguration, PluginInstance, PluginMainThreadHandle,
};
// use cpal::traits::DeviceTrait;
// use cpal::{
//     BufferSize, Device, SampleFormat, SampleRate, StreamConfig, SupportedBufferSize,
//     SupportedStreamConfigRange,
// };
use std::cmp::Ordering;
use std::error::Error;
use std::fmt::{Display, Formatter};
use rtaudio::{DeviceInfo, SampleFormat};
use rtaudio::SampleFormat::Float32;

/// A full audio configuration.
///
/// This contains everything needed to set up a CPAL stream and CLAP plugin audio buffers.
pub struct FullAudioConfig {
    /// Configuration for the plugin's input ports.
    pub plugin_input_port_config: PluginAudioPortsConfig,
    /// Configuration for the plugin's output ports.
    pub plugin_output_port_config: PluginAudioPortsConfig,
    /// The number of output channels for the CPAL stream. Only 1 or 2 is supported.
    pub output_channel_count: usize,
    /// The minimum size of the buffer CPAL will process at once.
    pub min_buffer_size: u32,
    /// The likely maximum size of the buffer CPAL will process at once.
    /// Unlike min_buffer_size this isn't a hard limit: CPAL will occasionally give us more sample
    /// to process at once, but this should be very rare.
    pub max_likely_buffer_size: u32,
    /// The sample rate the stream will run at.
    pub sample_rate: u32,
    /// The sample format the stream will use.
    pub sample_format: SampleFormat,
}

impl FullAudioConfig {
    /// Attempts to find the best audio configuration for the given CPAL device and CLAP plugin to
    /// work together.
    pub fn find_best_from(
        device: &DeviceInfo,
        instance: &mut PluginInstance<RtAudioHost>,
    ) -> Result<Self, Box<dyn Error>> {
        let input_ports = get_config_from_ports(&mut instance.plugin_handle(), true);
        let output_ports = get_config_from_ports(&mut instance.plugin_handle(), false);

        Ok(find_matching_output_config(
            &device,
            output_ports,
            input_ports,
        ))
    }

    // todo: could add converter for DeviceParams and StreamOptions here
    /// Returns the CPAL stream configuration describing this configuration.
    // pub fn as_cpal_stream_config(&self) -> StreamConfig {
    //     StreamConfig {
    //         channels: self.output_channel_count as u16,
    //         buffer_size: BufferSize::Fixed(self.max_likely_buffer_size),
    //         sample_rate: SampleRate(self.sample_rate),
    //     }
    // }

    /// Returns the CLAP plugin audio configuration describing this configuration.
    pub fn as_clack_plugin_config(&self) -> PluginAudioConfiguration {
        PluginAudioConfiguration {
            sample_rate: self.sample_rate as f64,
            min_frames_count: self.min_buffer_size,
            max_frames_count: self.max_likely_buffer_size,
        }
    }
}

impl Display for FullAudioConfig {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} channels at {:.1}kHz, with buffer length of {}-{}, fed from plugin's \"{}\" port ({})",
            self.output_channel_count,
            self.sample_rate as f64 / 1_000.0,
            self.min_buffer_size,
            self.max_likely_buffer_size,
            &self.plugin_output_port_config.main_port().name,
            self.plugin_output_port_config.main_port().port_layout
        )
    }
}

/// The configuration of a set of plugin audio ports.
///
/// This can be describing either the plugin input ports or output ports.
#[derive(Clone, Debug)]
pub struct PluginAudioPortsConfig {
    /// A list of all audio ports the plugin exposes.
    pub ports: Vec<PluginAudioPortInfo>,
    /// The index of the Main audio port in the ports list.
    pub main_port_index: u32,
}

impl PluginAudioPortsConfig {
    /// An empty configuration, if the plugin does not expose ports.
    fn empty() -> Self {
        PluginAudioPortsConfig {
            main_port_index: 0,
            ports: vec![],
        }
    }

    /// The default port configuration, if the plugin does not implement the port extension.
    fn default() -> Self {
        PluginAudioPortsConfig {
            main_port_index: 0,
            ports: vec![PluginAudioPortInfo {
                _id: None,
                port_layout: AudioPortLayout::Stereo,
                name: "Default".into(),
            }],
        }
    }

    /// Returns the main audio port.
    ///
    /// This will panic if there are no ports available at all.
    pub fn main_port(&self) -> &PluginAudioPortInfo {
        &self.ports[self.main_port_index as usize]
    }

    /// Returns the total number of channels across all ports.
    pub fn total_channel_count(&self) -> usize {
        self.ports
            .iter()
            .map(|p| p.port_layout.channel_count() as usize)
            .sum()
    }
}

/// Information about a plugin's port.
#[derive(Clone, Debug)]
pub struct PluginAudioPortInfo {
    /// The plugin-provided ID of the port, if it has one.
    pub _id: Option<ClapId>,
    /// The layout of the port.
    pub port_layout: AudioPortLayout,
    /// The user-friendly name of the port.
    pub name: String,
}

/// The layout of a port, i.e. how the channels are organized.
#[derive(Eq, PartialEq, Copy, Clone, Debug)]
pub enum AudioPortLayout {
    /// A single mono channel.
    Mono,
    /// A pair of channels in stereo.
    Stereo,
    /// Another channel configuration with an arbitrary number of channels.
    Unsupported {
        /// The number of channels.
        channel_count: u16,
    },
}

impl AudioPortLayout {
    /// Returns the number of channels in this layout.
    pub fn channel_count(&self) -> u16 {
        match self {
            AudioPortLayout::Mono => 1,
            AudioPortLayout::Stereo => 2,
            AudioPortLayout::Unsupported { channel_count } => *channel_count,
        }
    }
}

impl Display for AudioPortLayout {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            AudioPortLayout::Mono => f.write_str("mono"),
            AudioPortLayout::Stereo => f.write_str("stereo"),
            AudioPortLayout::Unsupported { channel_count } => write!(f, "{channel_count}-channels"),
        }
    }
}

/// Retrieves a given plugin's port configuration using the Audio Ports extension.
///
/// This can query either the input ports or the output ports.
pub fn get_config_from_ports(
    plugin: &mut PluginMainThreadHandle,
    is_input: bool,
) -> PluginAudioPortsConfig {
    let Some(ports) = plugin.get_extension::<PluginAudioPorts>() else {
        return PluginAudioPortsConfig::default();
    };

    let mut buffer = AudioPortInfoBuffer::new();
    let mut main_port_index = None;
    let mut discovered_ports = vec![];

    for i in 0..ports.count(plugin, is_input) {
        let Some(info) = ports.get(plugin, i, is_input, &mut buffer) else {
            continue;
        };
        // If no port type is specified, we try to assume it from the channel count
        let port_type = info
            .port_type
            .or_else(|| AudioPortType::from_channel_count(info.channel_count));

        let port_layout = match port_type {
            Some(l) if l == AudioPortType::MONO => AudioPortLayout::Mono,
            Some(l) if l == AudioPortType::STEREO => AudioPortLayout::Stereo,
            _ => AudioPortLayout::Unsupported {
                channel_count: info.channel_count as u16,
            },
        };

        // Store which port is the main one, and throw a warning if one already exists.
        if info.flags.contains(AudioPortFlags::IS_MAIN) && main_port_index.replace(i).is_some() {
            eprintln!("Warning: plugin defines multiple main ports. This shouldn't be allowed");
        }

        discovered_ports.push(PluginAudioPortInfo {
            _id: Some(info.id),
            port_layout,
            name: String::from_utf8_lossy(info.name).into_owned(),
        })
    }

    if discovered_ports.is_empty() {
        if is_input {
            return PluginAudioPortsConfig::empty();
        }
        eprintln!("Warning: Plugin's audio port extension returned NO port at all? Using default stereo configuration instead.");
        return PluginAudioPortsConfig::default();
    }

    let main_port_index = if let Some(main_port_index) = main_port_index {
        main_port_index
    } else {
        eprintln!("Warning: Plugin's audio port extension defines no main port! Using the first decent port as a fallback.");
        if let Some(first_stereo_port) = discovered_ports
            .iter()
            .enumerate()
            .find(|(_, p)| p.port_layout == AudioPortLayout::Stereo)
        {
            first_stereo_port.0 as u32
        } else if let Some(first_mono_port) = discovered_ports
            .iter()
            .enumerate()
            .find(|(_, p)| p.port_layout == AudioPortLayout::Mono)
        {
            first_mono_port.0 as u32
        } else {
            eprintln!("Warning: No suitable mono or stereo port found. Will do my best.");
            0 // Assume the first port is good enough, whatever it is.
        }
    };

    PluginAudioPortsConfig {
        main_port_index,
        ports: discovered_ports,
    }
}

/// Finds the best CPAL configuration for the given output & input plugin ports.
fn find_matching_output_config(
    device: &DeviceInfo,
    plugin_output_port_config: PluginAudioPortsConfig,
    plugin_input_port_config: PluginAudioPortsConfig,
) -> FullAudioConfig {
    FullAudioConfig {
        output_channel_count: 2 as usize,
        min_buffer_size: 64,
        max_likely_buffer_size: 64,
        sample_rate: 44_100,
        plugin_output_port_config,
        plugin_input_port_config,
        sample_format: Float32,
    }
}

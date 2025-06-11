use crate::host::RtAudioHost;
use clack_host::prelude::*;
use clack_host::process::StartedPluginAudioProcessor;
// use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
// use cpal::{
//     BuildStreamError, Device, FromSample, OutputCallbackInfo, SampleFormat, Stream, StreamConfig,
// };
use std::error::Error;
use rtaudio::{Api, Buffers, DeviceParams, SampleFormat, StreamFlags, StreamHandle, StreamInfo, StreamOptions, StreamStatus};

/// Handling of audio buffers.
mod buffers;
/// Negotiation for audio stream and port configuration.
mod config;
/// MIDI handling.
mod midi;

use buffers::*;
use config::*;
use midi::*;

/// Activates the given plugin instance, and outputs its processed audio to a new CPAL stream.
pub fn activate_to_stream(
    instance: &mut PluginInstance<RtAudioHost>,
) -> Result<StreamHandle, Box<dyn Error>> {
    // Initialize CPAL
    // let cpal_host = cpal::default_host();

    // let cpal_host = cpal::host_from_id(cpal::HostId::Asio).expect("failed to initialise ASIO host");
    // println!("Using ASIO");
    //
    // let output_device = cpal_host.default_output_device().unwrap();

    // let config = FullAudioConfig::find_best_from(&output_device, instance)?;
    // println!("Using negotiated audio output settings: {config}");
    //
    // let midi = MidiReceiver::new(44_100, instance)?;
    //
    // let plugin_audio_processor = instance
    //     .activate(|_, _| (), config.as_clack_plugin_config())?
    //     .start_processing()?;
    //
    // let sample_format = config.sample_format;
    // let cpal_config = config.as_cpal_stream_config();
    // let audio_processor = StreamAudioProcessor::new(plugin_audio_processor, midi, config);
    //
    // let stream = build_output_stream_for_sample_format(
    //     &output_device,
    //     audio_processor,
    //     &cpal_config,
    //     sample_format,
    // )?;
    // stream.play()?;
    //
    // Ok(stream)


    let host = rtaudio::Host::new(Api::WindowsASIO).unwrap();
    dbg!(host.api());

    let output_device = host.default_output_device().unwrap();

    let config = FullAudioConfig::find_best_from(&output_device, instance)?;
    println!("Using negotiated audio output settings: {config}");

    let midi = MidiReceiver::new(44_100, instance)?;

    let plugin_audio_processor = instance
        .activate(|_, _| (), config.as_clack_plugin_config())?
        .start_processing()?;

    // let sample_format = config.sample_format;
    // let cpal_config = config.as_cpal_stream_config();
    let mut audio_processor = StreamAudioProcessor::new(plugin_audio_processor, midi, config);

    let mut stream_handle = host
        .open_stream(
            Some(DeviceParams {
                device_id: output_device.id,
                num_channels: 2,
                first_channel: 0,
            }),
            None,
            SampleFormat::Float32,
            44100,
            // 48000,
            // out_device.preferred_sample_rate,
            64,
            StreamOptions {
                flags: StreamFlags::empty(),  // interleaved
                // flags: StreamFlags::NONINTERLEAVED,
                num_buffers: 4,
                priority: -1,
                name: String::from("RtAudio-rs Client"),
            },
            |error| eprintln!("{}", error),
        )
        .unwrap();
    dbg!(stream_handle.info());

    stream_handle
        .start(
            move |buffers: Buffers<'_>, _info: &StreamInfo, _status: StreamStatus| {
                // todo
                // match buffers {
                //     Buffers::SInt8 { .. } => {}
                //     Buffers::SInt16 { .. } => {}
                //     Buffers::SInt24 { .. } => {}
                //     Buffers::SInt32 { .. } => {}
                //     Buffers::Float32 { .. } => {}
                //     Buffers::Float64 { .. } => {}
                // }

                if let Buffers::Float32 { output, input: _ } = buffers {
                    audio_processor.process(output);

                    // For non-interleaved buffers, channels are stored as separate arrays
                    // let frames_per_channel = output.len() / 2;

                    // Split the output buffer into left and right channel slices
                    // let (left_channel, right_channel) = output.split_at_mut(frames_per_channel);

                    // Generate samples for each channel separately
                    // for i in 0..frames_per_channel {
                        // Generate a sine wave at 440 Hz at 50% volume
                        // let val = (phasor * std::f32::consts::TAU).sin() * AMPLITUDE;
                        // phasor = (phasor + phasor_inc).fract();

                        // Write the same value to both channels
                        // left_channel[i] = val;
                        // right_channel[i] = val;
                    // }
                }
            },
        )
        .unwrap();

    Ok(stream_handle)
}

/// Holds all of the data, buffers and state that are going to live and get used on the audio thread.
struct StreamAudioProcessor {
    /// The plugin's audio processor.
    audio_processor: StartedPluginAudioProcessor<RtAudioHost>,
    /// The audio buffers.
    buffers: HostAudioBuffers,
    /// The MIDI event receiver.
    midi_receiver: Option<MidiReceiver>,
    /// A steady frame counter, used by the plugin's process() method.
    steady_counter: u64,
}

impl StreamAudioProcessor {
    /// Initializes the audio thread data.
    pub fn new(
        plugin_instance: StartedPluginAudioProcessor<RtAudioHost>,
        midi_receiver: Option<MidiReceiver>,
        config: FullAudioConfig,
    ) -> Self {
        Self {
            audio_processor: plugin_instance,
            buffers: HostAudioBuffers::from_config(config),
            midi_receiver,
            steady_counter: 0,
        }
    }

    /// Processes the given output buffer using the loaded plugin.
    ///
    /// Because CPAL gives different, arbitrary buffer lengths for each process call, this method
    /// first ensures the host internal buffers are big enough, and resizes and reallocates them if
    /// necessary.
    ///
    /// This method also collects all the MIDI events that have been received since the last
    /// process call., and feeds them to the plugin.
    pub fn process(&mut self, output_buffer: &mut [f32]) {
        self.buffers.ensure_buffer_size_matches(output_buffer.len());
        let sample_count = self.buffers.cpal_buf_len_to_frame_count(output_buffer.len());

        let (ins, mut outs) = self.buffers.prepare_plugin_buffers(output_buffer.len());

        let events = if let Some(midi) = self.midi_receiver.as_mut() {
            midi.receive_all_events(sample_count as u64)
        } else {
            InputEvents::empty()
        };

        match self.audio_processor.process(
            &ins,
            &mut outs,
            &events,
            &mut OutputEvents::void(),
            Some(self.steady_counter),
            None,
        ) {
            Ok(_) => self.buffers.write_to_rt_audio_buffer(output_buffer),
            Err(e) => eprintln!("{e}"),
        }

        self.steady_counter += sample_count as u64;
    }
}

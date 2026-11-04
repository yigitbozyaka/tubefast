use opus_decoder::OpusDecoder;
use symphonia::core::audio::{AsAudioBufferRef, AudioBuffer, AudioBufferRef, Channels, Signal, SignalSpec};
use symphonia::core::codecs::{CODEC_TYPE_OPUS, CodecDescriptor, CodecParameters, Decoder, DecoderOptions, FinalizeResult};
use symphonia::core::errors::{Error, Result};
use symphonia::core::formats::Packet;

const RATE: u32 = 48_000;
const MAX_FRAMES: usize = 5760;
const HEAD_BYTES: usize = 19;
const MAX_FAMILY_ZERO_CHANNELS: usize = 2;

pub struct OpusStream {
    params: CodecParameters,
    decoder: OpusDecoder,
    channels: usize,
    pre_skip: usize,
    skip: usize,
    first_ts: Option<u64>,
    gain: f32,
    declip: [f32; 2],
    pcm: Vec<f32>,
    out: AudioBuffer<f32>,
}

impl Decoder for OpusStream {
    fn try_new(params: &CodecParameters, _: &DecoderOptions) -> Result<Self> {
        let head = params
            .extra_data
            .as_deref()
            .filter(|h| h.len() >= HEAD_BYTES && h.starts_with(b"OpusHead"));
        let head = head.ok_or(Error::DecodeError("opus: missing OpusHead"))?;
        let channels = head[9] as usize;
        if channels == 0 || channels > MAX_FAMILY_ZERO_CHANNELS || head[18] != 0 {
            return Err(Error::Unsupported("opus: unsupported channel layout"));
        }
        let layout = if channels == 1 {
            Channels::FRONT_LEFT
        } else {
            Channels::FRONT_LEFT | Channels::FRONT_RIGHT
        };
        let pre_skip = u16::from_le_bytes([head[10], head[11]]) as usize;
        let gain_q8 = i16::from_le_bytes([head[16], head[17]]);
        Ok(Self {
            params: params.clone(),
            decoder: OpusDecoder::new(RATE, channels).map_err(|_| Error::Unsupported("opus: decoder setup failed"))?,
            channels,
            pre_skip,
            skip: pre_skip,
            first_ts: None,
            declip: [0.0; 2],
            gain: 10f32.powf(gain_q8 as f32 / (256.0 * 20.0)),
            pcm: vec![0.0; MAX_FRAMES * channels],
            out: AudioBuffer::new(MAX_FRAMES as u64, SignalSpec::new(RATE, layout)),
        })
    }

    fn supported_codecs() -> &'static [CodecDescriptor] {
        &[CodecDescriptor {
            codec: CODEC_TYPE_OPUS,
            short_name: "opus",
            long_name: "Opus",
            inst_func: |params, options| Ok(Box::new(OpusStream::try_new(params, options)?)),
        }]
    }

    fn reset(&mut self) {
        self.decoder.reset();
        self.declip = [0.0; 2];
        self.skip = 0;
    }

    fn codec_params(&self) -> &CodecParameters {
        &self.params
    }

    fn decode(&mut self, packet: &Packet) -> Result<AudioBufferRef<'_>> {
        if *self.first_ts.get_or_insert(packet.ts()) == packet.ts() {
            self.skip = self.pre_skip;
        }
        let frames = self
            .decoder
            .decode_float(packet.buf(), &mut self.pcm, false)
            .map_err(|_| Error::DecodeError("opus: bad packet"))?;
        let decoded = &mut self.pcm[..frames * self.channels];
        for sample in decoded.iter_mut() {
            *sample *= self.gain;
        }
        soft_clip(decoded, self.channels, &mut self.declip);
        let skipped = self.skip.min(frames);
        self.skip -= skipped;
        self.out.clear();
        self.out.render_reserved(Some(frames - skipped));
        for (channel, plane) in self.out.planes_mut().planes().iter_mut().enumerate() {
            for (frame, sample) in plane.iter_mut().enumerate() {
                *sample = self.pcm[(skipped + frame) * self.channels + channel];
            }
        }
        Ok(self.out.as_audio_buffer_ref())
    }

    fn finalize(&mut self) -> FinalizeResult {
        FinalizeResult::default()
    }

    fn last_decoded(&self) -> AudioBufferRef<'_> {
        self.out.as_audio_buffer_ref()
    }
}

fn soft_clip(pcm: &mut [f32], channels: usize, memory: &mut [f32; 2]) {
    let frames = pcm.len() / channels;
    if frames == 0 {
        return;
    }
    for sample in pcm.iter_mut() {
        *sample = sample.clamp(-2.0, 2.0);
    }
    for (channel, memory) in memory.iter_mut().enumerate().take(channels) {
        let at = |i: usize| i * channels + channel;
        let mut a = *memory;
        for i in 0..frames {
            let x = pcm[at(i)];
            if x * a >= 0.0 {
                break;
            }
            pcm[at(i)] = x + a * x * x;
        }
        let mut current = 0;
        let first = pcm[at(0)];
        loop {
            let Some(over) = (current..frames).find(|&i| pcm[at(i)].abs() > 1.0) else {
                a = 0.0;
                break;
            };
            let sign = pcm[at(over)];
            let (mut start, mut end, mut peak, mut peak_at) = (over, over, sign.abs(), over);
            while start > 0 && sign * pcm[at(start - 1)] >= 0.0 {
                start -= 1;
            }
            while end < frames && sign * pcm[at(end)] >= 0.0 {
                if pcm[at(end)].abs() > peak {
                    peak = pcm[at(end)].abs();
                    peak_at = end;
                }
                end += 1;
            }
            let touches_start = start == 0 && sign * pcm[at(0)] >= 0.0;
            a = (peak - 1.0) / (peak * peak);
            a += a * 2.4e-7;
            if sign > 0.0 {
                a = -a;
            }
            for i in start..end {
                let x = pcm[at(i)];
                pcm[at(i)] = x + a * x * x;
            }
            if touches_start && peak_at >= 2 {
                let mut offset = first - pcm[at(0)];
                let delta = offset / peak_at as f32;
                for i in current..peak_at {
                    offset -= delta;
                    pcm[at(i)] = (pcm[at(i)] + offset).clamp(-1.0, 1.0);
                }
            }
            current = end;
            if current == frames {
                break;
            }
        }
        *memory = a;
    }
}

#[cfg(test)]
mod tests {
    use super::soft_clip;

    #[test]
    fn soft_clip_keeps_samples_in_range_and_leaves_quiet_audio_alone() {
        let mut quiet: Vec<f32> = (0..960).map(|i| 0.5 * (i as f32 * 0.05).sin()).collect();
        let before = quiet.clone();
        let mut memory = [0.0; 2];
        soft_clip(&mut quiet, 2, &mut memory);
        assert_eq!(quiet, before);
        let mut loud: Vec<f32> = (0..960).map(|i| 1.6 * (i as f32 * 0.05).sin()).collect();
        soft_clip(&mut loud, 2, &mut memory);
        assert!(loud.iter().all(|s| s.abs() <= 1.0));
        assert!(loud.windows(4).all(|w| (w[0] - w[2]).abs() < 0.2));
    }
}

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
        let skipped = self.skip.min(frames);
        self.skip -= skipped;
        self.out.clear();
        self.out.render_reserved(Some(frames - skipped));
        for (channel, plane) in self.out.planes_mut().planes().iter_mut().enumerate() {
            for (frame, sample) in plane.iter_mut().enumerate() {
                *sample = self.pcm[(skipped + frame) * self.channels + channel] * self.gain;
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

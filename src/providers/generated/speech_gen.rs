//


use super::providers::ProviderName;

//
//
#
pub struct SpeechModelDef {
    pub model_id: &'static str,
    pub label: &'static str,
    pub output_mime: &'static str,
    ///
    pub sample_rate_hz: i64,
}

#
pub struct SpeechGenDef {
    pub wire_shape: &'static str,
    //
    pub audio_response_encoding: &'static str,
    //
    pub gen_endpoint: &'static str,
    //
    pub voices: &'static [&'static str],
    pub models: &'static [SpeechModelDef],
}

static INWORLD_SPEECH_VOICES: &[&str] = &["Alex", "Ashley", "Dennis"];

static INWORLD_SPEECH_MODELS: &[SpeechModelDef] = &[
    SpeechModelDef {
        model_id: "inworld-tts-1.5-max",
        label: "Inworld TTS 1.5 Max",
        output_mime: "audio/wav",
        sample_rate_hz: 0,
    },
    SpeechModelDef {
        model_id: "inworld-tts-1.5-mini",
        label: "Inworld TTS 1.5 Mini",
        output_mime: "audio/wav",
        sample_rate_hz: 0,
    },
    SpeechModelDef {
        model_id: "inworld-tts-2",
        label: "Inworld TTS 2",
        output_mime: "audio/wav",
        sample_rate_hz: 0,
    },
];

static INWORLD_SPEECH_GEN: SpeechGenDef = SpeechGenDef {
    wire_shape: "SpeechInworld",
    audio_response_encoding: "base64Envelope",
    gen_endpoint: "/tts/v1/voice",
    voices: INWORLD_SPEECH_VOICES,
    models: INWORLD_SPEECH_MODELS,
};

static OPENAI_SPEECH_VOICES: &[&str] = &["alloy", "ash", "ballad", "coral", "echo", "fable", "nova", "onyx", "sage", "shimmer"];

static OPENAI_SPEECH_MODELS: &[SpeechModelDef] = &[
    SpeechModelDef {
        model_id: "gpt-4o-mini-tts",
        label: "GPT-4o mini TTS",
        output_mime: "audio/mpeg",
        sample_rate_hz: 0,
    },
    SpeechModelDef {
        model_id: "tts-1",
        label: "TTS 1",
        output_mime: "audio/mpeg",
        sample_rate_hz: 0,
    },
    SpeechModelDef {
        model_id: "tts-1-hd",
        label: "TTS 1 HD",
        output_mime: "audio/mpeg",
        sample_rate_hz: 0,
    },
];

static OPENAI_SPEECH_GEN: SpeechGenDef = SpeechGenDef {
    wire_shape: "SpeechOpenAI",
    audio_response_encoding: "rawBody",
    gen_endpoint: "/v1/audio/speech",
    voices: OPENAI_SPEECH_VOICES,
    models: OPENAI_SPEECH_MODELS,
};

pub fn speech_gen_config(provider: ProviderName) -> Option<&'static SpeechGenDef> {
    match provider {
        ProviderName::Inworld => Some(&INWORLD_SPEECH_GEN),
        ProviderName::OpenAI => Some(&OPENAI_SPEECH_GEN),
        _ => None,
    }
}

// Code generated — DO NOT EDIT.

use super::providers::ProviderName;

// Speech wire-shape discriminators. Each fully determines the request
// body and the response audio path.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SpeechModelDef {
    pub model_id: &'static str,
    pub label: &'static str,
    pub output_mime: &'static str,
    /// ADR-037 advisory fact (queryable, never gated); 0 when unverified.
    pub sample_rate_hz: i64,
    /// Speakers a multi-speaker request declares; 0 = one voice only (BUG-091).
    pub max_speakers: i64,
    /// Whether a style (style, add_styled_line) is sent to this model (BUG-091).
    pub supports_speech_style: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SpeechGenDef {
    pub wire_shape: &'static str,
    // audio_response_encoding selects the response decoder: "base64Envelope" | "rawBody" | "base64InteractionSteps" (ADR-051).
    pub audio_response_encoding: &'static str,
    // gen_endpoint is an override; empty = use provider main endpoint.
    pub gen_endpoint: &'static str,
    pub models: &'static [SpeechModelDef],
}

static GOOGLE_SPEECH_MODELS: &[SpeechModelDef] = &[
    SpeechModelDef {
        model_id: "gemini-3.8-flash-lite-tts",
        label: "Gemini 3.8 Flash-Lite TTS",
        output_mime: "audio/wav",
        sample_rate_hz: 24000,
        max_speakers: 2,
        supports_speech_style: true,
    },
    SpeechModelDef {
        model_id: "gemini-3.8-flash-tts",
        label: "Gemini 3.8 Flash TTS",
        output_mime: "audio/wav",
        sample_rate_hz: 24000,
        max_speakers: 2,
        supports_speech_style: true,
    },
];

static GOOGLE_SPEECH_GEN: SpeechGenDef = SpeechGenDef {
    wire_shape: "SpeechGemini",
    audio_response_encoding: "base64InteractionSteps",
    gen_endpoint: "/v1beta/interactions",
    models: GOOGLE_SPEECH_MODELS,
};

static INWORLD_SPEECH_MODELS: &[SpeechModelDef] = &[
    SpeechModelDef {
        model_id: "inworld-tts-1.5-max",
        label: "Inworld TTS 1.5 Max",
        output_mime: "audio/wav",
        sample_rate_hz: 0,
        max_speakers: 0,
        supports_speech_style: false,
    },
    SpeechModelDef {
        model_id: "inworld-tts-1.5-mini",
        label: "Inworld TTS 1.5 Mini",
        output_mime: "audio/wav",
        sample_rate_hz: 0,
        max_speakers: 0,
        supports_speech_style: false,
    },
    SpeechModelDef {
        model_id: "inworld-tts-2",
        label: "Inworld TTS 2",
        output_mime: "audio/wav",
        sample_rate_hz: 0,
        max_speakers: 0,
        supports_speech_style: false,
    },
];

static INWORLD_SPEECH_GEN: SpeechGenDef = SpeechGenDef {
    wire_shape: "SpeechInworld",
    audio_response_encoding: "base64Envelope",
    gen_endpoint: "/tts/v1/voice",
    models: INWORLD_SPEECH_MODELS,
};

static OPENAI_SPEECH_MODELS: &[SpeechModelDef] = &[
    SpeechModelDef {
        model_id: "gpt-4o-mini-tts",
        label: "GPT-4o mini TTS",
        output_mime: "audio/mpeg",
        sample_rate_hz: 0,
        max_speakers: 0,
        supports_speech_style: true,
    },
    SpeechModelDef {
        model_id: "tts-1",
        label: "TTS 1",
        output_mime: "audio/mpeg",
        sample_rate_hz: 0,
        max_speakers: 0,
        supports_speech_style: false,
    },
    SpeechModelDef {
        model_id: "tts-1-hd",
        label: "TTS 1 HD",
        output_mime: "audio/mpeg",
        sample_rate_hz: 0,
        max_speakers: 0,
        supports_speech_style: false,
    },
];

static OPENAI_SPEECH_GEN: SpeechGenDef = SpeechGenDef {
    wire_shape: "SpeechOpenAI",
    audio_response_encoding: "rawBody",
    gen_endpoint: "/v1/audio/speech",
    models: OPENAI_SPEECH_MODELS,
};

pub fn speech_gen_config(provider: ProviderName) -> Option<&'static SpeechGenDef> {
    match provider {
        ProviderName::Google => Some(&GOOGLE_SPEECH_GEN),
        ProviderName::Inworld => Some(&INWORLD_SPEECH_GEN),
        ProviderName::OpenAI => Some(&OPENAI_SPEECH_GEN),
        _ => None,
    }
}

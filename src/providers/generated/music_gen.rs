//


use super::providers::ProviderName;

//
//
#
pub struct MusicModelDef {
    pub model_id: &'static str,
    pub label: &'static str,
    pub supports_lyrics: bool,
    pub max_duration_seconds: i64,
    pub output_mime: &'static str,
    ///
    ///
    pub sample_rate_hz: i64,
    pub available_output_formats: &'static [&'static str],
}

#
pub struct MusicGenDef {
    pub wire_shape: &'static str,
    //
    pub gen_endpoint: &'static str,
    pub models: &'static [MusicModelDef],
}

static GOOGLE_MUSIC_MODELS: &[MusicModelDef] = &[
    MusicModelDef {
        model_id: "lyria-3-clip-preview",
        label: "Lyria 3 Clip",
        supports_lyrics: true,
        max_duration_seconds: 30,
        output_mime: "audio/mpeg",
        sample_rate_hz: 0,
        available_output_formats: &["audio/mpeg"],
    },
    MusicModelDef {
        model_id: "lyria-3-pro-preview",
        label: "Lyria 3 Pro",
        supports_lyrics: true,
        max_duration_seconds: 120,
        output_mime: "audio/mpeg",
        sample_rate_hz: 0,
        available_output_formats: &["audio/mpeg"],
    },
];

static GOOGLE_MUSIC_GEN: MusicGenDef = MusicGenDef {
    wire_shape: "MusicGenerateContent",
    gen_endpoint: "",
    models: GOOGLE_MUSIC_MODELS,
};

static MINIMAX_MUSIC_MODELS: &[MusicModelDef] = &[
    MusicModelDef {
        model_id: "music-2.6",
        label: "MiniMax Music 2.6",
        supports_lyrics: true,
        max_duration_seconds: 0,
        output_mime: "audio/mpeg",
        sample_rate_hz: 44100,
        available_output_formats: &["audio/mpeg", "audio/wav"],
    },
];

static MINIMAX_MUSIC_GEN: MusicGenDef = MusicGenDef {
    wire_shape: "MusicMinimax",
    gen_endpoint: "https://api.minimax.io/v1/music_generation",
    models: MINIMAX_MUSIC_MODELS,
};

static VERTEX_MUSIC_MODELS: &[MusicModelDef] = &[
    MusicModelDef {
        model_id: "lyria-002",
        label: "Lyria 2",
        supports_lyrics: false,
        max_duration_seconds: 30,
        output_mime: "audio/wav",
        sample_rate_hz: 48000,
        available_output_formats: &["audio/wav"],
    },
];

static VERTEX_MUSIC_GEN: MusicGenDef = MusicGenDef {
    wire_shape: "MusicPredict",
    gen_endpoint: "",
    models: VERTEX_MUSIC_MODELS,
};

pub fn music_gen_config(provider: ProviderName) -> Option<&'static MusicGenDef> {
    match provider {
        ProviderName::Google => Some(&GOOGLE_MUSIC_GEN),
        ProviderName::Minimax => Some(&MINIMAX_MUSIC_GEN),
        ProviderName::Vertex => Some(&VERTEX_MUSIC_GEN),
        _ => None,
    }
}

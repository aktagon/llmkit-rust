//


use super::providers::ProviderName;

//
//
#
pub struct TranscriptionDef {
    pub wire_shape: &'static str,
    //
    pub interaction: &'static str,
    pub request_encoding: &'static str,
    pub submit_endpoint: &'static str,
    //
    pub poll_endpoint: &'static str,
    //
    pub upload_endpoint: &'static str,
    //
    pub submit_handle_field: &'static str,
    //
    pub status_path: &'static str,
    pub done_status: &'static str,
    pub error_status: &'static str,
}

static ASSEMBLYAI_TRANSCRIPTION_GEN: TranscriptionDef = TranscriptionDef {
    wire_shape: "TranscriptionAssemblyAI",
    interaction: "async",
    request_encoding: "json",
    submit_endpoint: "/v2/transcript",
    poll_endpoint: "/v2/transcript/{id}",
    upload_endpoint: "/v2/upload",
    submit_handle_field: "id",
    status_path: "status",
    done_status: "completed",
    error_status: "error",
};

static OPENAI_TRANSCRIPTION_GEN: TranscriptionDef = TranscriptionDef {
    wire_shape: "TranscriptionOpenAI",
    interaction: "sync",
    request_encoding: "multipart",
    submit_endpoint: "/v1/audio/transcriptions",
    poll_endpoint: "",
    upload_endpoint: "",
    submit_handle_field: "",
    status_path: "",
    done_status: "",
    error_status: "",
};

pub fn transcription_config(provider: ProviderName) -> Option<&'static TranscriptionDef> {
    match provider {
        ProviderName::Assemblyai => Some(&ASSEMBLYAI_TRANSCRIPTION_GEN),
        ProviderName::OpenAI => Some(&OPENAI_TRANSCRIPTION_GEN),
        _ => None,
    }
}

// Code generated — DO NOT EDIT.


use super::providers::ProviderName;

//
//
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TranscriptionDef {
    pub wire_shape: &'static str,
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
    submit_endpoint: "/v2/transcript",
    poll_endpoint: "/v2/transcript/{id}",
    upload_endpoint: "/v2/upload",
    submit_handle_field: "id",
    status_path: "status",
    done_status: "completed",
    error_status: "error",
};

pub fn transcription_config(provider: ProviderName) -> Option<&'static TranscriptionDef> {
    match provider {
        ProviderName::Assemblyai => Some(&ASSEMBLYAI_TRANSCRIPTION_GEN),
        _ => None,
    }
}

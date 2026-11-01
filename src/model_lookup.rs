//! One catalogue lookup for every media operation (ADR-102 MPT-001, G4).
//! Mirrors go/model_lookup.go.

use crate::catalogue::ontology_capabilities;
use crate::error::Error;
use crate::providers::generated::providers::ProviderName;
use crate::types::Capability;

/// Finds `model_id` in one capability's compiled catalogue. It never errors:
/// an unlisted model returns `(None, false)`. The caller then sends the
/// request with the fields the caller set and skips every per-model check;
/// config checks still run.
pub(crate) fn lookup_model<M>(
    models: &'static [M],
    model_id: &str,
    id_of: fn(&M) -> &str,
) -> (Option<&'static M>, bool) {
    let model = models.iter().find(|m| id_of(m) == model_id);
    (model, model.is_some())
}

/// The MIME a reply without its own MIME is labelled with: the listed
/// model's, or for an unlisted model (`None`) the config's first listed
/// model's. Every model of one config states the same MIME, so the config's
///
/// and speech (CQ-MUS-015, CQ-VID-021, CQ-SPK-019).
pub(crate) fn fallback_mime<M>(
    models: &'static [M],
    model: Option<&'static M>,
    mime_of: fn(&M) -> &'static str,
) -> &'static str {
    model.or(models.first()).map_or("", mime_of)
}

/// The catalogue lookup for chat, agent and batch (ADR-102 MPT-002): true
/// when the compiled catalogue lists `model_id` as a chat model of
/// `provider`. It never errors; the chat send path runs no per-model check,
/// so the answer only marks the middleware event.
pub(crate) fn chat_model_listed(provider: ProviderName, model_id: &str) -> bool {
    ontology_capabilities(provider, model_id)
        .is_some_and(|caps| caps.contains(&Capability::ChatCompletion))
}

/// Whether a listed model's support fact rejects `option` before send
/// (ADR-102 MPT-006 G6, MPT-007). Only a stated `Some(false)` blocks, and only
/// on a config whose provider silently ignores the option (`ignored` is the
/// config's `ignores_unsupported`). Elsewhere the request is sent and the
/// provider answers; an unknown fact (`None`) never blocks (decision 009).
pub(crate) fn stated_false_blocks(ignored: &[&str], option: &str, support: Option<bool>) -> bool {
    support == Some(false) && ignored.contains(&option)
}

/// Names the stated facts of a listed model that a request contradicts
/// (ADR-102 MPT-008), for example `"catalogue: heygen/heygen-video-1 lists
/// no last frame, at most 15 seconds"`. The facts keep the order the caller
/// gives; every SDK uses the same order. No facts give no hint.
pub(crate) fn catalogue_hint(model_id: &str, facts: &[String]) -> Option<String> {
    if facts.is_empty() {
        return None;
    }
    Some(format!("catalogue: {model_id} lists {}", facts.join(", ")))
}

/// Attaches `hint` to an `Error::Api` with status 400 or 422; any other
/// error or status, and no hint, return `err` unchanged (ADR-102 MPT-008;
/// owner, 2026-11-01). Callers attach it before the post middleware event,
/// so middleware sees the error the caller gets.
pub(crate) fn with_hint(err: Error, hint: Option<String>) -> Error {
    match err {
        Error::Api {
            provider,
            status_code,
            message,
            reason,
            ..
        } if hint.is_some() && matches!(status_code, 400 | 422) => Error::Api {
            provider,
            status_code,
            message,
            reason,
            hint,
        },
        other => other,
    }
}

/// Whether `value` is set and outside a model's non-empty list. An empty
/// list states no fixed set, so no value is off it.
pub(crate) fn off_list(list: &[&str], value: &str) -> bool {
    !value.is_empty() && !list.is_empty() && !list.contains(&value)
}

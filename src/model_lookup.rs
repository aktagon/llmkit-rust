//! One catalogue lookup for every media operation (ADR-102 MPT-001, G4).
//! Mirrors go/model_lookup.go.

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
/// first model stands for the config.
pub(crate) fn fallback_mime<M>(
    models: &'static [M],
    model: Option<&'static M>,
    mime_of: fn(&M) -> &'static str,
) -> &'static str {
    model.or(models.first()).map_or("", mime_of)
}

// Single integration-test binary. Each former top-level `tests/*.rs` file is
// now a submodule of `integration` (resolved to `tests/integration/<name>.rs`),
// so cargo links ONE test binary against the crate + its dep tree instead of
// twenty. The shared mock-server plumbing stays in `tests/common/` and is
// reached from the submodules as `crate::common::...`.
mod common;

mod integration {
    mod builders_constructors;
    mod builders;
    mod catalogue_http;
    mod catalogue;
    mod examples;
    mod image;
    mod middleware;
    mod models_parsers;
    mod music;
    mod no_default_contract;
    mod options;
    mod prompt;
    mod provider_info;
    mod request_wire;
    mod responses;
    mod speech;
    mod telemetry;
    mod transcription;
    mod video;
    mod wire;
}

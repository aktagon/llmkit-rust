//!
//!
//!
//!
//!
//!

use std::error::Error as StdError;

use llmkit::MiddlewareVeto;

#
fn veto_display_prefixes_cause_with_marker() {
    let cause: Box<dyn StdError + Send + Sync> = "budget exceeded".into();
    let veto = MiddlewareVeto { cause };
    assert_eq!(format!("{}", veto), "middleware veto: budget exceeded");
}

#
fn veto_error_source_returns_underlying_cause() {
    //
    //
    let cause: Box<dyn StdError + Send + Sync> =
        std::io::Error::new(std::io::ErrorKind::Other, "network down").into();
    let veto = MiddlewareVeto { cause };
    let src = veto.source().expect("source should be the wrapped cause");
    assert_eq!(src.to_string(), "network down");
}

#
fn veto_debug_renders_struct_name() {
    let cause: Box<dyn StdError + Send + Sync> = "x".into();
    let veto = MiddlewareVeto { cause };
    let s = format!("{:?}", veto);
    assert!(
        s.contains("MiddlewareVeto"),
        "debug missing struct name: {}",
        s
    );
}

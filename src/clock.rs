//! The one place llmkit reads the clock and sleeps.
//!
//! Native builds use `std::time` and tokio's timer. On wasm32-unknown-unknown
//! the std clock panics ("time not implemented on this platform") and tokio
//! has no timer, so the wasm32 build reads the clock from JavaScript and
//! sleeps with `setTimeout`. It looks both up on the global object, so it
//! works in a window, a worker and Node.

use std::time::Duration;

use crate::error::Error;

#[cfg(not(target_arch = "wasm32"))]
pub(crate) use std::time::Instant;

#[cfg(target_arch = "wasm32")]
pub(crate) use self::wasm::Instant;

/// Time since the Unix epoch, from the wall clock. A clock set before 1970
/// reads as zero.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn unix_time() -> Duration {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn unix_time() -> Duration {
    // `as u64` saturates: a negative or NaN reading becomes zero.
    Duration::from_millis(js_sys::Date::now() as u64)
}

/// Waits for `duration`. Native builds cannot fail. The wasm32 build fails
/// only when the JavaScript host has no `setTimeout`.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) async fn sleep(duration: Duration) -> Result<(), Error> {
    tokio::time::sleep(duration).await;
    Ok(())
}

#[cfg(target_arch = "wasm32")]
pub(crate) async fn sleep(duration: Duration) -> Result<(), Error> {
    wasm::sleep(duration).await
}

#[cfg(target_arch = "wasm32")]
mod wasm {
    use std::ops::Add;
    use std::time::Duration;

    use js_sys::{Function, Promise, Reflect};
    use wasm_bindgen_futures::wasm_bindgen::{JsCast, JsValue};
    use wasm_bindgen_futures::JsFuture;

    use crate::error::Error;

    /// A point on the JavaScript monotonic clock (`performance.now()`), in
    /// milliseconds. Falls back to `Date.now()` where `performance` is
    /// missing. Supports what llmkit uses of `std::time::Instant`: `now`,
    /// `elapsed`, adding a `Duration` and comparing.
    #[derive(Clone, Copy, Debug, PartialEq, PartialOrd)]
    pub(crate) struct Instant {
        millis: f64,
    }

    impl Instant {
        pub(crate) fn now() -> Self {
            Self {
                millis: now_millis(),
            }
        }

        pub(crate) fn elapsed(&self) -> Duration {
            millis_to_duration(now_millis() - self.millis)
        }
    }

    impl Add<Duration> for Instant {
        type Output = Instant;

        fn add(self, duration: Duration) -> Instant {
            Instant {
                millis: self.millis + duration.as_secs_f64() * 1000.0,
            }
        }
    }

    /// A negative, NaN or out-of-range span reads as zero instead of panicking.
    fn millis_to_duration(millis: f64) -> Duration {
        Duration::try_from_secs_f64(millis / 1000.0).unwrap_or(Duration::ZERO)
    }

    fn now_millis() -> f64 {
        performance_now().unwrap_or_else(js_sys::Date::now)
    }

    fn performance_now() -> Option<f64> {
        let performance =
            Reflect::get(&js_sys::global(), &JsValue::from_str("performance")).ok()?;
        let now = Reflect::get(&performance, &JsValue::from_str("now"))
            .ok()?
            .dyn_into::<Function>()
            .ok()?;
        now.call0(&performance).ok()?.as_f64()
    }

    pub(super) async fn sleep(duration: Duration) -> Result<(), Error> {
        let global = js_sys::global();
        let set_timeout = Reflect::get(&global, &JsValue::from_str("setTimeout"))
            .ok()
            .and_then(|value| value.dyn_into::<Function>().ok())
            .ok_or_else(|| {
                Error::Unsupported("sleep: the JavaScript host has no setTimeout".into())
            })?;
        // setTimeout takes a 32-bit delay; a longer one fires at once.
        let delay = duration.as_millis().min(i32::MAX as u128) as f64;
        let mut scheduled = Ok(JsValue::UNDEFINED);
        let promise = Promise::new(&mut |resolve, _reject| {
            scheduled = set_timeout.call2(&global, &resolve, &JsValue::from_f64(delay));
        });
        scheduled
            .map_err(|cause| Error::Unsupported(format!("sleep: setTimeout failed: {cause:?}")))?;
        JsFuture::from(promise)
            .await
            .map(|_| ())
            .map_err(|cause| Error::Unsupported(format!("sleep: timer rejected: {cause:?}")))
    }


























}

use std::cell::RefCell;

use wasm_bindgen::prelude::*;

use crate::prelude::*;
use crate::ts;

#[derive(tsify::Tsify, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Callbacks {
    #[serde(with = "serde_wasm_bindgen::preserve")]
    pub on_lamp_group: ts::TsFn<fn(ts::TsNum) -> ts::TsVoid>,

    #[serde(with = "serde_wasm_bindgen::preserve")]
    pub on_progress: ts::TsFn<fn(ts::TsNum, ts::TsStr) -> ts::TsVoid>,
}

thread_local! {
    static CALLBACKS: RefCell<Option<Callbacks>> = const { RefCell::new(None) };

    #[cfg(feature = "perf_timing")]
    static WEB_SYS_PERFORMANCE: std::sync::LazyLock<web_sys::Performance> = std::sync::LazyLock::new(
        || {
            fn init() -> Result<web_sys::Performance, JsError> {
                let performance =
                    if let Ok(js_global) = js_sys::global().dyn_into::<web_sys::WorkerGlobalScope>() {
                        js_global.performance()
                    } else if let Some(window) = web_sys::window() {
                        window.performance()
                    } else {
                        err_js!("Could not access 'global' field on JS");
                    };
                performance.ok_or_else(|| js_err!("No 'performance' field!"))
            }

            #[expect(
                clippy::unwrap_used,
                reason = "For performance-critical access, this returns the value directly instead of wrapping in a Result<>"
            )]
            init().map_js_err("Could not initialize performance object").unwrap()
        },
    );
}

/// Sets the static callbacks to be used during blueprint generation.
///
/// # Arguments
///
/// * `callbacks` - An object of JS functions that will be called during the
///   generation process.
#[wasm_bindgen]
pub fn set_callbacks(callbacks: &tsify::Ts<Callbacks>) -> Result<(), JsError> {
    CALLBACKS.with_borrow_mut(|cbs| {
        *cbs = Some(callbacks.to_rust().map_js_err("Could not convert to Rust")?);
        Ok(())
    })
}

/// Reports progress by calling the registered progress callback.
///
/// # Arguments
///
/// * `percentage` - A value between 0 and 1 representing the progress.
/// * `status` - A status message indicating the current stage.
pub fn report_progress<T: Into<f64>>(percentage: T, status: &str) {
    CALLBACKS.with_borrow(|cbs| {
        if let Some(cbs) = cbs.as_ref() {
            let pct = 100.0 * percentage.into();
            _ = cbs.on_progress.call_n((&pct.into(), &status.into()));
        }
    });
}
pub fn report_lamp_group<T: Into<f64>>(percentage: T) {
    CALLBACKS.with_borrow(|cbs| {
        if let Some(cbs) = cbs.as_ref() {
            let pct = 100.0 * percentage.into();
            _ = cbs.on_lamp_group.call_n((&pct.into(),));
        }
    });
}

#[inline]
pub fn set_progress(start: f64, end: f64, progress: f64, msg: &str) {
    report_progress(progress.mul_add(end - start, start), msg);
}

#[inline]
#[must_use]
#[allow(dead_code, reason = "api")]
pub fn perf_now() -> f64 {
    #[cfg(feature = "perf_timing")]
    {
        WEB_SYS_PERFORMANCE.with(|perf| perf.now())
    }

    #[cfg(not(feature = "perf_timing"))]
    {
        js_sys::Date::now()
    }
}

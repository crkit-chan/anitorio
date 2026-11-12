use crate::{prelude::*, ts};
use std::cell::RefCell;
use wasm_bindgen::prelude::*;

#[derive(tsify::Tsify, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Callbacks {
    #[serde(with = "serde_wasm_bindgen::preserve")]
    pub on_lamp_group: ts::TsFn<fn(ts::TsNum) -> ts::TsVoid>,

    #[serde(with = "serde_wasm_bindgen::preserve")]
    pub on_progress: ts::TsFn<fn(ts::TsNum, ts::TsStr) -> ts::TsVoid>,

    #[serde(with = "serde_wasm_bindgen::preserve")]
    #[allow(clippy::allow_attributes, dead_code, reason = "api")]
    pub perf_now: ts::TsFn<fn() -> ts::TsNum>,
}

thread_local! {
    static CALLBACKS: RefCell<Option<Callbacks>> = const { RefCell::new(None) };
}

/// Sets the static callbacks to be used during blueprint generation.
///
/// # Arguments
///
/// * `callbacks` - An object of JS functions that will be called during the generation process.
#[wasm_bindgen]
pub fn set_callbacks(callbacks: &tsify::Ts<Callbacks>) {
    CALLBACKS.with_borrow_mut(|cbs| *cbs = Some(callbacks.to_rust().unwrap()));
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
            let _ = cbs.on_progress.call_n((&pct.into(), &status.into()));
        }
    });
}
pub fn report_lamp_group<T: Into<f64>>(percentage: T) {
    CALLBACKS.with_borrow(|cbs| {
        if let Some(cbs) = cbs.as_ref() {
            let pct = 100.0 * percentage.into();
            let _ = cbs.on_lamp_group.call_n((&pct.into(),));
        }
    });
}

pub fn set_progress(start: f64, end: f64, progress: f64, msg: &str) {
    report_progress(progress.mul_add(end - start, start), msg);
}

#[allow(clippy::allow_attributes, dead_code, reason = "api")]
pub fn perf_now() -> f64 {
    CALLBACKS
        .with_borrow(|cbs| -> Option<f64> { cbs.as_ref()?.perf_now.call_n(()).ok()?.as_f64() })
        .unwrap_or_else(crate::util::now_ms)
}

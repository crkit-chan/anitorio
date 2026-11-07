use crate::{prelude::*, ts};
use std::cell::RefCell;
use wasm_bindgen::prelude::*;

#[derive(tsify::Tsify, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgressCallbacks {
    #[serde(with = "serde_wasm_bindgen::preserve")]
    pub on_lamp_group: ts::TsFn<fn(ts::TsNum) -> ts::TsVoid>,

    #[serde(with = "serde_wasm_bindgen::preserve")]
    pub on_progress: ts::TsFn<fn(ts::TsNum, ts::TsStr) -> ts::TsVoid>,
}

thread_local! {
    static PROGRESS_CALLBACKS: RefCell<Option<ProgressCallbacks>> = const { RefCell::new(None) };
    // static LAMP_GROUP_CALLBACK: RefCell<Option<LampGroupGenerationCallback>> = const { RefCell::new(None) };
    // static PROGRESS_CALLBACK: RefCell<Option<ProgressCallback>> = const { RefCell::new(None) };
}

/// Sets the progress callback to be used during blueprint generation.
///
/// # Arguments
///
/// * `callbacks` - An object of JS functions that will be called during the generation process.
#[wasm_bindgen]
pub fn set_progress_callbacks(callbacks: &tsify::Ts<ProgressCallbacks>) {
    PROGRESS_CALLBACKS.with_borrow_mut(|value| *value = Some(callbacks.to_rust().unwrap()));
}

/// Reports progress by calling the registered progress callback.
///
/// # Arguments
///
/// * `percentage` - A value between 0 and 1 representing the progress.
/// * `status` - A status message indicating the current stage.
pub fn report_progress<T: Into<f64>>(percentage: T, status: &str) {
    PROGRESS_CALLBACKS.with_borrow(|progress| {
        if let Some(ref cbs) = *progress {
            let pct = 100.0 * percentage.into();
            let _ = cbs.on_progress.call_n((&pct.into(), &status.into()));
        }
    });
}
pub fn report_lamp_group<T: Into<f64>>(percentage: T) {
    PROGRESS_CALLBACKS.with_borrow(|progress| {
        if let Some(ref cbs) = *progress {
            let pct = 100.0 * percentage.into();
            let _ = cbs.on_lamp_group.call_n((&pct.into(),));
        }
    });
}

pub fn set_progress(start: f64, end: f64, progress: f64, msg: &str) {
    report_progress(progress.mul_add(end - start, start), msg);
}

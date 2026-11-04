use crate::{prelude::*, ts};
use std::cell::RefCell;
use wasm_bindgen::prelude::*;

type ProgressCallback = ts::TsFn<fn(ts::TsNum, ts::TsStr) -> ts::TsVoid>;

thread_local! {
    static PROGRESS_CALLBACK: RefCell<Option<ProgressCallback>> = const { RefCell::new(None) };
}

/// Sets the progress callback to be used during blueprint generation.
///
/// # Arguments
///
/// * `callback` - A JavaScript function accepting a percentage and a status message.
#[wasm_bindgen]
pub fn set_progress_callback(callback: ProgressCallback) {
    PROGRESS_CALLBACK.with_borrow_mut(|progress| *progress = Some(callback));
}

/// Reports progress by calling the registered progress callback.
///
/// # Arguments
///
/// * `percentage` - A value between 0 and 1 representing the progress.
/// * `status` - A status message indicating the current stage.
pub fn report_progress<T: Into<f64>>(percentage: T, status: &str) {
    PROGRESS_CALLBACK.with_borrow(|progress| {
        if let Some(ref callback) = *progress {
            let pct = 100.0 * percentage.into();
            let _ = callback.call_n((&pct.into(), &status.into()));
        }
    });
}

pub fn set_progress(start: f64, end: f64, progress: f64, msg: &str) {
    report_progress(progress.mul_add(end - start, start), msg);
}

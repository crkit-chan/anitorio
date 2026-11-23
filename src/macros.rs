/// Logs a message to the web console.
#[allow(
    clippy::allow_attributes,
    unused_macros,
    reason = "Might not necessarily be used"
)]
#[macro_export]
macro_rules! log {
    ($($arg:tt)*) => {
        ::web_sys::console::log_1(&format!($($arg)*).into());
    };
}

/// Utility wrapper around the existing
#[macro_export]
macro_rules! boxed_slice {
    ($elem:expr; $n:expr) => {
        ::std::vec![$elem; $n].into_boxed_slice()
    };
    ($($elem:expr),* $(,)?) => {
        ::std::vec![$($elem),*].into_boxed_slice()
    };
}

#[macro_export]
macro_rules! js_err {
    ($($arg:tt)*) => {
        JsError::new(&format!($($arg)*))
    };
}

pub use boxed_slice;
pub use js_err;
#[allow(
    clippy::allow_attributes,
    unused_imports,
    reason = "Might not necessarily be used"
)]
pub use log;

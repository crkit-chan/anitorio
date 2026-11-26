/// Logs a message to the web console.
#[allow(unused_macros, reason = "Might not necessarily be used")]
#[macro_export]
macro_rules! log {
    ($($arg:tt)*) => { ::web_sys::console::log_1(&::std::format!($($arg)*).into()) };
}

/// Utility wrapper around the existing
#[macro_export]
macro_rules! boxed_slice {
    ($elem:expr; $n:expr) => { ::std::vec![$elem; $n].into_boxed_slice() };
    ($($elem:expr),* $(,)?) => { ::std::vec![$($elem),*].into_boxed_slice() };
}

/// Utility with similar signature to ``panic!()``.
///
/// Supports variable interpolation using `{}` syntax.
#[macro_export]
macro_rules! err_js {
    ($($arg:tt)*) => { return ::std::result::Result::Err(::wasm_bindgen::JsError::new(&::std::format!($($arg)*))) };
}

/// Creates a new ``wasm_bindgen::JsError``.
///
/// Supports variable interpolation using `{}` syntax.
#[macro_export]
macro_rules! js_err {
    ($($arg:tt)*) => { ::wasm_bindgen::JsError::new(&::std::format!($($arg)*)) };
}

#[macro_export]
macro_rules! write_json_trimmed_to {
    ($writer:expr, $buf:expr) => {{
        let json_bytes = ::serde_json::to_vec($buf)
            .map_err(|e| ::wasm_bindgen::JsError::new(&::std::format!("Write error: {e:?}")))?;
        $crate::macros::write_to!($writer, &json_bytes[1..json_bytes.len() - 1]) // Remove braces
    }};
}

#[macro_export]
macro_rules! write_to {
    ($writer:expr, $buf:expr) => {{
        ($writer.write_all($buf))
            .map_err(|e| ::wasm_bindgen::JsError::new(&::std::format!("Write error: {e:?}")))
    }};
}

#[allow(unused_imports, reason = "Might not necessarily be used")]
pub use log;
pub use {boxed_slice, err_js, js_err, write_json_trimmed_to, write_to};

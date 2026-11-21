/// Logs a message to the web console.
#[allow(
    clippy::allow_attributes,
    unused_macros,
    reason = "Might not necessarily be used"
)]
#[macro_export]
macro_rules! log {
    ($($arg:tt)*) => {{
        let value = format!($($arg)*);
        ::web_sys::console::log_1(&value.clone().into());
    }};
}

/// Creates a constant value that is lazily evaluated once.
#[macro_export]
macro_rules! lazy_arc_const {
    (pub $name:ident: $type:ident = $value:expr) => {
        pub static $name: ::std::sync::LazyLock<::std::sync::Arc<$type>> =
            ::std::sync::LazyLock::new(|| ::std::sync::Arc::from($value));
    };
    ($name:ident: $type:ident = $value:expr) => {
        static $name: ::std::sync::LazyLock<::std::sync::Arc<$type>> =
            ::std::sync::LazyLock::new(|| ::std::sync::Arc::from($value));
    };
}

#[macro_export]
macro_rules! boxed_slice {
    ($elem:expr; $n:expr) => {
        ::std::vec![$elem; $n].into_boxed_slice()
    };
    ($($elem:expr),* $(,)?) => {
        ::std::vec![$($elem),*].into_boxed_slice()
    };
}

pub use lazy_arc_const;
#[allow(
    clippy::allow_attributes,
    unused_imports,
    reason = "Might not necessarily be used"
)]
pub use log;

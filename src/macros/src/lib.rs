use proc_macro::TokenStream;
use quote::{format_ident, quote};
use std::sync::atomic::{AtomicUsize, Ordering};
use syn::{Expr, LitStr, parse_macro_input};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

/// A custom timing macro for Anitorio.
///
/// Usage:
///
/// ```rust
/// #[crate::perf_timed("Possibly heavy computation")]
/// {
///     for i in 0..65_536 {
///         some_heavy_computation();
///     }
/// }
/// ```
///
/// Uses the JavaScript ``performance.now()`` API to measure performance timings for WASM code.
///
/// This is only enabled if the --perf_timing feature flag is set during compilation.
/// If the flag is disabled, this has no effect.
#[proc_macro_attribute]
pub fn perf_timed(attr: TokenStream, item: TokenStream) -> TokenStream {
    let label = parse_macro_input!(attr as LitStr);
    let expr = parse_macro_input!(item as Expr);

    let id = COUNTER.fetch_add(1, Ordering::Relaxed);

    let start = format_ident!("__perf_start_{id}", span = proc_macro2::Span::mixed_site());
    let result = format_ident!("__perf_result_{id}", span = proc_macro2::Span::mixed_site());
    let elapsed = format_ident!(
        "__perf_elapsed_{id}",
        span = proc_macro2::Span::mixed_site()
    );

    quote! {{
        #[cfg(feature = "perf_timing")]
        {
            let #start = crate::perf_now();
            let #result = #expr;
            let #elapsed = crate::perf_now() - #start;

            crate::log!("{}: {:.3}ms", #label, #elapsed);

            #result
        }

        #[cfg(not(feature = "perf_timing"))]
        {
            #expr
        }
    }}
    .into()
}

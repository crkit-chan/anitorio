use std::sync::atomic::{AtomicUsize, Ordering};

use proc_macro::TokenStream;
use proc_macro2::Span;
use quote::{format_ident, quote};
use syn::{Expr, ItemFn, LitStr, Stmt, parse_macro_input};

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
///
/// #[crate::perf_timed("Doing some stuff...")]
/// async fn foo(bar: Baz) -> Qux {
///     // some heavy computation
/// }
///
/// #[crate::perf_timed("Calling some_function()")]
/// let value = some_function().map(|v| v.as_ref());
/// ```
///
/// Uses the JavaScript ``performance.now()`` API to measure performance timings
/// for WASM code.
///
/// This is only enabled if the --perf_timing feature flag is set during
/// compilation. If the flag is disabled, this has no effect.
#[proc_macro_attribute]
pub fn perf(attr: TokenStream, item: TokenStream) -> TokenStream {
    let label = parse_macro_input!(attr as LitStr);

    let id = COUNTER.fetch_add(1, Ordering::Relaxed);

    let start = format_ident!("__perf_start_{id}", span = Span::mixed_site());
    let result = format_ident!("__perf_result_{id}", span = Span::mixed_site());
    let elapsed = format_ident!("__perf_elapsed_{id}", span = Span::mixed_site());

    // Function-level timing
    if let Ok(mut func) = syn::parse::<ItemFn>(item.clone()) {
        let fn_name = &func.sig.ident;
        let block = func.block;

        let timing = if func.sig.asyncness.is_some() {
            quote! {
                let #start = crate::perf_now();
                let #result = (async move #block).await;
                let #elapsed = crate::perf_now() - #start;

                crate::log!("{} ({}()): {:.2}ms", #label, stringify!(#fn_name), #elapsed);

                #result
            }
        } else {
            quote! {
                let #start = crate::perf_now();
                let #result = #block;
                let #elapsed = crate::perf_now() - #start;

                crate::log!( "{} ({}): {:.2}ms", #label, stringify!(#fn_name), #elapsed);

                #result
            }
        };

        func.block = Box::new(syn::parse_quote!({
            #[cfg(feature = "perf_timing")]
            { #timing }

            #[cfg(not(feature = "perf_timing"))]
            #block
        }));

        return quote!(#func).into();
    }

    // Local let statement: wrap only the initializer
    if let Ok(mut stmt @ Stmt::Local(_)) = syn::parse::<Stmt>(item.clone()) {
        if let Stmt::Local(local) = &mut stmt
            && let Some(init) = &mut local.init
        {
            let expr = &init.expr;

            init.expr = Box::new(syn::parse_quote!({
                #[cfg(feature = "perf_timing")]
                {
                    let #start = crate::perf_now();
                    let #result = #expr;
                    let #elapsed = crate::perf_now() - #start;
                    crate::log!("{}: {:.2}ms", #label, #elapsed);
                    #result
                }

                #[cfg(not(feature = "perf_timing"))]
                { #expr }
            }));
        }

        return quote!(#stmt).into();
    }

    // Expression/block-level timing
    let expr = syn::parse_macro_input!(item as Expr);

    quote! {{
        #[cfg(feature = "perf_timing")]
        {
            let #start = crate::perf_now();
            let #result = #expr;
            let #elapsed = crate::perf_now() - #start;

            crate::log!("{}: {:.2}ms", #label, #elapsed);

            #result
        }

        #[cfg(not(feature = "perf_timing"))]
        { #expr }
    }}
    .into()
}

#![feature(
    cold_path,
    const_cmp,
    const_convert,
    const_default,
    const_destruct,
    const_result_trait_fn,
    const_trait_impl,
    const_slice_make_iter,
    hash_set_entry,
    inherent_associated_types,
    likely_unlikely,
    proc_macro_hygiene,
    stmt_expr_attributes
)]

pub use anitorio_macros::perf;
#[cfg(feature = "perf_timing")]
pub use callbacks::perf_now;
use wasm_bindgen::prelude::*;

mod blueprint;
mod callbacks;
mod constants;
mod hash;
mod image_processing;
mod image_util;
mod ioutil;
mod macros;
mod models;
mod palette;
mod prelude;
mod ts;
mod util;

/// .
///
/// # Errors
///
/// This function will return an error if rayon fails to initialize.
#[wasm_bindgen]
#[allow(
    clippy::unused_async,
    unused,
    reason = "n_threads only used when threads feature is enabled"
)]
pub async fn init_rayon(n_threads: Option<usize>) -> Result<bool, JsError> {
    #[cfg(not(feature = "threads"))]
    {
        Ok(false)
    }

    #[cfg(feature = "threads")]
    {
        use crate::prelude::*;
        wasm_bindgen_rayon::init_thread_pool(n_threads.unwrap_or(4))
            .await
            .map_js_err("Could not init thread pool")?;
        Ok(true)
    }
}

#[wasm_bindgen]
pub fn init() {
    console_error_panic_hook::set_once();
}

#[derive(tsify::Tsify, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetNextFrameArgs {
    dimensions:          Option<glam::UVec2>,
    #[serde(with = "serde_wasm_bindgen::preserve")]
    in_buffer:           util::WasmSharedMemory,
    use_sparse_decoding: Option<bool>,
}

#[derive(tsify::Tsify, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnFrameArgs {
    dimensions: glam::UVec2,
    #[serde(with = "serde_wasm_bindgen::preserve")]
    frame:      ts::TsUint8ClampedArrayBacked,
}

#[derive(tsify::Tsify, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunBlueprintCallbacks {
    #[serde(with = "serde_wasm_bindgen::preserve")]
    pub get_data:        ts::TsFn<fn(ts::TsNum, ts::TsNum) -> js_sys::Uint8Array>,
    #[serde(with = "serde_wasm_bindgen::preserve")]
    pub get_data_len:    ts::TsFn<fn() -> ts::TsNum>,
    #[serde(with = "serde_wasm_bindgen::preserve")]
    pub get_next_frame:  ts::TsFn<fn(ts::TsIntoRust<GetNextFrameArgs>) -> ts::TsFut<JsValue>>,
    #[serde(with = "serde_wasm_bindgen::preserve")]
    pub js_yield:        ts::TsFn<fn() -> ts::TsFut>,
    #[serde(with = "serde_wasm_bindgen::preserve")]
    pub on_chunk:        ts::TsFn<fn(js_sys::Uint8Array) -> ts::TsFut>,
    #[serde(with = "serde_wasm_bindgen::preserve")]
    pub on_frame:        ts::TsFn<fn(ts::TsIntoRust<OnFrameArgs>) -> ts::TsFut>,
    #[serde(with = "serde_wasm_bindgen::preserve")]
    pub reset_generator: ts::TsFn<fn() -> ts::TsFut>,
    #[serde(with = "serde_wasm_bindgen::preserve")]
    pub should_preview:  ts::TsFn<fn() -> ts::TsBool>,
}

/// Public entry point for WebAssembly.
///
/// # Parameters
///
/// - `options`: Options dict passed in
/// - `image_data`: A ``Uint8Array`` passed in from JS. May be empty if `mode`
///   (specified in ``options``) is not `"full"`.
///
/// # Returns
///
/// A Factorio blueprint string on success.
///
/// # Panics
///
/// Throws ``JsValue`` if any error occurs.
///
/// # Errors
///
/// Returns ``JsValue`` if any error occurs.
#[wasm_bindgen]
pub async fn run_blueprint(
    args: tsify::Ts<models::BlueprintArgs>, callbacks: tsify::Ts<RunBlueprintCallbacks>,
) -> Result<JsValue, JsError> {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    use blueprint::encoder::BlueprintEncoder;
    use blueprint::generator::BlueprintGenerator;
    use blueprint::models::SignalID;
    use image_processing::{FrameCallback, FrameData, ImageFrameData};
    use ioutil::JsFileReader;
    use models::BlueprintArgs;

    use crate::prelude::*;

    const SYNC_TIME_MS: f64 = 1000.0;

    let mut args: BlueprintArgs = args.to_rust().map_js_err("Could not parse arguments")?;
    let cbs: RunBlueprintCallbacks = callbacks.to_rust().map_js_err("Could not callbacks")?;
    let on_frame = Rc::new(cbs.on_frame);
    let should_show_preview = Rc::new(cbs.should_preview);
    let yield_to_js = Rc::new(cbs.js_yield);

    args.timing_signals = (args.timing_signals.into_iter())
        .map(|(k, sig)| Ok((k, SignalID::intern(sig.type_.as_ref(), sig.name.as_ref())?)))
        .collect::<Result<_, JsError>>()?;

    if let Some(ref name) = args.power_pole_name {
        let ref_sig =
            SignalID::intern_with_quality("entity", name, args.power_pole_quality.as_deref())?;

        args.power_pole_name = Some(RC::clone(&ref_sig.name));
        args.power_pole_quality = Some(RC::clone(&ref_sig.quality));
    }

    let mut frame_data: Option<FrameData> = None;
    let promises = Rc::new(const { RefCell::new(Vec::<ts::TsFut>::new()) });
    let promise_q = Rc::clone(&promises);
    let next_yield_time = Rc::new(Cell::new(now_ms() + SYNC_TIME_MS));

    if matches!(args.image_args.mode, Mode::Static { .. }) {
        args.signal_compression = None;
        args.image_args.target_fps = 1;
    }
    if matches!(args.image_args.mode, Mode::Full | Mode::Static { .. }) {
        let on_frame: Option<Rc<FrameCallback>> = Some(Rc::new(move |img| {
            let on_frame = Rc::clone(&on_frame);
            let should_show_preview = Rc::clone(&should_show_preview);
            let yield_to_js = Rc::clone(&yield_to_js);
            let next_yield_time = Rc::clone(&next_yield_time);

            Box::pin(async move {
                let show_preview = should_show_preview.call_n(())?;
                let show_preview = show_preview.as_bool() == Some(true);
                let now = now_ms();

                // yield to js so preview can be updated
                if show_preview || (now > next_yield_time.get()) {
                    next_yield_time.set(now + SYNC_TIME_MS);
                    yield_to_js.call_n(())?.await?;
                }
                if !show_preview {
                    return Ok(());
                }

                #[expect(
                    clippy::option_if_let_else,
                    reason = "Cannot return reference to a local value"
                )]
                let rgba = match img.as_rgba8() {
                    Some(rgba) => rgba,
                    _ => &img.to_rgba8(),
                };
                let js_args = &ts::TsIntoRust::new(&OnFrameArgs {
                    dimensions: rgba.dimensions().into(),
                    frame:      js_sys::Uint8ClampedArray::new_with_byte_offset_and_length(
                        &wasm_memory().buffer(),
                        rgba.as_ptr() as u32,
                        rgba.len() as u32,
                    ),
                })?;
                on_frame.call_n((js_args,))?.await?;

                Ok(())
            })
        }));

        let mut palette: Option<ColorPalette> = None;
        let palette_size = args.image_args.color_palette_size;
        let shading_speed = i32::from(args.image_args.shading_speed);

        let data_len = {
            let l = cbs.get_data_len.call_n(())?;
            l.as_f64().ok_or_else(|| {
                js_err!("getDataLen() did not return a value parsable as a f64: '{l}'")
            })?
        };
        let get_next_frame = (args.image_args.mime_type)
            .starts_with("video/")
            .then(|| cbs.get_next_frame.clone());
        let data_len = data_len as u32;

        let chunk_size = (1 << (usize::BITS - 1 - data_len.max(1).leading_zeros()))
            // Set chunk size to the largest power of 2 <= data_len, clamped between 16 KiB - 1 MiB
            .clamp(16 * 1024, 1024 * 1024);

        if palette_size > 0 {
            callbacks::set_is_preview(true);

            // For generating the palette (first pass), we reduce resolution and FPS
            let mut image_args = args.image_args.clone();

            if !args.image_args.full_quality_scanning {
                if let Some(duration) = image_args.duration {
                    /// Above this number of seconds, we'll start reducing the
                    /// FPS we sample at for performance.
                    const SECS_THRESHOLD: f64 = 5.0;

                    let fps = f64::from(image_args.target_fps);

                    image_args.target_fps = fps
                        // Gradually reduce the FPS in proportion to how long the video is
                        .min(((SECS_THRESHOLD * fps) / duration).ceil())
                        .max(1.0) as u8; // Safety fallback
                }

                if let Some(dimensions) = image_args.dimensions {
                    const DIM_THRESHOLD: u16 = 50;

                    image_args.max_size = (dimensions.min_element() as u16).min(DIM_THRESHOLD);
                }
            }

            let reader = JsFileReader::new(cbs.get_data.clone(), u64::from(data_len), chunk_size);
            let decoder = ImageFrameData::new(reader, &image_args, get_next_frame.clone())?;
            let generator = FrameData::new(decoder, &image_args, on_frame.clone())?
                .with_progress_message("Scanning colors");

            palette = Some(
                image_processing::build_palette(generator, palette_size, shading_speed).await?,
            );
            cbs.reset_generator.call_n(())?.await?;
        }

        let reader = JsFileReader::new(cbs.get_data.clone(), u64::from(data_len), chunk_size);
        let decoder = ImageFrameData::new(reader, &args.image_args, get_next_frame.clone())?;
        let generator = FrameData::new(decoder, &args.image_args, on_frame.clone())?
            .with_progress_message("Processing Frame");

        frame_data = Some(generator.with_palette(palette));
    }

    callbacks::set_is_preview(false);

    // Custom writer that invokes a callback whenever chunk size is reached.
    let writer = ioutil::BufWriter::new(1024 * 1024, !args.low_mem_usage, |chunk| {
        let promise = cbs.on_chunk.call_n((&(&*chunk).into(),))?;
        promise_q.borrow_mut().push(promise);
        Ok(())
    });

    // Yields JSON or compressed blueprint in chunks
    let generator = BlueprintGenerator::new(frame_data, &args).await?;
    let mut encoder = BlueprintEncoder::new(generator, &args, writer)?;

    let flush = async || -> Result<(), JsError> {
        let js_promises = js_sys::Array::from(&JsValue::from(promises.take()));
        (js_sys::Promise::all(&js_promises).await).map_js_err("Could not wait for write")?;
        Ok(())
    };

    // `encoder.next()` may generate a series of chunks
    while encoder.next().await?.is_some() {
        flush().await?; // Wait for all those chunks to be sent
    }

    let mut writer = encoder.finish().await?;
    writer.finish().map_js_err("Stream finish err")?;

    flush().await?; // Flush tail

    let obj = js_sys::Object::new();
    Ok(JsValue::from(obj))
}

/// Sets the list of signals and qualities available for use in the blueprint.
///
/// # Arguments
///
/// * `data_json` - The raw JSON data as a ``Uint8Array``.
///
/// # Returns
/// Nothing.
///
/// # Errors
///
/// ``JsError`` if any error occurs.
#[wasm_bindgen]
pub fn set_signal_data(data_json: &[u8]) -> Result<(), JsError> {
    blueprint::data::load_data(data_json)?;
    Ok(())
}

///
///
/// # Errors
///
/// ``JsError`` if any error occurs.
#[wasm_bindgen]
pub fn set_comb_pos_data(data_json: &[u8]) -> Result<(), JsError> {
    blueprint::combinator::load_comb_pos_json(data_json)?;
    Ok(())
}

#[derive(tsify::Tsify, serde::Deserialize)]
pub struct EncodeGifCallbacks {
    #[serde(with = "serde_wasm_bindgen::preserve")]
    pub get_next_frame: image_processing::GetJsFrameCallback,

    #[serde(with = "serde_wasm_bindgen::preserve")]
    pub on_chunk: ts::TsFn<fn(js_sys::Uint8Array) -> ts::TsFut>,
}

/// # Errors
///
/// ``JsValue`` if any error occurs.
#[wasm_bindgen]
pub async fn encode_gif(
    args: JsValue, callbacks: tsify::Ts<EncodeGifCallbacks>,
) -> Result<(), JsError> {
    use image_processing::{GetJsFrameCallback, encode_gif};
    use util::JsReflectGetSet as _;

    let args: models::EncodeGifArgs = serde_wasm_bindgen::from_value(args)?;
    let get_next_frame = callbacks.reflect_get::<GetJsFrameCallback>("getFrameData")?;
    let on_chunk = callbacks
        .reflect_get::<js_sys::Function<fn(js_sys::Uint8Array) -> ts::TsFut>>("onChunk")?;

    encode_gif(get_next_frame, on_chunk, &args).await?;

    Ok(())
}

#[wasm_bindgen]
#[must_use]
pub fn wasm_memory() -> js_sys::WebAssembly::Memory {
    wasm_bindgen::memory().unchecked_into::<js_sys::WebAssembly::Memory>()
}

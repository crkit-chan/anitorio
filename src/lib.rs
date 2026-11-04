use wasm_bindgen::prelude::*;

mod blueprint;
mod constants;
mod image_processing;
mod image_util;
mod ioutil;
mod macros;
mod models;
mod prelude;
mod progress;
mod ts;
mod util;

#[cfg(feature = "profiling")]
#[global_allocator]
static GLOBAL_ALLOCATOR: wasm_tracing_allocator::WasmTracingAllocator<std::alloc::System> =
    wasm_tracing_allocator::WasmTracingAllocator(std::alloc::System);

#[wasm_bindgen]
pub fn init() {
    console_error_panic_hook::set_once();
}

#[derive(tsify::Tsify, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunBlueprintCallbacks {
    #[serde(with = "serde_wasm_bindgen::preserve")]
    pub get_data: ts::TsFn<fn(ts::number, ts::number) -> js_sys::Uint8Array>,

    #[serde(with = "serde_wasm_bindgen::preserve")]
    pub get_data_len: ts::TsFn<fn() -> ts::number>,

    #[serde(with = "serde_wasm_bindgen::preserve")]
    pub get_next_frame: ts::TsFn<fn(JsValue) -> js_sys::Promise<JsValue>>,

    #[serde(with = "serde_wasm_bindgen::preserve")]
    pub js_yield: ts::TsFn<fn() -> js_sys::Promise<()>>,

    #[serde(with = "serde_wasm_bindgen::preserve")]
    pub on_chunk: ts::TsFn<fn(js_sys::Uint8Array) -> js_sys::Promise<()>>,

    #[serde(with = "serde_wasm_bindgen::preserve")]
    pub on_frame: ts::TsFn<fn(JsValue) -> js_sys::Promise<()>>,

    #[serde(with = "serde_wasm_bindgen::preserve")]
    pub should_preview: ts::TsFn<fn() -> ts::boolean>,
}

/// Public entry point for WebAssembly.
///
/// # Parameters
///
/// - `options`: Options dict passed in
/// - `image_data`: A ``Uint8Array`` passed in from JS. May be empty if `mode` (specified in ``options``) is not `"full"`.
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
    args: ts::TsIntoRust<models::BlueprintArgs>,
    callbacks: ts::TsIntoRust<RunBlueprintCallbacks>,
) -> Result<JsValue, JsValue> {
    use crate::util::{JsCall, JsReflectGetSet, ResultJsExt, now_ms, wasm_shared_memory};
    use blueprint::{encoder::BlueprintEncoder, generator::BlueprintGenerator};
    use image::GenericImageView;
    use image_processing::{FrameCallback, FrameData};
    use js_sys as js;
    use models::Mode;
    use std::{
        cell::{Cell, RefCell},
        rc::Rc,
    };

    const SYNC_TIME_MS: f64 = 1000.0;

    let mut args: models::BlueprintArgs = args.to_rust()?;
    let callbacks: RunBlueprintCallbacks = callbacks.to_rust()?;
    let on_frame = Rc::new(callbacks.on_frame);
    let should_show_preview = Rc::new(callbacks.should_preview);
    let yield_to_js = Rc::new(callbacks.js_yield);

    let mut frame_data: Option<FrameData> = None;
    let promises = Rc::new(const { RefCell::new(Vec::<js::Promise<()>>::new()) });
    let promise_q = Rc::clone(&promises);
    let next_yield_time = Rc::new(Cell::new(now_ms() + SYNC_TIME_MS));

    if matches!(args.image_args.mode, Mode::Static { .. }) {
        args.signal_compression = None;
        args.image_args.target_fps = 1;
    }
    if matches!(args.image_args.mode, Mode::Full | Mode::Static { .. }) {
        let on_frame: FrameCallback = Box::new(move |img| {
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

                let js_args = js::Object::new();
                js_args.reflect_set("outBuffer", wasm_shared_memory(rgba)?)?;
                js_args.reflect_set(
                    "dimensions",
                    serde_wasm_bindgen::to_value(&img.dimensions())?,
                )?;
                on_frame.call_n((&js_args.into(),))?.await?;

                Ok(())
            })
        });

        let data_len = callbacks.get_data_len.call_n(())?.as_f64().unwrap();
        let reader =
            ioutil::JsFileReader::new(callbacks.get_data.clone(), data_len as u64, 1024 * 1024);

        // Create iterator over frames.
        frame_data = Some(FrameData::new(
            reader,
            &args.image_args,
            Some(on_frame),
            (args.image_args.mime_type)
                .starts_with("video/")
                .then(|| callbacks.get_next_frame.clone()),
        )?);
    }

    // Custom writer that invokes a callback whenever chunk size is reached.
    let writer = ioutil::BufWriter::new(1024 * 1024, !args.low_mem_usage, |chunk| {
        let data = js::Uint8Array::from(chunk.as_slice());
        let promise = callbacks.on_chunk.call_n((&data,))?;
        promise_q.borrow_mut().push(promise);
        Ok(())
    });

    // Yields JSON or compressed blueprint in chunks
    let generator = BlueprintGenerator::new(frame_data, &args).await?;
    let mut encoder = BlueprintEncoder::new(generator, &args, writer)?;

    let flush = async || -> Result<(), JsValue> {
        for promise in promises.take() {
            (promise).await?;
        }
        Ok(())
    };

    // `encoder.next()` may generate a series of chunks
    while encoder.next().await?.is_some() {
        flush().await?; // Wait for all those chunks to be sent
    }

    let mut writer = encoder.finish()?;
    writer.finish().map_js_err("Stream finish err")?;

    flush().await?; // Flush tail

    let obj = js::Object::new();
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
/// ``JsValue`` if any error occurs.
#[wasm_bindgen]
pub fn set_signal_data(data_json: js_sys::Uint8Array) -> Result<(), JsValue> {
    let reader = ioutil::Uint8ArrayReader::new(data_json);
    blueprint::data::load_data(reader)?;
    Ok(())
}

///
///
/// # Errors
///
/// ``JsValue`` if any error occurs.
#[wasm_bindgen]
pub fn set_comb_pos_data(data_json: js_sys::Uint8Array) -> Result<(), JsValue> {
    let reader = ioutil::Uint8ArrayReader::new(data_json);
    blueprint::combinator::load_comb_pos_json(reader)?;
    Ok(())
}

#[derive(tsify::Tsify, serde::Deserialize)]
pub struct EncodeGifCallbacks {
    #[serde(with = "serde_wasm_bindgen::preserve")]
    pub get_next_frame: image_processing::JsVideoFrameCallback,

    #[serde(with = "serde_wasm_bindgen::preserve")]
    pub on_chunk: js_sys::Function<fn(js_sys::Uint8Array) -> js_sys::Promise<()>>,
}

/// # Errors
///
/// ``JsValue`` if any error occurs.
#[wasm_bindgen]
pub async fn encode_gif(
    args: JsValue,
    callbacks: tsify::Ts<EncodeGifCallbacks>,
) -> Result<(), JsValue> {
    use image_processing::{JsVideoFrameCallback, encode_gif};
    use util::JsReflectGetSet;

    let args: models::EncodeGifArgs = serde_wasm_bindgen::from_value(args)?;
    let get_next_frame = callbacks.reflect_get::<JsVideoFrameCallback>("getFrameData")?;
    let on_chunk = callbacks
        .reflect_get::<js_sys::Function<fn(js_sys::Uint8Array) -> js_sys::Promise>>("onChunk")?;

    encode_gif(get_next_frame, on_chunk, &args).await?;

    Ok(())
}

#[wasm_bindgen]
#[must_use]
pub fn wasm_memory() -> JsValue {
    wasm_bindgen::memory()
}

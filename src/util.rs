use glam::{DVec2, IVec2, dvec2};
use wasm_bindgen::prelude::*;

pub fn bounding_box_midpoints(min: DVec2, max: DVec2) -> impl Iterator<Item = DVec2> {
    (min.y as i32..max.y as i32).flat_map(move |y| {
        (min.x as i32..max.x as i32).map(move |x| dvec2(x.into(), y.into()) + DVec2::splat(0.5))
    })
}

#[must_use]
pub fn format_duration<T: Into<u128>>(ms: T) -> String {
    let ms = ms.into();
    let total_seconds = ms.div_euclid(1000);

    let hours = total_seconds.div_euclid(3600);
    let minutes = (total_seconds.rem_euclid(3600)).div_euclid(60);
    let seconds = total_seconds.rem_euclid(60);
    let milliseconds = ms.rem_euclid(1000);

    match hours {
        0 => format!("{minutes:02}:{seconds:02}:{milliseconds:03}"),
        _ => format!("{hours:02}:{minutes:02}:{seconds:02}"),
    }
}

/// Checks if the value is the default value for the type.
#[inline]
#[must_use]
pub const fn is_default<T>(value: &T) -> bool
where T: [const] Default + [const] PartialEq + [const] std::marker::Destruct {
    value == &T::default()
}

/// Checks if an Option is None or the default value for the type.
#[inline]
#[must_use]
pub const fn is_none_or_default<T>(value: &Option<T>) -> bool
where T: [const] Default + [const] PartialEq + [const] std::marker::Destruct {
    if let Some(value) = value.as_ref() {
        return value == &T::default();
    }
    true
}

#[must_use]
#[inline]
pub fn now_ms() -> f64 {
    js_sys::Date::now()
}

pub type WasmSharedMemory = js_sys::Object;
pub fn wasm_shared_memory(data: &[u8]) -> Result<WasmSharedMemory, JsError> {
    let obj = js_sys::Object::new();

    obj.reflect_set("memory", wasm_bindgen::memory())?;
    obj.reflect_set("ptr", js_sys::Number::from(data.as_ptr() as u32))?;
    obj.reflect_set("len", js_sys::Number::from(data.len() as u32))?;

    Ok(obj)
}

#[derive(serde::Deserialize)]
#[serde(untagged)]
pub enum OneOrMany<T> {
    One(T),
    Many(Vec<T>),
}
impl<T> OneOrMany<T> {
    #[must_use]
    #[inline]
    pub fn into_vec(self) -> Vec<T> {
        match self {
            Self::One(v) => vec![v],
            Self::Many(v) => v,
        }
    }

    /// Returns a new clone of the contained value.
    #[must_use]
    #[inline]
    pub fn to_vec(&self) -> Vec<T>
    where T: Clone {
        match *self {
            Self::One(ref v) => vec![v.clone()],
            Self::Many(ref v) => v.clone(),
        }
    }
}

pub trait JsReflectGetSet: Into<JsValue> + Clone {
    #[inline]
    fn reflect_get<T: JsCast>(&self, k: impl Into<JsValue>) -> Result<T, JsError> {
        let result = js_sys::Reflect::get(&self.clone().into(), &k.into())
            .map_js_err("Error whilst getting key on Object");
        (result?.dyn_into()).map_js_err("Error whilst getting key on Object")
    }

    #[inline]
    fn reflect_set(&self, k: impl Into<JsValue>, v: impl Into<JsValue>) -> Result<bool, JsError> {
        js_sys::Reflect::set(&self.clone().into(), &k.into(), &v.into())
            .map_js_err("Error whilst setting key on Object")
    }
}
impl<T: Into<JsValue> + Clone> JsReflectGetSet for T {}

pub trait JsCall<T: js_sys::JsFunction>: Into<js_sys::Function<T>> + Clone {
    #[inline]
    fn call_n<Args: js_sys::JsArgs<T>>(&self, args: Args) -> Result<T::Ret, JsError> {
        args.apply_call(&self.clone().into(), &JsValue::NULL).map_js_err("Call error")
    }
}
impl<T: js_sys::JsFunction, S: Into<js_sys::Function<T>> + Clone> JsCall<T> for S {}

pub trait OptionJsExt<T> {
    fn ok_or_js_err(self, prefix: &str) -> Result<T, JsError>;
}
impl<T> OptionJsExt<T> for Option<T> {
    #[inline]
    fn ok_or_js_err(self, prefix: &str) -> Result<T, JsError> {
        self.ok_or_else(|| JsError::new(prefix))
    }
}

pub trait ResultJsExt<T, E> {
    fn map_js_err(self, prefix: &str) -> Result<T, JsError>;
}
impl<T, E: std::fmt::Debug> ResultJsExt<T, E> for Result<T, E> {
    #[inline]
    fn map_js_err(self, prefix: &str) -> Result<T, JsError> {
        self.map_err(|e| JsError::new(&format!("{prefix}: {e:?}")))
    }
}

#[inline]
pub fn hash_f64<H: std::hash::Hasher>(value: f64, state: &mut H) {
    use std::hash::Hash as _;

    let bits = if value.is_nan() {
        const { f64::NAN.to_bits() }
    } else if value == 0.0 {
        0
    } else {
        value.to_bits()
    };

    bits.hash(state);
}

#[inline]
#[must_use]
pub fn u8_histogram_4chunks(data: &[u8]) -> [u32; 256] {
    let mut h = [[0_u32; 256]; 4];

    let chunks = data.chunks_exact(4);
    let remainder = chunks.remainder();

    for chunk in chunks {
        h[0][chunk[0] as usize] += 1;
        h[1][chunk[1] as usize] += 1;
        h[2][chunk[2] as usize] += 1;
        h[3][chunk[3] as usize] += 1;
    }

    let mut result = [0_u32; 256];

    for i in 0..256 {
        result[i] = h[0][i] + h[1][i] + h[2][i] + h[3][i];
    }

    for &byte in remainder {
        result[byte as usize] += 1;
    }

    result
}

#[inline]
#[must_use]
pub fn u8_histogram_8chunks(data: &[u8]) -> [u32; 256] {
    let mut h = [[0_u32; 256]; 8];

    let chunks = data.chunks_exact(8);
    let remainder = chunks.remainder();

    for chunk in chunks {
        h[0][chunk[0] as usize] += 1;
        h[1][chunk[1] as usize] += 1;
        h[2][chunk[2] as usize] += 1;
        h[3][chunk[3] as usize] += 1;
        h[4][chunk[4] as usize] += 1;
        h[5][chunk[5] as usize] += 1;
        h[6][chunk[6] as usize] += 1;
        h[7][chunk[7] as usize] += 1;
    }

    let mut result = [0_u32; 256];

    for i in 0..256 {
        result[i] = h[0][i] + h[1][i] + h[2][i] + h[3][i] + h[4][i] + h[5][i] + h[6][i] + h[7][i];
    }

    for &byte in remainder {
        result[byte as usize] += 1;
    }

    result
}

pub const RAW_VALUE_COMMA: &serde_json::value::RawValue = raw_value_from_borrowed(",");

pub const fn raw_value_from_borrowed(value: &str) -> &serde_json::value::RawValue {
    // SAFETY: Uses same pattern serde_json::value::RawValue uses to construct
    // constants
    unsafe { std::mem::transmute::<&str, &serde_json::value::RawValue>(value) }
}

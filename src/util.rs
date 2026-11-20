use wasm_bindgen::prelude::*;

#[must_use]
pub fn format_duration<T: Into<u128>>(ms: T) -> String {
    let ms = ms.into();
    let total_seconds = ms / 1000;

    let hours = total_seconds / 3600;
    let minutes = (total_seconds % 3600) / 60;
    let seconds = total_seconds % 60;
    let milliseconds = ms % 1000;

    match hours {
        0 => format!("{minutes:02}:{seconds:02}:{milliseconds:03}"),
        _ => format!("{hours:02}:{minutes:02}:{seconds:02}"),
    }
}

/// Checks if the value is the default value for the type.
#[must_use]
pub fn is_default<T: Default + PartialEq>(value: &T) -> bool {
    value == &T::default()
}

/// Checks if an Option is None or the default value for the type.
#[must_use]
#[expect(clippy::ref_option, reason = "Use with serde")]
pub fn is_none_or_default<T: Default + PartialEq>(value: &Option<T>) -> bool {
    value.as_ref().is_none_or(|v| v == &T::default())
}

#[must_use]
#[inline]
pub fn now_ms() -> f64 {
    js_sys::Date::now()
}

pub fn wasm_shared_memory(data: &[u8]) -> Result<js_sys::Object, JsError> {
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
    pub fn into_vec(self) -> Vec<T> {
        match self {
            Self::One(v) => vec![v],
            Self::Many(v) => v,
        }
    }

    /// Returns a new clone of the contained value.
    pub fn to_vec(&self) -> Vec<T>
    where
        T: Clone,
    {
        match self {
            Self::One(v) => vec![v.clone()],
            Self::Many(v) => v.clone(),
        }
    }
}

pub trait JsReflectGetSet: Into<JsValue> + Clone {
    fn reflect_get<T: JsCast>(&self, k: impl Into<JsValue>) -> Result<T, JsError> {
        let result = js_sys::Reflect::get(&self.clone().into(), &k.into())
            .map_js_err("Error whilst getting key on Object");
        (result?.dyn_into()).map_js_err("Error whilst getting key on Object")
    }
    fn reflect_set(&self, k: impl Into<JsValue>, v: impl Into<JsValue>) -> Result<bool, JsError> {
        js_sys::Reflect::set(&self.clone().into(), &k.into(), &v.into())
            .map_js_err("Error whilst setting key on Object")
    }
}
impl<T: Into<JsValue> + Clone> JsReflectGetSet for T {}

pub trait JsCall<T: js_sys::JsFunction>: Into<js_sys::Function<T>> + Clone {
    fn call_n<Args: js_sys::JsArgs<T>>(&self, args: Args) -> Result<T::Ret, JsError> {
        args.apply_call(&self.clone().into(), &JsValue::NULL).map_js_err("Call error")
    }
}
impl<T: js_sys::JsFunction, S: Into<js_sys::Function<T>> + Clone> JsCall<T> for S {}

pub trait ResultJsExt<T, E> {
    fn map_js_err(self, prefix: &str) -> Result<T, JsError>;
}
impl<T, E: std::fmt::Debug> ResultJsExt<T, E> for Result<T, E> {
    fn map_js_err(self, prefix: &str) -> Result<T, JsError> {
        self.map_err(|e| JsError::new(&format!("{prefix}: {e:?}")))
    }
}

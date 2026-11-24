use glam::{DVec2, dvec2};
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
pub const fn is_default<T: [const] Default + [const] PartialEq + [const] core::marker::Destruct>(
    value: &T,
) -> bool {
    value == &T::default()
}

/// Checks if an Option is None or the default value for the type.
#[inline]
#[must_use]
#[expect(clippy::ref_option, reason = "Use with serde")]
pub const fn is_none_or_default<
    T: [const] Default + [const] PartialEq + [const] core::marker::Destruct,
>(
    value: &Option<T>,
) -> bool {
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
    where
        T: Clone,
    {
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
impl<T, E: core::fmt::Debug> ResultJsExt<T, E> for Result<T, E> {
    #[inline]
    fn map_js_err(self, prefix: &str) -> Result<T, JsError> {
        self.map_err(|e| JsError::new(&format!("{prefix}: {e:?}")))
    }
}

#[allow(
    clippy::allow_attributes,
    dead_code,
    reason = "Overarching convenience lib"
)]
pub trait TryIteratorExt: Iterator {
    #[inline]
    fn try_any<E, F: FnMut(Self::Item) -> Result<bool, E>>(&mut self, mut f: F) -> Result<bool, E>
    where
        Self: Sized,
    {
        match self.try_fold((), |(), item| match f(item) {
            Ok(true) => ::core::ops::ControlFlow::Break(Ok(true)),
            Ok(false) => ::core::ops::ControlFlow::Continue(()),
            Err(err) => ::core::ops::ControlFlow::Break(Err(err)),
        }) {
            ::core::ops::ControlFlow::Break(result) => result,
            ::core::ops::ControlFlow::Continue(..) => Ok(false),
        }
    }

    #[inline]
    fn try_all<E, F: FnMut(Self::Item) -> Result<bool, E>>(&mut self, mut f: F) -> Result<bool, E>
    where
        Self: Sized,
    {
        match self.try_fold((), |(), item| match f(item) {
            Ok(true) => ::core::ops::ControlFlow::Continue(()),
            Ok(false) => ::core::ops::ControlFlow::Break(Ok(false)),
            Err(err) => ::core::ops::ControlFlow::Break(Err(err)),
        }) {
            ::core::ops::ControlFlow::Break(result) => result,
            ::core::ops::ControlFlow::Continue(..) => Ok(true),
        }
    }
}

impl<I: Iterator + ?Sized> TryIteratorExt for I {}

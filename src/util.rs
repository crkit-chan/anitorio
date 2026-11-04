use wasm_bindgen::prelude::*;

pub fn js_num<N: Into<f64>>(num: N) -> js_sys::Number {
    js_sys::Number::from(num.into())
}

pub fn format_duration<T: Into<u128>>(ms: T) -> String {
    let ms = ms.into();

    let total_seconds = ms / 1000;

    let hours = total_seconds / 3600;
    let minutes = (total_seconds % 3600) / 60;
    let seconds = total_seconds % 60;

    if hours == 0 {
        format!("{minutes:02}:{seconds:02}")
    } else {
        format!("{hours:02}:{minutes:02}:{seconds:02}")
    }
}

/// Checks if the value is the default value for the type.
pub fn is_default<T: Default + PartialEq>(value: &T) -> bool {
    value == &T::default()
}

/// Checks if an Option is None or the default value for the type.
#[expect(clippy::ref_option, reason = "Use with serde")]
pub fn is_none_or_default<T: Default + PartialEq>(value: &Option<T>) -> bool {
    value.as_ref().is_none_or(|v| v == &T::default())
}

pub fn now_ms() -> f64 {
    js_sys::Date::now()
}

pub fn wasm_shared_memory(data: &[u8]) -> Result<js_sys::Object, JsValue> {
    let obj = js_sys::Object::new();

    js_sys::Reflect::set(&obj, &"memory".into(), &wasm_bindgen::memory())?;
    js_sys::Reflect::set(&obj, &"ptr".into(), &js_num(data.as_ptr() as u32))?;
    js_sys::Reflect::set(&obj, &"len".into(), &js_num(data.len() as u32))?;

    Ok(obj)
}

#[derive(serde::Deserialize)]
#[serde(untagged)]
pub enum OneOrMany<T> {
    One(T),
    Many(Vec<T>),
}
impl<T: Clone> OneOrMany<T> {
    /// Returns a new clone of the contained value.
    pub fn to_vec(&self) -> Vec<T> {
        match self {
            Self::One(v) => vec![v.clone()],
            Self::Many(v) => v.clone(),
        }
    }
}

pub trait JsReflectGetSet: Into<JsValue> + Clone {
    fn reflect_get<T: JsCast>(&self, k: impl Into<JsValue>) -> Result<T, JsValue> {
        js_sys::Reflect::get(&self.clone().into(), &k.into())?.dyn_into()
    }
    fn reflect_set(&self, k: impl Into<JsValue>, v: impl Into<JsValue>) -> Result<bool, JsValue> {
        js_sys::Reflect::set(&self.clone().into(), &k.into(), &v.into())
    }
}
impl<T: Into<JsValue> + Clone> JsReflectGetSet for T {}

pub trait JsCall<T: js_sys::JsFunction>: Into<js_sys::Function<T>> + Clone {
    fn call_n<Args: js_sys::JsArgs<T>>(&self, args: Args) -> Result<T::Ret, JsValue> {
        args.apply_call(&self.clone().into(), &JsValue::NULL)
    }
}
impl<T: js_sys::JsFunction, S: Into<js_sys::Function<T>> + Clone> JsCall<T> for S {}

pub trait ResultJsExt<T, E> {
    fn map_js_err(self, prefix: &str) -> Result<T, JsValue>;
}
impl<T, E: std::fmt::Display> ResultJsExt<T, E> for Result<T, E> {
    fn map_js_err(self, prefix: &str) -> Result<T, JsValue> {
        self.map_err(|e| JsValue::from_str(&format!("{prefix}: {e}")))
    }
}

macro_rules! glam_vec2_const_fn {
    ($name:literal $op:tt $ty:ident) => {
        paste::paste! {
            pub const fn [<$ty:lower _$name>](lhs: glam::$ty, rhs: glam::$ty) -> glam::$ty {
                glam::$ty::new(lhs.x $op rhs.x, lhs.y $op rhs.y)
            }
        }
    };
}
macro_rules! glam_vec2_custom_const_fn {
    ($name:literal, $ty:ident) => {
        paste::paste! {
            pub const fn [<$ty:lower _$name>](value: glam::$ty) -> glam::$ty {
                glam::$ty::new(value.x.[<$name>](), value.y.[<$name>]())
            }
        }
    };
}
macro_rules! glam_vec2_arithmetic_const_fns {
    ($ty:ident) => {
        paste::paste! {
            glam_vec2_const_fn!("add" + $ty);
            glam_vec2_const_fn!("div" / $ty);
            glam_vec2_const_fn!("mod" % $ty);
            glam_vec2_const_fn!("mul" * $ty);
            glam_vec2_const_fn!("sub" - $ty);
        }
    };
}
macro_rules! glam_vec2_bitwise_const_fns {
    ($ty:ident) => {
        paste::paste! {
            glam_vec2_const_fn!("bitand" &  $ty);
            glam_vec2_const_fn!("bitor"  |  $ty);
            glam_vec2_const_fn!("bitxor" ^  $ty);
            glam_vec2_const_fn!("shl"    << $ty);
            glam_vec2_const_fn!("shr"    >> $ty);
        }
    };
}
macro_rules! glam_vec2_signed_const_fns {
    ($ty:ident) => {
        paste::paste! {
            glam_vec2_custom_const_fn!("abs",    $ty);
            glam_vec2_custom_const_fn!("signum", $ty);
        }
    };
}
macro_rules! glam_vec2_floating_const_fns {
    ($ty:ident) => {
        paste::paste! {
            glam_vec2_custom_const_fn!("floor",  $ty);
            glam_vec2_custom_const_fn!("fract",  $ty);
            glam_vec2_custom_const_fn!("round",  $ty);
        }
    };
}

#[allow(clippy::allow_attributes, dead_code, reason = "Extensive API")]
pub mod dvec_const_fns {
    glam_vec2_arithmetic_const_fns!(Vec2);
    glam_vec2_arithmetic_const_fns!(DVec2);
    glam_vec2_arithmetic_const_fns!(I8Vec2);
    glam_vec2_arithmetic_const_fns!(U8Vec2);
    glam_vec2_arithmetic_const_fns!(I16Vec2);
    glam_vec2_arithmetic_const_fns!(U16Vec2);
    glam_vec2_arithmetic_const_fns!(IVec2);
    glam_vec2_arithmetic_const_fns!(UVec2);
    glam_vec2_arithmetic_const_fns!(I64Vec2);
    glam_vec2_arithmetic_const_fns!(U64Vec2);
    glam_vec2_arithmetic_const_fns!(ISizeVec2);
    glam_vec2_arithmetic_const_fns!(USizeVec2);

    glam_vec2_bitwise_const_fns!(I8Vec2);
    glam_vec2_bitwise_const_fns!(U8Vec2);
    glam_vec2_bitwise_const_fns!(I16Vec2);
    glam_vec2_bitwise_const_fns!(U16Vec2);
    glam_vec2_bitwise_const_fns!(IVec2);
    glam_vec2_bitwise_const_fns!(UVec2);
    glam_vec2_bitwise_const_fns!(I64Vec2);
    glam_vec2_bitwise_const_fns!(U64Vec2);
    glam_vec2_bitwise_const_fns!(ISizeVec2);
    glam_vec2_bitwise_const_fns!(USizeVec2);

    glam_vec2_floating_const_fns!(Vec2);
    glam_vec2_floating_const_fns!(DVec2);

    glam_vec2_signed_const_fns!(DVec2);
    glam_vec2_signed_const_fns!(Vec2);
    glam_vec2_signed_const_fns!(I8Vec2);
    glam_vec2_signed_const_fns!(I16Vec2);
    glam_vec2_signed_const_fns!(IVec2);
    glam_vec2_signed_const_fns!(I64Vec2);
    glam_vec2_signed_const_fns!(ISizeVec2);
}

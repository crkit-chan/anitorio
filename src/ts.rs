use serde::de::DeserializeOwned;
use wasm_bindgen::{
    ErasableGeneric,
    convert::{FromWasmAbi, IntoWasmAbi, UpcastFrom},
    describe::WasmDescribe,
    prelude::*,
};

/// Typed as `boolean` in TypeScript
pub type TsBool = js_sys::Boolean;

/// Typed as `type TsFn<T> = T` in TypeScript (wrapper type).
pub type TsFn<T> = js_sys::Function<T>;

/// Typed as `Promise<T>` in TypeScript. Generic `E` is discarded in TypeScript.
pub type TsFut<T = (), E = JsError> = js_sys::Promise<T, E>;

/// Typed as `number` in TypeScript
pub type TsNum = js_sys::Number;

/// Typed as `string` in TypeScript
pub type TsStr = js_sys::JsString;

/// Typed as a `Uint8ClampedArray<ArrayBuffer>` in TypeScript.
/// Note that this should be used to represent a buffer whose memory is backed in Rust.
pub type TsUint8ClampedArrayBacked = js_sys::Uint8ClampedArray;

/// Typed as `void` in TypeScript.
pub type TsVoid = js_sys::Undefined;

#[repr(transparent)]
pub struct TsIntoRust<T: DeserializeOwned> {
    value: JsValue,
    _marker: std::marker::PhantomData<fn() -> T>,
}
impl<T: DeserializeOwned> TsIntoRust<T> {
    #[expect(clippy::wrong_self_convention, reason = "Matches `tsify` API")]
    pub fn to_rust(self) -> Result<T, serde_wasm_bindgen::Error> {
        serde_wasm_bindgen::from_value(self.value)
    }
}
impl<T: DeserializeOwned> From<JsValue> for TsIntoRust<T> {
    fn from(value: JsValue) -> Self {
        Self {
            value,
            _marker: std::marker::PhantomData,
        }
    }
}
impl<T: DeserializeOwned> From<TsIntoRust<T>> for JsValue {
    fn from(value: TsIntoRust<T>) -> Self {
        value.value
    }
}
impl<T: DeserializeOwned> FromWasmAbi for TsIntoRust<T> {
    type Abi = <JsValue as FromWasmAbi>::Abi;
    unsafe fn from_abi(js: Self::Abi) -> Self {
        Self::from(unsafe { JsValue::from_abi(js) })
    }
}
impl<T: DeserializeOwned> WasmDescribe for TsIntoRust<T> {
    fn describe() {
        <JsValue as WasmDescribe>::describe();
    }
}
unsafe impl<T: DeserializeOwned> ErasableGeneric for TsIntoRust<T> {
    type Repr = JsValue;
}
impl<T: DeserializeOwned> IntoWasmAbi for TsIntoRust<T> {
    type Abi = <JsValue as IntoWasmAbi>::Abi;

    fn into_abi(self) -> Self::Abi {
        self.value.into_abi()
    }
}

#[allow(clippy::allow_attributes, reason = "may be fixed in future")]
#[allow(clippy::use_self, reason = "clippy bug")]
impl<T: DeserializeOwned> UpcastFrom<TsIntoRust<T>> for TsIntoRust<T> {} // A wrapper can upcast to itself.
impl<T: DeserializeOwned> UpcastFrom<TsIntoRust<T>> for JsValue {} // A wrapper can upcast to JsValue.

// JsValue access.
impl<T: DeserializeOwned> AsRef<JsValue> for TsIntoRust<T> {
    fn as_ref(&self) -> &JsValue {
        &self.value
    }
}

// JsCast implementation.
impl<T: DeserializeOwned> JsCast for TsIntoRust<T> {
    fn instanceof(_value: &JsValue) -> bool {
        true
    }

    fn unchecked_from_js(value: JsValue) -> Self {
        Self::from(value)
    }

    fn unchecked_from_js_ref(value: &JsValue) -> &Self {
        unsafe { &*std::ptr::from_ref::<JsValue>(value).cast::<Self>() }
    }
}

#[wasm_bindgen(typescript_custom_section)]
const TS_TYPES: &str = r#"
export type TsBool = boolean;
export type TsFn<T extends (...args: readonly any[]) => any> = T;
export type TsFut<Ret = void, Err = any> = Promise<Ret>;
export type TsIntoRust<T> = T;
export type TsNum = number;
export type TsStr = string;
export type TsUint8ClampedArrayBacked = Uint8ClampedArray<ArrayBuffer>;
export type TsVoid = void;
"#;

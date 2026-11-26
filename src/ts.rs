use serde::Serialize;
use serde::de::DeserializeOwned;
use wasm_bindgen::ErasableGeneric;
use wasm_bindgen::convert::{FromWasmAbi, IntoWasmAbi, UpcastFrom};
use wasm_bindgen::describe::WasmDescribe;
use wasm_bindgen::prelude::*;

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
/// Note that this should be used to represent a buffer whose memory is backed
/// in Rust.
pub type TsUint8ClampedArrayBacked = js_sys::Uint8ClampedArray;

/// Typed as `void` in TypeScript.
pub type TsVoid = js_sys::Undefined;

#[repr(transparent)]
pub struct TsIntoRust<T: DeserializeOwned> {
    value:   JsValue,
    _marker: std::marker::PhantomData<fn() -> T>,
}
impl<T: DeserializeOwned> TsIntoRust<T> {
    #[expect(clippy::wrong_self_convention, reason = "Matches `tsify` API")]
    pub fn to_rust(self) -> Result<T, serde_wasm_bindgen::Error> {
        serde_wasm_bindgen::from_value(self.value)
    }

    pub fn new(value: &T) -> Result<Self, JsError>
    where T: tsify::Tsify + Serialize {
        let value = value.into_js()?.into();
        Ok(Self { value, _marker: std::marker::PhantomData })
    }
}
impl<T: DeserializeOwned> const From<JsValue> for TsIntoRust<T> {
    fn from(value: JsValue) -> Self {
        Self { value, _marker: std::marker::PhantomData }
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
        // SAFETY: `TsIntoRust<T>` uses the same ABI representation as `JsValue`.
        // The incoming ABI value is therefore valid to reconstruct as a `JsValue`.
        Self::from(unsafe { JsValue::from_abi(js) })
    }
}
impl<T: DeserializeOwned> WasmDescribe for TsIntoRust<T> {
    fn describe() {
        <JsValue as WasmDescribe>::describe();
    }
}
// SAFETY: `TsIntoRust<T>` is `#[repr(transparent)]` over `JsValue`.
// Its only additional field is `PhantomData`, which has no runtime
// representation.
unsafe impl<T: DeserializeOwned> ErasableGeneric for TsIntoRust<T> {
    type Repr = JsValue;
}
impl<T: DeserializeOwned> IntoWasmAbi for TsIntoRust<T> {
    type Abi = <JsValue as IntoWasmAbi>::Abi;

    fn into_abi(self) -> Self::Abi {
        self.value.into_abi()
    }
}

#[allow(clippy::use_self, reason = "clippy bug")]
impl<T: DeserializeOwned> UpcastFrom<TsIntoRust<T>> for TsIntoRust<T> {} // A wrapper can upcast to itself.
impl<T: DeserializeOwned> UpcastFrom<TsIntoRust<T>> for JsValue {} // A wrapper can upcast to JsValue.

// JsValue access.
impl<T: DeserializeOwned> const AsRef<JsValue> for TsIntoRust<T> {
    fn as_ref(&self) -> &JsValue {
        &self.value
    }
}

// JsCast implementation.
impl<T: DeserializeOwned> JsCast for TsIntoRust<T> {
    fn instanceof(_value: &JsValue) -> bool {
        true
    }

    fn unchecked_from_js(val: JsValue) -> Self {
        Self::from(val)
    }

    fn unchecked_from_js_ref(val: &JsValue) -> &Self {
        // SAFETY: `TsIntoRust<T>` is `#[repr(transparent)]` over `JsValue`.
        // The `PhantomData` field has no runtime representation, so both types
        // have identical layouts and alignment. The returned reference cannot
        // outlive the input reference.
        unsafe { &*std::ptr::from_ref::<JsValue>(val).cast::<Self>() }
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

#![expect(non_camel_case_types, reason = "Compatbility with TypeScript")]

use wasm_bindgen::JsValue;

pub type boolean = js_sys::Boolean;
pub type number = js_sys::Number;

pub type TsFn<T> = js_sys::Function<T>;

pub struct TsIntoRust<T: serde::de::DeserializeOwned> {
    value: JsValue,
    _marker: std::marker::PhantomData<T>,
}
impl<T: serde::de::DeserializeOwned> TsIntoRust<T> {
    #[expect(clippy::wrong_self_convention, reason = "Matches `tsify` API")]
    pub fn to_rust(self) -> Result<T, serde_wasm_bindgen::Error> {
        serde_wasm_bindgen::from_value(self.value)
    }
}
impl<T: serde::de::DeserializeOwned> From<JsValue> for TsIntoRust<T> {
    fn from(value: JsValue) -> Self {
        Self {
            value,
            _marker: std::marker::PhantomData,
        }
    }
}
impl<T: serde::de::DeserializeOwned> From<TsIntoRust<T>> for JsValue {
    fn from(value: TsIntoRust<T>) -> Self {
        value.value
    }
}
impl<T: serde::de::DeserializeOwned> wasm_bindgen::convert::FromWasmAbi for TsIntoRust<T> {
    type Abi = <JsValue as wasm_bindgen::convert::FromWasmAbi>::Abi;
    unsafe fn from_abi(js: Self::Abi) -> Self {
        Self::from(unsafe { JsValue::from_abi(js) })
    }
}
impl<T: serde::de::DeserializeOwned> wasm_bindgen::describe::WasmDescribe for TsIntoRust<T> {
    fn describe() {
        <JsValue as wasm_bindgen::describe::WasmDescribe>::describe();
    }
}

#[wasm_bindgen::prelude::wasm_bindgen(typescript_custom_section)]
const TS_TYPES: &str = r#"
export type TsFn<T extends (...args: readonly any[]) => any> = T;
export type TsIntoRust<T> = T;
"#;

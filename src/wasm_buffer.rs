use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub struct WasmBuffer {
    data: Vec<u8>,
}
#[wasm_bindgen]
impl WasmBuffer {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        Self { data: vec![] }
    }
    pub fn with_capacity(len: usize) -> Self {
        Self { data: vec![0; len] }
    }

    pub fn ptr(&self) -> u32 {
        self.data.as_ptr() as u32
    }

    pub fn resize(&mut self, len: usize) {
        self.data.resize(len, 0)
    }
    pub fn take(self) -> Vec<u8> {
        self.data
    }
    pub fn take_and_resize(&mut self, len: usize) -> Vec<u8> {
        std::mem::replace(&mut self.data, vec![0; len])
    }
    pub fn take_mut(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.data)
    }
}

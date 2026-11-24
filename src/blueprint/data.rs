use super::prelude::*;
use crate::prelude::*;
use alloc::sync::Arc;
use core::cell::RefCell;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

thread_local! {
    pub static PROTOTYPE_DATA: RefCell<PrototypeData> = RefCell::new(PrototypeData {
        electric_poles: boxed_slice![],
        qualities:      boxed_slice![],

        blacklist_signals: boxed_slice![],
        signal_ids:        boxed_slice![],
        signals:           boxed_slice![],
        signal_types:      BoundedSignalTypes { types: HashMap::new() },
    });
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ElectricPoleSupplyAreaDistancePrototypeData {
    pub quality:              String,
    pub supply_area_distance: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ElectricPolePrototypeData {
    pub name:                  String,
    pub supply_area_distances: Box<[ElectricPoleSupplyAreaDistancePrototypeData]>,
    pub tile_width:            i32,
    pub tile_height:           i32,
}

#[derive(Clone, Debug)]
pub struct PrototypeData {
    pub electric_poles: Box<[ElectricPolePrototypeData]>,
    pub qualities:      Box<[String]>,

    /// Tracks signal names added for compatibility (e.g. if the original signal list didn't include
    /// decider combinators, this list tracks that decider combinators were manually added).
    /// We need to add our own signals since Entity serialization depends on signals existing.
    pub blacklist_signals: Box<[SignalID]>,
    pub signal_ids:        Box<[SignalID]>,
    pub signals:           Box<[String]>,
    pub signal_types:      BoundedSignalTypes,
}

#[derive(Clone, Debug)]
pub struct BoundedSignalTypes {
    types: HashMap<core::ops::Range<usize>, String>,
}
impl BoundedSignalTypes {
    #[inline]
    #[must_use]
    #[expect(clippy::unwrap_used, reason = "Convenience function")]
    pub fn get_type(&self, index: usize) -> &str {
        self.try_get_type(index)
            .ok_or_else(|| js_err!("Invalid signal index '{index}', valid: {:?}", self.types))
            .unwrap()
    }

    #[inline]
    #[must_use]
    #[expect(clippy::unwrap_used, reason = "Convenience function")]
    pub fn get_range(&self, type_: &str) -> core::ops::Range<usize> {
        self.try_get_range(type_)
            .ok_or_else(|| js_err!("Unknown signal type '{type_}'"))
            .unwrap()
    }

    #[inline]
    #[must_use]
    pub fn has_type(&self, type_: &str) -> bool {
        self.types.values().any(|v| v == type_)
    }

    #[inline]
    #[must_use]
    pub fn try_get_type(&self, index: usize) -> Option<&str> {
        (self.types.iter())
            .find_map(|(bounds, type_)| bounds.contains(&index).then_some(type_.as_str()))
    }

    #[inline]
    #[must_use]
    pub fn try_get_range(&self, type_: &str) -> Option<core::ops::Range<usize>> {
        (self.types.iter()).find_map(|(bounds, t)| (t == type_).then(|| bounds.clone()))
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct SignalIDRepr {
    #[serde(rename = "type", default)]
    pub type_:   Option<String>,
    pub name:    String,
    #[serde(default)]
    pub quality: Option<String>,
}
impl SignalIDRepr {
    #[inline]
    #[must_use]
    pub fn new(type_: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            type_:   Some(type_.into()),
            name:    name.into(),
            quality: None,
        }
    }

    #[inline]
    #[must_use]
    pub fn with_quality(mut self, quality: impl Into<Option<String>>) -> Self {
        self.quality = quality.into();
        self
    }
}

/// Enhances the provided signals by associating quality levels based on DLC usage.
///
/// # Arguments
///
/// * `data_json_reader` - The raw JSON string of signals, as a ``Uint8Array``.
pub fn load_data<R: std::io::Read>(data_json_reader: R) -> Result<(), JsError> {
    #[derive(Deserialize)]
    struct PrototypeDataJson {
        electric_poles: Box<[ElectricPolePrototypeData]>,
        qualities:      Vec<String>,
        signals:        Vec<SignalIDRepr>,
    }

    let mut data_json: PrototypeDataJson = serde_json::from_reader(data_json_reader)?;
    let mut bounded_types = HashMap::<String, core::ops::Range<usize>>::new();
    let mut signals = Vec::<String>::with_capacity(data_json.signals.len());
    let mut blacklist_signals = Vec::<SignalIDRepr>::new();

    for (type_, names) in [
        ("virtual", vec![EACH]),
        ("entity", vec![ARI_CB, CONSTANT_CB, DEC_CB, LAMP]),
    ] {
        for name in names {
            if !data_json.signals.iter().try_any(|s| -> Result<_, JsError> {
                Ok(s.name == name && {
                    let t = (s.type_.as_ref())
                        .ok_or_js_err("Input signal data missing 'type' field")?;
                    type_ == t
                })
            })? {
                let sig = SignalIDRepr {
                    name:    name.to_owned(),
                    type_:   Some(type_.to_owned()),
                    quality: None, // Default quality
                };
                blacklist_signals.push(sig.clone());
                data_json.signals.push(sig);
            }
        }
    }
    data_json.signals.sort_by(|a, b| a.type_.cmp(&b.type_));

    for (i, sig) in core::mem::take(&mut data_json.signals).into_iter().enumerate() {
        let type_ = sig.type_.ok_or_else(|| js_err!(""))?;
        if let Some(bounded_type) = bounded_types.get_mut(&type_) {
            bounded_type.end = i + 1;
        } else {
            bounded_types.insert(type_, i..i);
        }
        signals.push(sig.name);
    }

    PROTOTYPE_DATA.with_borrow_mut(|prev| {
        for signal_id in core::mem::take(&mut prev.signal_ids) {
            assert_eq!(
                Arc::strong_count(&signal_id.0),
                1,
                "Too many references to existing prototype data!",
            );
        }

        *prev = PrototypeData {
            signal_ids:   (0..signals.len() * data_json.qualities.len().max(1))
                .map(|i| SignalID(Arc::new(i)))
                .collect(),
            signals:      signals.into_boxed_slice(),
            signal_types: BoundedSignalTypes {
                types: bounded_types.iter().map(|(k, v)| (v.clone(), k.clone())).collect(),
            },

            blacklist_signals: vec![].into(), // We'll assign this just afterwards
            electric_poles:    core::mem::take(&mut data_json.electric_poles),
            qualities:         core::mem::take(&mut data_json.qualities).into_boxed_slice(),
        };
    });

    // Note: we must create this OUTSIDE of the mutable borrow, since SignalID::try_from
    // will need access to the PROTOTYPE_DATA
    let blacklist_signals = (blacklist_signals.into_iter())
        .map(SignalID::try_from)
        .collect::<Result<Box<_>, _>>()?;

    PROTOTYPE_DATA.with_borrow_mut(|data| data.blacklist_signals = blacklist_signals);

    Ok(())
}

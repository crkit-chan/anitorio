use super::prelude::*;
use crate::prelude::*;
use serde::{Deserialize, Serialize};
use std::{cell::RefCell, collections::HashSet, sync::Arc};

type SignalsHasher = std::hash::BuildHasherDefault<std::collections::hash_map::DefaultHasher>;

thread_local! {
    static PROTOTYPE_DATA: RefCell<Option<PrototypeData>> = const { RefCell::new(None) };
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ElectricPoleSupplyAreaDistancePrototypeData {
    pub quality:              Arc<str>,
    pub supply_area_distance: f64,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ElectricPolePrototypeData {
    pub name:                  Arc<str>,
    pub supply_area_distances: Box<[ElectricPoleSupplyAreaDistancePrototypeData]>,
    pub tile_width:            i32,
    pub tile_height:           i32,
}

#[derive(Clone, Default)]
pub struct PrototypeData {
    pub blacklist_signals: Vec<Arc<SignalID>>,
    pub electric_poles:    Vec<ElectricPolePrototypeData>,
    pub signals:           indexmap::IndexSet<Arc<SignalID>, SignalsHasher>,
}

const DEFAULT_SIGNALS: &[(&str, &[&str])] = &[
    ("entity", &[ARI_CB, CONSTANT_CB, DEC_CB, LAMP]),
    ("virtual", &[EACH]),
];

/// Enhances the provided signals by associating quality levels based on DLC usage.
///
/// # Arguments
///
/// * `data_json_reader` - The raw JSON string of signals, as a ``Uint8Array``.
///
/// # Returns
///
/// A new vector of signal JSON objects with added quality attributes.
pub fn load_data<R: std::io::Read>(data_json_reader: R) -> Result<(), JsError> {
    #[derive(Deserialize)]
    struct PrototypeDataRepr {
        electric_poles: Vec<ElectricPolePrototypeData>,
        #[serde(default)]
        qualities:      Vec<String>,
        signals:        indexmap::IndexSet<SignalIDRepr, SignalsHasher>,
    }

    #[derive(Deserialize, Clone, Hash, PartialEq, Eq)]
    struct SignalIDRepr {
        #[serde(rename = "type")]
        type_: String,
        name:  String,
    }

    let mut data_repr: PrototypeDataRepr = serde_json::from_reader(data_json_reader)?;
    let mut blacklist_signals = Vec::<SignalIDRepr>::new();

    log!("HEREa");

    // #[crate::perf_timed("Populate prototype data")]
    let qualities: Box<_> = data_repr.qualities.into_iter().map(Arc::from).collect();
    let mut signal_names = HashSet::<Arc<str>>::new();
    let mut signal_types = HashSet::<Arc<str>>::new();
    let mut signals = indexmap::IndexSet::with_hasher(SignalsHasher::new());

    for &(type_, names) in DEFAULT_SIGNALS {
        for &name in names {
            let sig_repr = SignalIDRepr { type_: type_.to_owned(), name: name.to_owned() };
            if data_repr.signals.insert(sig_repr.clone()) {
                blacklist_signals.push(sig_repr);
            }
        }
    }

    for sig in data_repr.signals {
        #[allow(
            clippy::allow_attributes,
            clippy::option_if_let_else,
            reason = "Clippy bug"
        )]
        let (type_, name) = (Arc::from(sig.type_), Arc::from(sig.name));

        signal_types.insert(Arc::clone(&type_));
        signal_names.insert(Arc::clone(&name));

        let type_ = signal_types.get(&type_).unwrap();
        let name = signal_names.get(&name).unwrap();

        for quality in &qualities {
            signals.insert(Arc::from(SignalID {
                type_:   Arc::clone(type_),
                name:    Arc::clone(name),
                quality: Arc::clone(quality),
            }));
        }
    }

    let blacklist_signals = (blacklist_signals.into_iter().map(|sig| {
        let ref_sig = SignalID::new(sig.type_.into(), sig.name.into(), DEFAULT_QUALITY.into());
        let sig = (signals.get(&Arc::from(ref_sig.clone())))
            .ok_or_else(|| js_err!("Could not find blacklisted signal '{ref_sig:?}'"))?;
        Ok(Arc::clone(sig))
    }))
    .collect::<Result<Vec<_>, JsError>>()?;

    log!("HERE");

    PROTOTYPE_DATA.replace(Some(PrototypeData {
        blacklist_signals,
        electric_poles: data_repr.electric_poles,
        signals,
    }));

    Ok(())
}

#[expect(clippy::expect_used, reason = "Convenience API")]
#[inline]
pub fn with_prototype_data<R>(f: impl FnOnce(&PrototypeData) -> R) -> R {
    PROTOTYPE_DATA.with_borrow(|data| f(data.as_ref().expect("Prototypes not initialized yet!")))
}

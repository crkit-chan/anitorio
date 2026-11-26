use std::cell::RefCell;
use std::collections::HashSet;

use indexmap::IndexSet;
use serde::{Deserialize, Serialize};

use super::prelude::*;
use crate::prelude::*;

thread_local! {
    static PROTOTYPE_DATA: RefCell<PrototypeData> = RefCell::new(PrototypeData {
        blacklist_signals: HashSet::new(),
        electric_poles:    Vec::new(),
        signals:           IndexSet::new(),
    });
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ElectricPoleSupplyAreaDistancePrototypeData {
    pub quality:              RC<str>,
    pub supply_area_distance: f64,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ElectricPolePrototypeData {
    pub name:                  RC<str>,
    pub supply_area_distances: Box<[ElectricPoleSupplyAreaDistancePrototypeData]>,
    pub tile_width:            i32,
    pub tile_height:           i32,
}

#[derive(Clone, Default)]
pub struct PrototypeData {
    pub blacklist_signals: HashSet<RC<SignalID>>,
    pub electric_poles:    Vec<ElectricPolePrototypeData>,
    pub signals:           IndexSet<RC<SignalID>>,
}

const DEFAULT_SIGNALS: &[(&str, &[&str])] = &[
    ("entity", &[ARI_CB, CONSTANT_CB, DEC_CB, LAMP]),
    ("virtual", &[EACH]),
];

/// Loads the signals from JSON.
///
/// # Arguments
///
/// * `data_json_reader` - The raw JSON string of signals, as a ``Uint8Array``.
///
/// # Returns
///
/// A new vector of signal JSON objects with added quality attributes.
#[crate::perf("Populate prototype data")]
pub fn load_data<R: std::io::Read>(data_json_reader: R) -> Result<(), JsError> {
    #[derive(Deserialize)]
    struct PrototypeDataRepr {
        electric_poles: Vec<ElectricPolePrototypeData>,
        #[serde(default)]
        qualities:      Vec<String>,
        signals:        IndexSet<SignalIDRepr>,
    }

    #[derive(Deserialize, Clone, Hash, PartialEq, Eq)]
    struct SignalIDRepr {
        #[serde(rename = "type")]
        type_: String,
        name:  String,
    }

    #[crate::perf("Make data repr")]
    let mut data_repr: PrototypeDataRepr = serde_json::from_reader(data_json_reader)?;

    PROTOTYPE_DATA.with_borrow_mut(|data| {
        data.blacklist_signals.clear();
        data.signals.clear();

        let signal_qualities: IndexSet<_> = data_repr.qualities.into_iter().map(RC::from).collect();
        let mut blacklist_signals = Vec::<SignalIDRepr>::new();
        let mut uniq_strings: HashSet<_> = signal_qualities.iter().cloned().collect();

        // Add required signals to signal list + all power poles.
        // Non-user signals are tracked using ``blacklist-_signals``.
        for (type_, name) in (DEFAULT_SIGNALS.iter())
            .flat_map(|&(type_, names)| names.iter().map(move |&name| (type_, name)))
            .chain(data_repr.electric_poles.iter().map(|pole| ("entity", pole.name.as_ref())))
        {
            let sig_repr = SignalIDRepr { type_: type_.to_owned(), name: name.to_owned() };

            // If data_repr.signals did not contain this sig_repr, we add
            // blacklist_signals to indicate it should be ommitted.
            if data_repr.signals.insert(sig_repr.clone()) {
                blacklist_signals.push(sig_repr);
            }
        }

        #[crate::perf("Iterate signals")]
        for sig in data_repr.signals {
            let type_ = &RC::clone(uniq_strings.get_or_insert(sig.type_.into()));
            let name = &RC::clone(uniq_strings.get_or_insert(sig.name.into()));

            for quality in &signal_qualities {
                data.signals.insert(RC::from(SignalID {
                    type_:   RC::clone(type_),
                    name:    RC::clone(name),
                    quality: RC::clone(quality),
                }));
            }
        }

        data.blacklist_signals = (blacklist_signals.into_iter().map(|sig| {
            let ref_sig = SignalID::new(sig.type_.into(), sig.name.into(), DEFAULT_QUALITY.into());
            let sig = (data.signals.get(&RC::from(ref_sig.clone())))
                .ok_or_else(|| js_err!("Could not find blacklisted signal '{ref_sig:?}'"))?;
            Ok(RC::clone(sig))
        }))
        .collect::<Result<_, JsError>>()?;

        data.electric_poles = (data_repr.electric_poles.into_iter().map(|mut pole| {
            pole.name = RC::clone(
                #[expect(clippy::unwrap_used, reason = "Garunteed to be populated")]
                uniq_strings.get(&pole.name).unwrap(),
            );

            for d in &mut pole.supply_area_distances {
                d.quality = RC::clone(
                    (signal_qualities.get(&d.quality))
                        .ok_or_else(|| js_err!("Could not find the quality '{}'", d.quality))?,
                );
            }
            Ok(pole)
        }))
        .collect::<Result<_, JsError>>()?;

        Ok(())
    })
}

#[inline]
pub fn with_prototype_data<R>(f: impl FnOnce(&PrototypeData) -> R) -> R {
    PROTOTYPE_DATA.with_borrow(f)
}

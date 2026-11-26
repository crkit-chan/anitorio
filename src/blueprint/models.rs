#![allow(dead_code, reason = "Satisfy Factorio Lua API")]
#![allow(clippy::struct_field_names, reason = "Factorio Lua API")]

use glam::DVec2;
use serde::{Deserialize, Serialize};
use tsify::Tsify;
use wasm_bindgen::prelude::*;

use super::dvec2_xy;
use crate::blueprint::data::{PrototypeData, with_prototype_data};
use crate::blueprint::prelude::*;
use crate::models::RawJsonString;
use crate::prelude::*;
use crate::util::{is_default, is_none_or_default};

pub struct ObjectGeneratorMetadata {
    pub wires:      Wires,
    pub next_ent_n: u32,
}

#[rustfmt::skip]
#[derive(Tsify, Default, Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ArithmeticCombinatorParameterOperation {
    #[default]
    #[serde(rename = "*"  )] Mul,
    #[serde(rename = "/"  )] Div,
    #[serde(rename = "+"  )] Add,
    #[serde(rename = "-"  )] Sub,
    #[serde(rename = "%"  )] Mod,
    #[serde(rename = "^"  )] Pow,
    #[serde(rename = "<<" )] Shl,
    #[serde(rename = ">>" )] Shr,
    #[serde(rename = "AND")] And,
    #[serde(rename = "OR" )] Or ,
    #[serde(rename = "XOR")] Xor,
}

#[derive(Tsify, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArithmeticCombinatorParameters {
    /// First signal to use in an operation. If not specified, the second
    /// argument will be the value of ``first_constant``.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_signal:           Option<RC<SignalID>>,
    /// Which circuit networks (red/green) to read ``first_signal`` from.
    /// Defaults to both.
    #[serde(default, skip_serializing_if = "is_default")]
    pub first_signal_networks:  CircuitNetworkSelection,
    /// When not specified, defaults to "*".
    #[serde(default, skip_serializing_if = "is_default")]
    pub operation:              ArithmeticCombinatorParameterOperation,
    /// Second signal to use in an operation. If not specified, the second
    /// argument will be the value of ``second_constant``.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub second_signal:          Option<RC<SignalID>>,
    /// Which circuit networks (red/green) to read ``second_signal`` from.
    /// Defaults to both.
    #[serde(default, skip_serializing_if = "is_default")]
    pub second_signal_networks: CircuitNetworkSelection,
    /// Constant to use as the second argument of the operation. Has no effect
    /// when ``second_signal`` is set. Defaults to 0.
    #[serde(default, skip_serializing_if = "is_default")]
    pub second_constant:        i32,
    /// Specifies the signal to output.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_signal:          Option<RC<SignalID>>,
}
impl ArithmeticCombinatorParameters {
    pub const fn swap_networks(&mut self) {
        self.first_signal_networks.swap();
        self.second_signal_networks.swap();
    }
}

#[derive(Tsify, Clone, Serialize, Deserialize, PartialEq)]
pub struct Blueprint {
    pub blueprint: BlueprintInner,
}
impl Blueprint {
    pub fn from_blueprint<T: AsRef<[u8]>>(src: T) -> Result<Self, JsError> {
        use base64::Engine as _;
        use base64::engine::general_purpose::STANDARD;

        // Remove the leading "0" prefix
        let b64 = (STANDARD.decode(&src.as_ref()[1..]))?;
        let zlib = flate2::read::ZlibDecoder::new(&*b64);
        Ok(Self::from_json_writer(zlib)?)
    }
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str::<Self>(json)
    }
    pub fn from_json_writer<R: std::io::Read + Sized>(
        reader: R,
    ) -> Result<Self, serde_json::Error> {
        serde_json::from_reader(reader)
    }
}

#[derive(Tsify, Clone, Serialize, Deserialize, PartialEq)]
#[expect(clippy::struct_field_names, reason = "Factorio Lua API")]
pub struct BlueprintBook {
    pub icons:          Option<Box<[Icon]>>,
    pub active_index:   u32,
    pub blueprint_book: BlueprintBookInner,
    pub item:           String,
    pub label:          Option<String>,
    pub version:        Option<u64>,
}

#[derive(Tsify, Clone, Serialize, Deserialize, PartialEq)]
pub struct BlueprintBookInner {
    pub blueprints: Box<[Blueprint]>,
}

macro_rules! blueprint_entity_property {
    ($name:ident, &str) => {
        paste::paste! {
            #[must_use]
            pub fn [<$name:snake>](&self) -> Option<&str> {
                for p in &self.properties {
                    if let BlueprintEntityProperty::$name(ref v) = *p {
                        return Some(v);
                    }
                }
                None
            }
            pub fn [<set_ $name:snake>](&mut self, v: &str) {
                for p in &mut self.properties {
                    if let BlueprintEntityProperty::$name(ref mut t) = *p {
                        v.clone_into(t);
                        return;
                    }
                }
                let mut properties = std::mem::take(&mut self.properties).into_vec();
                properties.push(BlueprintEntityProperty::$name(v.to_owned()));
                self.properties = properties.into();
            }
            #[inline]
            #[must_use = concat!("Use set_", stringify!([<$name:snake>]), "() instead if you don't use for chaining")]
            pub fn [<with_ $name:snake>](mut self, v: &str) -> Self {
                self.[<set_ $name:snake>](v);
                self
            }
        }
    };
    ($name:ident, $typ: ty) => {
        paste::paste! {
            #[must_use]
            pub fn [<$name:snake>](&self) -> Option<$typ> {
                for p in &self.properties {
                    if let BlueprintEntityProperty::$name(ref v) = *p {
                        return Some(v).cloned();
                    }
                }
                None
            }
            #[must_use]
            pub fn [<$name:snake _mut>](&mut self) -> Option<&mut $typ> {
                for p in &mut self.properties {
                    if let BlueprintEntityProperty::$name(ref mut v) = *p {
                        return Some(v);
                    }
                }
                None
            }
            pub fn [<set_ $name:snake>](&mut self, v: $typ) {
                for p in &mut self.properties {
                    if let BlueprintEntityProperty::$name(ref mut t) = *p {
                        *t = v;
                        return;
                    }
                }
                let mut properties = std::mem::take(&mut self.properties).into_vec();
                properties.push(BlueprintEntityProperty::$name(v.to_owned()));
                self.properties = properties.into();
            }
            #[inline]
            #[must_use = concat!("Use set_", stringify!([<$name:snake>]), "() instead if you don't use for chaining")]
            pub fn [<with_ $name:snake>](mut self, v: $typ) -> Self {
                self.[<set_ $name:snake>](v);
                self
            }
        }
    };
}

/// The representation of an entity inside of a blueprint.
#[derive(Tsify, Clone, PartialEq)]
pub struct BlueprintEntity {
    /// The entity's unique identifier in the blueprint.
    pub entity_number: u32,
    /// Reference to the prototype name + quality of the Entity
    signal_ref:        RC<SignalID>,
    /// The position of the entity.
    #[serde(with = "dvec2_xy")]
    pub position:      DVec2,
    pub properties:    Box<[BlueprintEntityProperty]>,
}
impl BlueprintEntity {
    /// Create a new entity with default None values for optional fields
    #[inline]
    #[must_use]
    #[expect(
        clippy::unwrap_used,
        reason = "Convenience function that should panic!"
    )]
    pub fn new(entity_number: u32, name: &str, position: DVec2) -> Self {
        let properties = boxed_slice![];
        let signal_ref = SignalID::intern("entity", name).unwrap();
        Self { entity_number, signal_ref, position, properties }
    }

    /// Create a new entity with default None values for optional fields
    #[inline]
    #[must_use]
    pub fn new_from_signal(
        entity_number: u32, signal: impl Into<RC<SignalID>>, position: DVec2,
    ) -> Self {
        let properties = boxed_slice![];
        let signal_ref = signal.into();
        Self { entity_number, signal_ref, position, properties }
    }

    blueprint_entity_property!(AlwaysOn, bool);
    blueprint_entity_property!(Color, Color);
    blueprint_entity_property!(ControlBehavior, ControlBehavior);
    blueprint_entity_property!(CustomTag, &str);
    blueprint_entity_property!(Direction, u8);
    blueprint_entity_property!(PlayerDescription, &str);

    #[must_use]
    #[inline]
    pub fn has_custom_tag(&self, v: &'static str) -> bool {
        (self.properties.iter())
            .any(|p| matches!(p, BlueprintEntityProperty::CustomTag(t) if *t == v))
    }

    #[must_use]
    #[inline]
    pub fn name(&self) -> RC<str> {
        RC::clone(&self.signal_ref.name)
    }
}
impl Serialize for BlueprintEntity {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap as _;

        let mut map = serializer.serialize_map(None)?;
        self.signal_ref.serialize_fields_into_map(&mut map, Some("entity"), false)?;

        map.serialize_entry("entity_number", &self.entity_number)?;
        map.serialize_entry("position", &self.position)?;

        for property in &self.properties {
            match *property {
                BlueprintEntityProperty::AlwaysOn(ref v) => map.serialize_entry("always_on", v),
                BlueprintEntityProperty::Color(ref v) => map.serialize_entry("color", v),
                BlueprintEntityProperty::ControlBehavior(ref v) => {
                    map.serialize_entry("control_behavior", v)
                },
                BlueprintEntityProperty::CustomTag(..) => Ok(()),
                BlueprintEntityProperty::Direction(ref v) => map.serialize_entry("direction", v),
                BlueprintEntityProperty::PlayerDescription(ref v) => {
                    map.serialize_entry("player_description", v)
                },
            }?;
        }

        map.end()
    }
}
impl<'de> Deserialize<'de> for BlueprintEntity {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Entity {
            entity_number:      u32,
            name:               String,
            #[serde(with = "dvec2_xy")]
            position:           DVec2,
            direction:          Option<u8>,
            control_behavior:   Option<ControlBehavior>,
            player_description: Option<String>,
            quality:            Option<String>,
            always_on:          Option<bool>,
            color:              Option<Color>,
        }

        let en = Entity::deserialize(deserializer)?;
        let mut properties = Vec::new();

        if let Some(v) = en.direction {
            properties.push(BlueprintEntityProperty::Direction(v));
        }
        if let Some(v) = en.control_behavior {
            properties.push(BlueprintEntityProperty::ControlBehavior(v));
        }
        if let Some(v) = en.player_description {
            properties.push(BlueprintEntityProperty::PlayerDescription(v));
        }
        if let Some(v) = en.always_on {
            properties.push(BlueprintEntityProperty::AlwaysOn(v));
        }
        if let Some(v) = en.color {
            properties.push(BlueprintEntityProperty::Color(v));
        }

        Ok(Self {
            signal_ref:    SignalID::intern_with_quality("entity", &en.name, en.quality.as_deref())
                .map_err(|e| serde::de::Error::custom(format!("Could not parse signal: {e:?}")))?,
            entity_number: en.entity_number,
            position:      en.position,
            properties:    properties.into_boxed_slice(),
        })
    }
}

#[derive(Tsify, Clone, Serialize, Deserialize, PartialEq)]
pub enum BlueprintEntityProperty {
    AlwaysOn(bool),
    Color(Color),
    ControlBehavior(ControlBehavior),
    #[serde(skip)]
    CustomTag(String),
    /// The direction the entity is facing. Only present for entities that can
    /// face in different directions and when the entity is not facing
    /// north.
    Direction(u8),
    PlayerDescription(String),
}

#[derive(Tsify, Clone, Serialize, Deserialize, PartialEq)]
pub struct BlueprintInner {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub icons:       Option<Box<[Icon]>>,
    #[serde(default, skip_serializing_if = "is_default")]
    pub entities:    Box<[BlueprintEntity]>,
    #[serde(default, skip_serializing_if = "is_none_or_default")]
    pub wires:       Option<Wires>,
    pub item:        String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label:       Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label_color: Option<Color>,
    pub version:     u64,
}

#[derive(Tsify, Clone, Deserialize, PartialEq, Eq)]
pub struct BlueprintLogisticFilter {
    pub index:      LogisticFilterIndex,
    #[serde(flatten)]
    signal_ref:     RC<SignalID>,
    /// The comparator for quality. nil if any quality.
    #[serde(default)]
    pub comparator: Option<ComparatorString>,
    #[serde(default)]
    pub count:      i32,
}
impl BlueprintLogisticFilter {
    #[inline]
    pub fn new<C: TryInto<ComparatorString>>(
        index: LogisticFilterIndex, signal_ref: &RC<SignalID>, comparator: C, count: i32,
    ) -> Result<Self, C::Error> {
        Ok(Self {
            index,
            signal_ref: RC::clone(signal_ref),
            comparator: Some(comparator.try_into()?),
            count,
        })
    }
}
impl Serialize for BlueprintLogisticFilter {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap as _;

        let mut map = serializer.serialize_map(None)?;
        self.signal_ref
            .serialize_fields_into_map(&mut map, Some(DEFAULT_SIGNAL_TYPE), true)?;

        if let &Some(comparator) = &self.comparator {
            map.serialize_entry("comparator", &comparator)?;
        }
        if self.count != 0 {
            map.serialize_entry("count", &self.count)?;
        }
        map.serialize_entry("index", &self.index)?;
        map.end()
    }
}

#[derive(Tsify, Clone, Serialize, Deserialize, PartialEq)]
pub struct Color {
    #[serde(default, skip_serializing_if = "is_default")]
    pub r: f64,
    #[serde(default, skip_serializing_if = "is_default")]
    pub g: f64,
    #[serde(default, skip_serializing_if = "is_default")]
    pub b: f64,
    #[serde(
        default = "Color::default_a",
        skip_serializing_if = "Color::is_default_a"
    )]
    pub a: f64,
}
impl Color {
    pub const fn default_a() -> f64 {
        1.0
    }
    pub const fn is_default_a(a: &f64) -> bool {
        (*a - const { Self::default_a() }).abs() < 0.0001
    }

    #[must_use]
    pub fn from_hex(hex: &str) -> Option<Self> {
        let hex = hex.strip_prefix('#').unwrap_or(hex);
        if hex.len() != 6 {
            return None;
        }
        let r = f64::from(u8::from_str_radix(&hex[0..2], 16).ok()?) / 255.0;
        let g = f64::from(u8::from_str_radix(&hex[2..4], 16).ok()?) / 255.0;
        let b = f64::from(u8::from_str_radix(&hex[4..6], 16).ok()?) / 255.0;

        Some(Self { r, g, b, a: 1.0 })
    }
}

/// A string that specifies how the inputs should be compared.
///
/// While the API accepts both versions for "less/greater than or equal to" and
/// "not equal", it'll always return "≥", "≤" or "≠" respectively when reading
/// them back.
#[derive(Tsify, Default, Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ComparatorString {
    /// "equal to"
    #[serde(rename = "=")]
    Eq,
    /// "greater than"
    #[serde(rename = ">")]
    Gt,
    /// "lesser than"
    #[default]
    #[serde(rename = "<")]
    Lt,
    /// "greater than or equal to"
    #[serde(rename = "\u{2265}", alias = ">=")] // ≥
    Ge,
    /// "lesser than or equal to"
    #[serde(rename = "\u{2264}", alias = "<=")] // ≤
    Le,
    /// "not equal to"
    #[serde(rename = "\u{2260}", alias = "!=")] // ≠
    Ne,
}
impl TryFrom<&str> for ComparatorString {
    type Error = JsError;
    fn try_from(other: &str) -> Result<Self, Self::Error> {
        Ok(match other.to_lowercase().as_str() {
            "=" | "==" | "eq" => Self::Eq,
            ">" | "gt" => Self::Gt,
            "<" | "lt" => Self::Lt,
            ">=" | "ge" | "gte" => Self::Ge,
            "<=" | "le" | "lte" => Self::Le,
            "!=" | "^" | "ne" | "neq" => Self::Ne,
            _ => err_js!("Attempted to parse unknown ComparatorString '{other}'"),
        })
    }
}

#[derive(Tsify, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum ControlBehavior {
    ArithmeticCombinator {
        arithmetic_conditions: ArithmeticCombinatorParameters,
    },
    ConstantCombinator {
        sections: LogisticSections,
    },
    DeciderCombinator {
        decider_conditions: DeciderCombinatorParameters,
    },
    Lamp {
        /// Defaults to false.
        #[serde(default, skip_serializing_if = "is_default")]
        use_colors:   bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        red_signal:   Option<RC<SignalID>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        green_signal: Option<RC<SignalID>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        blue_signal:  Option<RC<SignalID>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        rgb_signal:   Option<RC<SignalID>>,
        /// Defaults to ``defines.control_behavior.lamp.color_mapping``.
        color_mode:   i8,
    },
    #[serde(skip_deserializing)]
    DeciderCombinatorCustom {
        decider_conditions: DeciderCombinatorCustomParameters,
    },
}
impl ControlBehavior {
    #[inline]
    #[must_use]
    pub const fn arithmetic(arithmetic_conditions: ArithmeticCombinatorParameters) -> Self {
        Self::ArithmeticCombinator { arithmetic_conditions }
    }

    #[inline]
    #[must_use]
    pub const fn decider(decider_conditions: DeciderCombinatorParameters) -> Self {
        Self::DeciderCombinator { decider_conditions }
    }

    #[inline]
    pub fn maybe_swap_networks(&mut self) {
        match *self {
            // This actually swaps outputs + inputs, it's just named weirdly in Factorio
            Self::DeciderCombinator { decider_conditions: ref mut conds } => conds.swap_networks(),
            Self::ArithmeticCombinator { arithmetic_conditions: ref mut conds } => {
                conds.swap_networks();
            },
            _ => {},
        }
    }
}

#[derive(Tsify, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[tsify(type = "CircuitNetworkSelection")]
struct CircuitNetworkSelectionRepr {
    pub red:   bool,
    pub green: bool,
}

impl const From<CircuitNetworkSelectionRepr> for CircuitNetworkSelection {
    fn from(value: CircuitNetworkSelectionRepr) -> Self {
        let mut obj = Self::NONE;
        obj.set_green(value.green);
        obj.set_red(value.red);
        obj
    }
}
impl const From<CircuitNetworkSelection> for CircuitNetworkSelectionRepr {
    fn from(value: CircuitNetworkSelection) -> Self {
        Self { red: value.red(), green: value.green() }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CircuitNetworkSelection(u8);
impl const Default for CircuitNetworkSelection {
    fn default() -> Self {
        Self::BOTH
    }
}
impl Serialize for CircuitNetworkSelection {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        CircuitNetworkSelectionRepr::from(*self).serialize(serializer)
    }
}
impl<'de> Deserialize<'de> for CircuitNetworkSelection {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(CircuitNetworkSelectionRepr::deserialize(deserializer)?.into())
    }
}
impl const From<&str> for CircuitNetworkSelection {
    fn from(value: &str) -> Self {
        if value.eq_ignore_ascii_case("both") {
            Self::BOTH
        } else if value.eq_ignore_ascii_case("g") || value.eq_ignore_ascii_case("green") {
            Self::GREEN
        } else if value.eq_ignore_ascii_case("r") || value.eq_ignore_ascii_case("red") {
            Self::RED
        } else {
            Self::NONE
        }
    }
}
impl CircuitNetworkSelection {
    #[inline]
    #[must_use]
    pub const fn red(self) -> bool {
        self.0 & 1 != 0
    }
    #[inline]
    #[must_use]
    pub const fn green(self) -> bool {
        self.0 & 2 != 0
    }
    #[inline]
    pub const fn set_red(&mut self, value: bool) {
        self.0 = (self.0 & !1) | (value as u8);
    }
    #[inline]
    pub const fn set_green(&mut self, value: bool) {
        self.0 = (self.0 & !2) | ((value as u8) << 1);
    }

    pub const BOTH: Self = Self(Self::RED.0 | Self::GREEN.0);
    pub const GREEN: Self = Self(1 << 1);
    pub const NONE: Self = Self(0);
    pub const RED: Self = Self(1 << 0);

    #[inline]
    pub const fn swap(&mut self) {
        if self.red() != self.green() {
            self.0 ^= Self::BOTH.0;
        }
    }
}

#[derive(Tsify, Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CompareTypeString {
    #[serde(rename = "and")]
    And,
    #[serde(rename = "or")]
    Or,
}
impl const Default for CompareTypeString {
    fn default() -> Self {
        Self::Or
    }
}
impl const From<&str> for CompareTypeString {
    fn from(value: &str) -> Self {
        match value {
            "&" | "&&" => Self::And,
            x if x.eq_ignore_ascii_case("both") => Self::And,
            _ => Self::default(),
        }
    }
}

#[derive(Tsify, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeciderCombinatorCondition {
    /// Defaults to blank
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_signal:          Option<RC<SignalID>>,
    /// Which circuit networks (red/green) to read ``first_signal`` from.
    /// Defaults to both.
    #[serde(default, skip_serializing_if = "is_default")]
    pub first_signal_networks: CircuitNetworkSelection,
    /// Constant to compare ``first_signal`` to. Has no effect when
    /// ``second_signal`` is set. When neither ``second_signal`` nor
    /// constant are specified, the effect is as though constant were
    /// specified with the value 0.
    #[serde(default, skip_serializing_if = "is_default")]
    pub constant:              Option<i32>,
    /// Specifies how the inputs should be compared. If not specified, defaults
    /// to "<".
    #[serde(default, skip_serializing_if = "is_default")]
    pub comparator:            ComparatorString,
    /// Tells how this condition is compared with the preceding conditions in
    /// the corresponding conditions array. Defaults to "or".
    #[serde(default, skip_serializing_if = "is_default")]
    pub compare_type:          CompareTypeString,
}
impl DeciderCombinatorCondition {
    pub fn new(
        first_signal: impl Into<RC<SignalID>>, constant: i32,
        comparator: impl Into<ComparatorString>,
    ) -> Self {
        Self {
            first_signal:          Some(first_signal.into()),
            constant:              Some(constant),
            comparator:            comparator.into(),
            compare_type:          const { CompareTypeString::default() },
            first_signal_networks: CircuitNetworkSelection::BOTH,
        }
    }

    #[must_use]
    pub fn with_compare_type(mut self, compare_type: impl Into<CompareTypeString>) -> Self {
        self.compare_type = compare_type.into();
        self
    }

    #[must_use]
    pub fn with_first_signal_networks(
        mut self, networks: impl Into<CircuitNetworkSelection>,
    ) -> Self {
        self.first_signal_networks = networks.into();
        self
    }

    #[inline]
    pub const fn swap_networks(&mut self) {
        self.first_signal_networks.swap();
    }
}

#[derive(Tsify, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeciderCombinatorOutput {
    /// Specifies a signal to output.
    pub signal:                RC<SignalID>,
    /// Defaults to true. When false, will output the value from constant for
    /// the given ``output_signal``.
    #[serde(
        default = "DeciderCombinatorOutput::default_copy_count_from_input",
        skip_serializing_if = "DeciderCombinatorOutput::is_default_copy_count_from_input"
    )]
    pub copy_count_from_input: bool,
    /// The value to output when not copying input. Defaults to 1.
    #[serde(
        default = "DeciderCombinatorOutput::default_constant",
        skip_serializing_if = "DeciderCombinatorOutput::is_default_constant"
    )]
    pub constant:              i32,
    /// Sets which input network to read the value of signal from if
    /// ``copy_count_from_input`` is true. Defaults to both.
    #[serde(default, skip_serializing_if = "is_default")]
    pub networks:              CircuitNetworkSelection,
}
impl DeciderCombinatorOutput {
    pub fn new(signal: impl Into<RC<SignalID>>, constant: Option<i32>) -> Self {
        Self {
            copy_count_from_input: constant.is_none(),
            constant:              constant.unwrap_or(Self::default_constant()),
            signal:                signal.into(),
            networks:              CircuitNetworkSelection::BOTH,
        }
    }

    pub const fn default_constant() -> i32 {
        1
    }
    pub const fn default_copy_count_from_input() -> bool {
        true
    }

    #[expect(clippy::trivially_copy_pass_by_ref, reason = "Use with serde")]
    pub const fn is_default_constant(constant: &i32) -> bool {
        *constant == const { Self::default_constant() }
    }
    #[expect(clippy::trivially_copy_pass_by_ref, reason = "Use with serde")]
    pub const fn is_default_copy_count_from_input(copy_count_from_input: &bool) -> bool {
        *copy_count_from_input == const { Self::default_copy_count_from_input() }
    }

    #[must_use]
    pub fn with_networks(mut self, networks: impl Into<CircuitNetworkSelection>) -> Self {
        self.networks = networks.into();
        self
    }

    #[inline]
    pub const fn swap_networks(&mut self) {
        self.networks.swap();
    }
}

#[derive(Tsify, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeciderCombinatorParameters {
    /// List of conditions.
    #[serde(default, skip_serializing_if = "is_default")]
    pub conditions: Box<[DeciderCombinatorCondition]>,
    /// List of outputs.
    #[serde(default, skip_serializing_if = "is_default")]
    pub outputs:    Box<[DeciderCombinatorOutput]>,
}
impl DeciderCombinatorParameters {
    pub fn swap_networks(&mut self) {
        self.outputs.iter_mut().for_each(DeciderCombinatorOutput::swap_networks);
        self.conditions.iter_mut().for_each(DeciderCombinatorCondition::swap_networks);
    }
}

#[derive(Clone)]
pub struct DeciderCombinatorCustomParameters {
    pub conditions:  Box<[DeciderCombinatorCondition]>,
    pub indices:     Box<[(usize, i32)]>,
    pub signal_list: RC<[RawJsonString]>,
}
impl PartialEq for DeciderCombinatorCustomParameters {
    fn eq(&self, other: &Self) -> bool {
        self.conditions == other.conditions
            && self.indices == other.indices
            && (self.signal_list.iter())
                .zip(other.signal_list.iter())
                .all(|(a, b)| a.get() == b.get())
    }
}
impl Eq for DeciderCombinatorCustomParameters {}
impl Serialize for DeciderCombinatorCustomParameters {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct as _;

        let mut state = serializer.serialize_struct("DeciderCombinatorParameters", 2)?;
        if unlikely(self.conditions.is_empty()) {
            state.skip_field("conditions")
        } else {
            state.serialize_field("conditions", &self.conditions)
        }?;
        if unlikely(self.indices.is_empty()) {
            state.skip_field("outputs")
        } else {
            let outputs = IndexedDeciderCombinatorOutputs {
                indices:     &self.indices,
                signal_list: &self.signal_list,
            };
            state.serialize_field("outputs", &outputs)
        }?;
        state.end()
    }
}
struct IndexedDeciderCombinatorOutputs<'a> {
    indices:     &'a [(usize, i32)],
    signal_list: &'a [RawJsonString],
}
impl Serialize for IndexedDeciderCombinatorOutputs<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeSeq as _;

        let mut seq = serializer.serialize_seq(Some(self.indices.len()))?;
        for &(i, constant) in self.indices {
            seq.serialize_element(&IndexedDeciderCombinatorOutput {
                signal: &self.signal_list[i],
                copy_count_from_input: false,
                constant,
            })?;
        }
        seq.end()
    }
}
#[derive(Serialize)]
struct IndexedDeciderCombinatorOutput<'a> {
    signal:                &'a serde_json::value::RawValue,
    copy_count_from_input: bool,
    constant:              i32,
}

#[derive(Tsify, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Icon {
    pub signal: RC<SignalID>,
    pub index:  u32,
}

pub type LogisticFilterIndex = u16;

#[derive(Tsify, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LogisticSection {
    pub index:   u8,
    #[serde(default, skip_serializing_if = "is_default")]
    pub filters: Box<[BlueprintLogisticFilter]>,
}
#[derive(Tsify, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LogisticSections {
    #[serde(default, skip_serializing_if = "is_default")]
    pub sections: Box<[LogisticSection]>,
}

#[derive(Tsify, Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SignalID {
    /// The type of the signal. If the type is "item", this will be nil when
    /// reading.
    pub type_:   RC<str>,
    /// The prototype name of the signal's quality. Defaults to normal.
    pub quality: RC<str>,
    /// Name of the prototype.
    pub name:    RC<str>,
}
impl SignalID {
    #[inline]
    #[must_use]
    pub const fn new(type_: RC<str>, name: RC<str>, quality: RC<str>) -> Self {
        Self { type_, quality, name }
    }

    /// Creates a new ``SignalID`` without any references to static ``RC`` data.
    #[inline]
    #[must_use]
    pub fn raw(type_: &str, name: &str, quality: &str) -> Self {
        Self {
            type_:   RC::from(type_),
            quality: RC::from(quality),
            name:    RC::from(name),
        }
    }

    #[inline]
    pub fn intern(type_: &str, name: &str) -> Result<RC<Self>, JsError> {
        Self::new(type_.into(), name.into(), DEFAULT_QUALITY.into()).interned()
    }

    #[inline]
    pub fn intern_with_quality(
        type_: &str, name: &str, quality: Option<&str>,
    ) -> Result<RC<Self>, JsError> {
        let quality = quality.unwrap_or(DEFAULT_QUALITY);
        Self::new(type_.into(), name.into(), quality.into()).interned()
    }

    /// Creates data pointed to the static ``RC`` stored in signals on
    /// ``PrototypeData``.
    #[inline]
    pub fn interned(self) -> Result<RC<Self>, JsError> {
        with_prototype_data(|data| self.interned_on_prototype_data(data))
    }

    /// Creates data pointed to the static ``RC`` stored in signals on
    /// ``PrototypeData``.
    #[inline]
    pub fn with_quality(self, quality: Option<&str>) -> Result<RC<Self>, JsError> {
        let quality = quality.unwrap_or(DEFAULT_QUALITY);
        let sig = Self { quality: quality.into(), ..self };
        with_prototype_data(|data| sig.interned_on_prototype_data(data))
    }

    /// Creates data pointed to the static ``RC`` stored in signals on
    /// ``PrototypeData``.
    #[inline]
    pub fn interned_on_prototype_data(self, data: &PrototypeData) -> Result<RC<Self>, JsError> {
        let sig = data.signals.get(&self);
        let sig = sig.ok_or_else(|| js_err!("Could not find signal '{self:?}'"))?;
        Ok(RC::clone(sig))
    }

    #[inline]
    fn serialize_fields_into_map<S: serde::ser::SerializeMap>(
        &self, map: &mut S, default_type: Option<&str>, always_include_quality: bool,
    ) -> Result<(), S::Error> {
        if default_type.is_none_or(|t| t != self.type_.as_ref()) {
            map.serialize_entry("type", &self.type_)?;
        }
        if always_include_quality || self.quality.as_ref() != DEFAULT_QUALITY {
            map.serialize_entry("quality", &self.quality)?;
        }
        map.serialize_entry("name", &self.name)
    }

    #[inline]
    fn deserialize_fields<'de, D: serde::Deserializer<'de>>(
        deserializer: D, default_type: Option<&str>,
    ) -> Result<Self, D::Error> {
        let repr = SignalIDRepr::deserialize(deserializer).map_err(|e| {
            serde::de::Error::custom(format!("Could not deserialize SignalID: {e:?}"))
        })?;
        let type_ =
            (repr.type_.as_deref()).unwrap_or_else(|| default_type.unwrap_or(DEFAULT_SIGNAL_TYPE));
        let sig =
            Self::intern_with_quality(type_, &repr.name, repr.quality.as_deref()).map_err(|e| {
                serde::de::Error::custom(format!("Could not deserialize SignalID: {e:?}"))
            })?;
        Ok((*sig).clone())
    }
}
impl Serialize for SignalID {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap as _;

        let mut map = serializer.serialize_map(None)?;
        self.serialize_fields_into_map(&mut map, Some(DEFAULT_SIGNAL_TYPE), false)?;
        map.end()
    }
}
impl<'de> Deserialize<'de> for SignalID {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::deserialize_fields(deserializer, Some(DEFAULT_SIGNAL_TYPE))
    }
}
impl From<std::rc::Rc<Self>> for SignalID {
    fn from(val: std::rc::Rc<Self>) -> Self {
        std::rc::Rc::unwrap_or_clone(val)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SignalIDRepr {
    #[serde(default, rename = "type")]
    pub type_:   Option<String>,
    #[serde(default)]
    pub quality: Option<String>,
    pub name:    String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WireConnectorId(u8);
impl WireConnectorId {
    #[inline]
    #[must_use]
    pub const fn pack(a: u8, b: u8) -> u8 {
        debug_assert!(a >= 1 && a <= 9, "Wire connector ID out of bounds");
        debug_assert!(b >= 1 && b <= 9, "Wire connector ID out of bounds");
        (a << 4) | b
    }

    #[inline]
    #[must_use]
    const fn swap_raw(wire_id: u8) -> u8 {
        match wire_id {
            1 => 2,
            2 => 1,
            3 => 4,
            4 => 3,
            _ => wire_id,
        }
    }

    #[inline]
    #[must_use]
    pub const fn unpack(value: u8) -> (u8, u8) {
        (value >> 4, value & 0x0F)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Wires {
    entity_ids:  Vec<[u32; 2]>,
    should_swap: bool, /* We use a flag to determine if we should swap the wires when
                        * serializing / converting to iterator */
    wire_ids:    Vec<u8>,
}
impl Wires {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn extend<T: IntoIterator<Item = [u32; 4]>>(&mut self, iter: T) {
        iter.into_iter().for_each(|w| self.push(w));
    }
    pub fn extend_from_slice(&mut self, other: &[[u32; 4]]) {
        self.extend(other.iter().copied());
    }

    #[inline]
    pub fn push(&mut self, wire: [u32; 4]) {
        self.entity_ids.push([wire[0], wire[2]]);
        self.wire_ids.push(WireConnectorId::pack(wire[1] as u8, wire[3] as u8));
    }

    #[must_use]
    #[inline]
    pub fn len(&self) -> usize {
        debug_assert_eq!(
            self.entity_ids.len(),
            self.wire_ids.len(),
            "Lengths should match"
        );
        self.wire_ids.len()
    }

    #[inline]
    pub const fn swap_networks(&mut self) {
        self.should_swap ^= true;
    }

    #[inline]
    #[must_use]
    pub const fn unpack_item((ents, wires): ([u32; 2], u8), invert: bool) -> [u32; 4] {
        let (mut w1, mut w2) = WireConnectorId::unpack(wires);
        if invert {
            (w1, w2) = (WireConnectorId::swap_raw(w1), WireConnectorId::swap_raw(w2));
        }
        [ents[0], w1 as u32, ents[1], w2 as u32]
    }
}
impl IntoIterator for Wires {
    type Item = [u32; 4];
    type IntoIter = std::iter::Map<
        std::iter::Zip<std::vec::IntoIter<[u32; 2]>, std::vec::IntoIter<u8>>,
        fn(([u32; 2], u8)) -> [u32; 4],
    >;

    fn into_iter(self) -> Self::IntoIter {
        let iter = self.entity_ids.into_iter().zip(self.wire_ids);
        if self.should_swap {
            iter.map(|v| Self::unpack_item(v, true))
        } else {
            iter.map(|v| Self::unpack_item(v, false))
        }
    }
}
impl Serialize for Wires {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeSeq as _;

        let mut seq = serializer.serialize_seq(Some(self.entity_ids.len()))?;
        for entry in self.entity_ids.iter().copied().zip(self.wire_ids.iter().copied()) {
            seq.serialize_element(&Self::unpack_item(entry, self.should_swap))?;
        }
        seq.end()
    }
}
impl<'de> serde::Deserialize<'de> for Wires {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct WiresVisitor;
        impl<'de> serde::de::Visitor<'de> for WiresVisitor {
            type Value = Wires;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("a sequence of [u32; 4] wire entries")
            }

            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self, mut seq: A,
            ) -> Result<Self::Value, A::Error> {
                let mut wires = Wires::new();
                while let Some([e1, w1, e2, w2]) = seq.next_element::<[u32; 4]>()? {
                    wires.entity_ids.push([e1, e2]);
                    wires.wire_ids.push(WireConnectorId::pack(w1 as u8, w2 as u8));
                }
                Ok(wires)
            }
        }
        deserializer.deserialize_seq(WiresVisitor)
    }
}

#[wasm_bindgen(typescript_custom_section)]
const TS_TYPES: &str = r#"
export type LogisticFilterIndex = number;
export type Wire = [number, number, number, number];
"#;

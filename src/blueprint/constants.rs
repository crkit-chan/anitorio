use crate::blueprint::models::SignalID;
use glam::{DVec2, dvec2};
use std::sync::LazyLock;

/// Timer entity positions.
pub const TIMER1_POS: DVec2 = dvec2(-2.5, -0.5);
pub const TIMER2_POS: DVec2 = dvec2(-1.0, -0.5);
pub const TIMER3_POS: DVec2 = dvec2(-1.0, -1.5);
pub const TIMER4_POS: DVec2 = dvec2(-1.0, -2.5);
pub const TIMER5_POS: DVec2 = dvec2(-2.5, -3.0);
pub const TIMER6_POS: DVec2 = dvec2(-1.0, -3.5);

/// Direction constants.
pub const DIR_U: u8 = 0;
pub const DIR_R: u8 = 4;
pub const DIR_D: u8 = 8;
pub const DIR_L: u8 = 12;

pub static SIG_EACH: LazyLock<SignalID> = LazyLock::new(|| SignalID::new("virtual", EACH));

pub static ENT_ARI_CB: LazyLock<SignalID> = LazyLock::new(|| SignalID::new("entity", ARI_CB));
pub static ENT_CONSTANT_CB: LazyLock<SignalID> =
    LazyLock::new(|| SignalID::new("entity", CONSTANT_CB));
pub static ENT_DEC_CB: LazyLock<SignalID> = LazyLock::new(|| SignalID::new("entity", DEC_CB));
pub static ENT_LAMP: LazyLock<SignalID> = LazyLock::new(|| SignalID::new("entity", LAMP));

/// Blueprint version constant.
pub const ARI_CB: &str = "arithmetic-combinator";
pub const CONSTANT_CB: &str = "constant-combinator";
pub const DEC_CB: &str = "decider-combinator";
pub const EACH: &str = "signal-each";
pub const LAMP: &str = "small-lamp";

pub const BLUEPRINT_VERSION: u64 = 562_949_955_518_464;
pub const BLUEPRINT: &str = "blueprint";
pub const DEFAULT_POWER_POLE_NAME: &str = "substation";
pub const DEFAULT_QUAL_KEY: &str = "<default>";
pub const DEFAULT_QUALITY: &str = "normal";
pub const DEFAULT_SIGNAL_TYPE: &str = "item";

pub const WIRE_R: u32 = 1;
pub const WIRE_G: u32 = 2;
pub const WIRE_OUT_R: u32 = 3;
pub const WIRE_OUT_G: u32 = 4;
pub const WIRE_C: u32 = 5;

use crate::blueprint::models::SignalID;
use crate::macros::lazy_arc_const;
use glam::{DVec2, dvec2};
use std::sync::Arc;

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

/// Blueprint version constant.
pub const BLUEPRINT_VERSION: u64 = 562_949_955_518_464;

lazy_arc_const!(pub ARI_CB             : str = "arithmetic-combinator");
lazy_arc_const!(pub BLUEPRINT          : str = "blueprint"            );
lazy_arc_const!(pub CONSTANT_COMB      : str = "constant-combinator"  );
lazy_arc_const!(pub DEC_CB             : str = "decider-combinator"   );
lazy_arc_const!(pub DEFAULT_QUAL_KEY   : str = "<default>"            );
lazy_arc_const!(pub DEFAULT_SIGNAL_TYPE: str = "item"                 );
lazy_arc_const!(pub DEFAULT_QUALITY    : str = "normal"               );
lazy_arc_const!(pub LAMP               : str = "small-lamp"           );

lazy_arc_const!(pub SIG_EACH: SignalID = SignalID {
    name: Arc::from("signal-each"),
    quality: None,
    type_: Arc::from("virtual"),
});

pub const DEFAULT_POWER_POLE_NAME: &str = "substation";

pub const WIRE_R: u32 = 1;
pub const WIRE_G: u32 = 2;
pub const WIRE_OUT_R: u32 = 3;
pub const WIRE_OUT_G: u32 = 4;
pub const WIRE_C: u32 = 5;

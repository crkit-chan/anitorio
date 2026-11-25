#![allow(clippy::unwrap_used, reason = "Utility")]

use crate::{blueprint::models::SignalID, prelude::*};
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

lazy_const!(pub ENT_ARI_CB:      Arc<SignalID> = Arc::new(SignalID::raw("entity", ARI_CB     , DEFAULT_QUALITY)));
lazy_const!(pub ENT_CONSTANT_CB: Arc<SignalID> = Arc::new(SignalID::raw("entity", CONSTANT_CB, DEFAULT_QUALITY)));
lazy_const!(pub ENT_DEC_CB:      Arc<SignalID> = Arc::new(SignalID::raw("entity", DEC_CB     , DEFAULT_QUALITY)));
lazy_const!(pub ENT_LAMP:        Arc<SignalID> = Arc::new(SignalID::raw("entity", LAMP       , DEFAULT_QUALITY)));

lazy_const!(pub VIRTUAL_EACH: Arc<SignalID> = Arc::new(SignalID::raw("virtual", EACH, DEFAULT_QUALITY)));

/// Blueprint version constant.
pub const ARI_CB: &str = "arithmetic-combinator";
pub const CONSTANT_CB: &str = "constant-combinator";
pub const DEC_CB: &str = "decider-combinator";
pub const EACH: &str = "signal-each";
pub const LAMP: &str = "small-lamp";

pub const BLUEPRINT_VERSION: u64 = 562_949_955_518_464;
pub const BLUEPRINT: &str = "blueprint";
pub const DEFAULT_POWER_POLE_NAME: &str = "substation";
pub const DEFAULT_QUALITY: &str = "normal";
pub const DEFAULT_SIGNAL_TYPE: &str = "item";

pub const WIRE_R: u32 = 1;
pub const WIRE_G: u32 = 2;
pub const WIRE_OUT_R: u32 = 3;
pub const WIRE_OUT_G: u32 = 4;
pub const WIRE_C: u32 = 5;

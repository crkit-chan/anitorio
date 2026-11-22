use super::prelude::*;
use crate::{arithmetic_conds, boxed_slice, decider_cond, prelude::*};
use std::sync::Arc;

/// Generates timer entities and wires for the blueprint's timing mechanism.
///
/// This is only run once for the entire blueprint.
///
/// # Arguments
///
/// * `stop` - Maximum tick value before reset.
/// * `ticks_per_frame` - Ticks per frame (derived from FPS).
/// * `frames_per_combinator` - Number of frames processed per combinator (derived from
///   compression).
/// * `args` - Uses ``BlueprintArgs.grayscale_bits`` (affects extra combinators).
///
/// # Returns
///
/// A tuple with entities, wires, and the timer reset entity (which is not included in entities)
pub fn generate_timer(
    stop: usize,
    ticks_per_frame: u8,
    frames_per_comb: u32,
    args: &BlueprintArgs,
) -> Result<(Vec<BlueprintEntity>, Wires), JsError> {
    let (sig_f, sig_p, sig_s, sig_t) = {
        let s = &args.timing_signals;
        (&s.f, &s.p, &s.s, &s.t)
    };

    let bits = args.image_args.bits();
    let mut ents: Vec<BlueprintEntity> = Vec::new();
    let mut wires = Wires::new();

    {
        let mut en = BlueprintEntity::new(1, &CONSTANT_COMB, TIMER1_POS).with_direction(DIR_R);
        let sections = LogisticSections {
            sections: boxed_slice![LogisticSection {
                index:   1,
                filters: boxed_slice![BlueprintLogisticFilter {
                    index:      1,
                    type_:      Arc::clone(&sig_t.type_),
                    name:       Arc::clone(&sig_t.name),
                    quality:    sig_t.quality.clone(),
                    comparator: Some(ComparatorString::Eq),
                    count:      1,
                }],
            }],
        };
        en.set_control_behavior(ControlBehavior::ConstantCombinator { sections });
        ents.push(en.with_custom_tag("incrementer"));
    }
    {
        let mut en = BlueprintEntity::new(2, &DEC_CB, TIMER2_POS).with_direction(DIR_R);
        let condition = decider_cond!(sig_t, <, i32::try_from(stop)?);
        let decider_conditions = DeciderCombinatorParameters {
            conditions: boxed_slice![condition],
            outputs:    boxed_slice![DeciderCombinatorOutput::new(sig_t, None)],
        };
        if args.detailed_descriptions {
            en.set_player_description("Increments 60 times per second up to the GIF's frame count, then resets to 0. It determines the current frame.");
        }
        en.set_control_behavior(ControlBehavior::DeciderCombinator { decider_conditions });
        ents.push(en.with_custom_tag("base_timer"));
    }
    {
        let en = BlueprintEntity::new(3, &ARI_CB, TIMER3_POS).with_direction(DIR_R);
        let conds = arithmetic_conds!(sig_t - 1 => sig_p);
        ents.push(en.with_control_behavior(ControlBehavior::arithmetic(conds)));
    }

    wires.push([1, WIRE_R, 2, WIRE_R]);
    wires.push([2, WIRE_R, 2, WIRE_OUT_R]);
    wires.push([2, WIRE_R, 3, WIRE_R]);

    if bits > 0 {
        {
            let en = BlueprintEntity::new(4, &ARI_CB, TIMER4_POS).with_direction(DIR_L);
            let conds = arithmetic_conds!(sig_t % (u32::from(ticks_per_frame) * frames_per_comb).cast_signed() => sig_s);
            ents.push(en.with_control_behavior(ControlBehavior::arithmetic(conds)));
        }
        {
            let en = BlueprintEntity::new(5, &ARI_CB, TIMER5_POS).with_direction(DIR_U);
            let conds =
                arithmetic_conds!(sig_s / u32::from(ticks_per_frame).cast_signed() => sig_f);
            ents.push(en.with_control_behavior(ControlBehavior::arithmetic(conds)));
        }
        {
            let mut en = BlueprintEntity::new(6, &ARI_CB, TIMER6_POS).with_direction(DIR_R);
            let conds = arithmetic_conds!(SIG_EACH * bits.cast_signed() => SIG_EACH);
            if args.detailed_descriptions {
                en.set_player_description("Calculates the bit shift for current frame.");
            }
            ents.push(en.with_control_behavior(ControlBehavior::arithmetic(conds)));
        }

        wires.push([3, WIRE_R, 4, WIRE_R]);
        wires.push([4, WIRE_OUT_R, 5, WIRE_R]);
        wires.push([5, WIRE_OUT_R, 6, WIRE_R]);
        wires.push([6, WIRE_OUT_R, 3, WIRE_OUT_R]);
    }

    Ok((ents, wires))
}

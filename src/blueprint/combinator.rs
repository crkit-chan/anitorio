use super::{power_pole::PowerPoleOccupied, prelude::*};
use crate::prelude::*;
use glam::{DVec2, IVec2, dvec2};
use std::{cell::RefCell, collections::BTreeMap, sync::Arc};

thread_local! {
    static COMB_POS_DATA: RefCell<Option<Box<[CombinatorPositionData]>>> = const { RefCell::new(None) };
}

#[derive(serde::Serialize, Clone)]
struct CombinatorPositionData {
    compression:    Option<SignalCompression>,
    dim:            DVec2,
    grayscale_bits: Vec<u32>,
    blueprint:      Blueprint,
}

pub fn load_comb_pos_json(data_json_reader: impl std::io::Read) -> Result<(), JsError> {
    #[derive(serde::Deserialize)]
    struct CombinatorPositionJson {
        compression:    Option<SignalCompression>,
        #[serde(rename = "grayscaleBits")]
        grayscale_bits: OneOrMany<u32>,
        blueprints:     OneOrMany<PositionEntry>,
    }
    #[derive(Clone, serde::Deserialize)]
    struct PositionEntry {
        blueprint: String,
    }

    fn get_bounding_box(en: &BlueprintEntity) -> Result<core::ops::Range<DVec2>, JsError> {
        let offset = DVec2::from(match en.name.as_ref() {
            name if name == &**constants::ARI_CB || name == &**constants::DEC_CB => {
                match en.direction.unwrap_or(0) {
                    constants::DIR_U | constants::DIR_D => (1.0, 2.0),
                    constants::DIR_L | constants::DIR_R => (2.0, 1.0),
                    _ => return Err(JsError::new("Unsupported entity direction")),
                }
            }
            name if name == &**constants::CONSTANT_COMB => (1.0, 1.0),
            _ => return Err(JsError::new("Unsupported entity type")),
        });
        Ok((en.position - offset / 2.0)..(en.position + offset / 2.0))
    }

    let parsed: Box<[CombinatorPositionJson]> = serde_json::from_reader(data_json_reader)?;

    let data_iter = parsed.iter().flat_map(|cb| {
        cb.blueprints.to_vec().into_iter().map(move |entry| {
            let mut blueprint = Blueprint::from_blueprint(entry.blueprint)?;
            let mut bounds = DVec2::INFINITY..DVec2::NEG_INFINITY;

            for en in &blueprint.blueprint.entities {
                let en_bounds = get_bounding_box(en)?;
                bounds.start = bounds.start.min(en_bounds.start);
                bounds.end = bounds.end.max(en_bounds.end);
            }
            let offset = -bounds.start;

            for en in &mut blueprint.blueprint.entities {
                en.position += offset;
            }

            Ok(CombinatorPositionData {
                compression: cb.compression,
                dim: bounds.end - bounds.start,
                grayscale_bits: cb.grayscale_bits.to_vec(),
                blueprint,
            })
        })
    });

    COMB_POS_DATA.replace(Some(data_iter.collect::<Result<Box<_>, JsError>>()?));
    Ok(())
}

pub struct GenerateCombinatorsOutput {
    pub data_cbs:   LazyCombinators,
    pub other_ents: Vec<BlueprintEntity>,
    pub metadata:   ObjectGeneratorMetadata,

    pub in_ent_n:  u32,
    pub out_ent_n: u32,
}

/// Generates combinator entities and wiring for each frame group.
///
/// # Arguments
///
/// * `ticks_per_group` - Ticks per group.
/// * `occupied` - Tracks cells occupied by power poles.
/// * `base_ent_n` - Starting entity number.
/// * `start_pos` - Starting X and Y coordinates.
/// * `max_cols_per_grp` - Maximum columns per group. Used to detect if compact layout should be
///   used.
/// * `args` Blueprint arguments.
///
/// # Returns
///
/// ``ObjectGeneratorOutput`` (containing data comb entities), and (``other_entities``, first
/// connection entity, target output entity).
///
/// Does not populate combinators with data.
pub fn generate_combinators(
    ticks_per_group: u32,
    occupied: &PowerPoleOccupied,
    base_ent_n: u32,
    start_pos: DVec2,
    max_cols_per_grp: u32,
    args: &BlueprintArgs,
) -> GenerateCombinatorsOutput {
    // Positions for combinators, for different widths.
    // TODO: Don't recalculate this for every group
    let mut best_pos_data: Option<CombinatorPositionData> = None;
    let mut pos_dict: BTreeMap<String, &BlueprintEntity> = BTreeMap::new();
    let mut base_pos = start_pos;
    let use_compact_layout = max_cols_per_grp < 2;
    let use_delta_comp = args.signal_compression == Some(SignalCompression::Delta);

    COMB_POS_DATA.with_borrow(|data| {
        for d in data.as_ref().expect("No combinator positions data") {
            if d.dim.x <= f64::from(max_cols_per_grp)
                && use_delta_comp == matches!(d.compression, Some(SignalCompression::Delta))
                && d.grayscale_bits.contains(&args.image_args.grayscale_bits)
                && d.dim.x > best_pos_data.as_ref().map_or(0.0, |v| v.dim.x)
            {
                best_pos_data = Some(d.clone());
            }
        }
    });

    if let Some(d) = best_pos_data.as_ref() {
        for en in &d.blueprint.blueprint.entities {
            if let Some(tag) = en.player_description.as_ref() {
                pos_dict.insert(tag.clone(), en);
            }
        }
    }
    let mut first_data_cb_ent_n: Option<u32> = None;
    let mut first_connection_ent_n: Option<u32> = None;
    let mut curr_ent_n = base_ent_n;
    let mut other_ents = Vec::new(); // shifters / etc
    // let mut data_cbs = Vec::new(); // data entities
    let mut wires = Vec::new();

    let use_const_cbs = matches!(args.image_args.mode, Mode::Static { const_cb: true, .. });
    let gray_bits = args.image_args.grayscale_bits;
    let height = best_pos_data.as_ref().map_or(0.0, |v| v.dim.y);
    let base_offset = base_pos; // Add a constant offset on dimensions

    base_pos.y -= height; // Set offset for combinators

    // Finds the position for the combinator with the given tag, based off the blueprint string we
    // decoded earlier
    let get_pos = |tag: &str| -> DVec2 {
        let entry = pos_dict.get(tag).unwrap_or_else(|| panic!("No '{tag}'!"));
        let mut pos = entry.position + base_offset;
        pos.y -= height;
        pos
    };
    let get_dir = |tag: &str| -> u32 { pos_dict.get(tag).unwrap().direction.unwrap_or(0) };

    // Note: signal compression is auto-disabled if generating a static image
    if use_delta_comp {
        let mut en = BlueprintEntity::new(curr_ent_n, Arc::clone(&DEC_CB), get_pos("delay"));
        let cb_output = DeciderCombinatorOutput::new(Arc::clone(&SIG_EACH), None);
        let cond = DeciderCombinatorCondition::new(SIG_EACH.clone(), 0, CompStr::Ne);
        let dc = DeciderCombinatorParameters {
            conditions: vec![cond],
            outputs:    vec![cb_output],
        };
        en = en.with_tag("delay comb").with_direction(get_dir("delay"));
        en = en.with_control_behavior(ControlBehavior::decider(dc));
        other_ents.push(en.with_description("Utility 1-tick delay combinator."));
        first_connection_ent_n = Some(curr_ent_n);
        curr_ent_n += 1;

        let desc = "Memory cell for delta compression. This holds all the signal values, then allows us to set any signal value by applying a delta. We can reach any number in one tick using overflow logic.";
        let mut en = BlueprintEntity::new(curr_ent_n, Arc::clone(&DEC_CB), get_pos("memory"));
        let cb_output = DeciderCombinatorOutput::new(Arc::clone(&SIG_EACH), None);
        let dc = DeciderCombinatorParameters {
            conditions: vec![
                DeciderCombinatorCondition::new(SIG_EACH.clone(), 0, CompStr::Ne)
                    .with_first_signal_networks(CircuitNetworkSelection::green()),
                DeciderCombinatorCondition::new(args.timing_signals.p.clone(), 0, CompStr::Ne)
                    .with_compare_type(CompareTypeString::And)
                    .with_first_signal_networks(CircuitNetworkSelection::red()),
            ],
            outputs:    vec![cb_output.with_networks(CircuitNetworkSelection::green())],
        };
        en = en.with_direction(get_dir("memory"));
        en = en.with_control_behavior(ControlBehavior::decider(dc));
        other_ents.push(en.with_tag("memory comb").with_description(desc));

        // Self connection for memory
        wires.push([curr_ent_n, WIRE_G, curr_ent_n, WIRE_OUT_G]);
        wires.push(mkwires!(R other_ents; OUT "delay comb" => IN "memory comb"));
        curr_ent_n += 1;
    }

    if gray_bits > 0 {
        let mut en = BlueprintEntity::new(curr_ent_n, Arc::clone(&ARI_CB), get_pos(">>"));
        let desc = "Shifts the input numbers until they are in the range of the current frame.";
        let ac = ArithmeticCombinatorParameters {
            first_signal:    Some(SIG_EACH.clone()),
            second_signal:   Some(args.timing_signals.f.clone()),
            second_constant: 0,
            operation:       ArithmeticCombinatorParameterOperation::Shr,
            output_signal:   Some(SIG_EACH.clone()),
        };
        en = en.with_tag(">> comb").with_direction(get_dir(">>"));
        en = en.with_control_behavior(ControlBehavior::arithmetic(ac));
        other_ents.push(en.with_description(desc));

        if first_connection_ent_n.is_none() {
            first_connection_ent_n = Some(curr_ent_n);
        }
        curr_ent_n += 1;

        let mut en = BlueprintEntity::new(curr_ent_n, Arc::clone(&ARI_CB), get_pos("AND"));
        let ac = arithmetic_conds!(SIG_EACH AND match gray_bits { 1 => 1, 4 => 15, _ => 255 } => SIG_EACH);
        en = en.with_tag("AND comb").with_direction(get_dir("AND")).with_description("Filters out the bits that are not relevant for the current frame, after bit-shifting. (The value of each signal encodes multiple frames)");
        other_ents.push(en.with_control_behavior(ControlBehavior::arithmetic(ac)));
        wires.push(mkwires!(R other_ents; OUT ">> comb" => IN "AND comb"));

        if use_delta_comp {
            wires.push(mkwires!(G other_ents; OUT "memory comb" => IN ">> comb"));
            wires.push(mkwires!(R other_ents; OUT "delay comb" => IN ">> comb"));
        }
        curr_ent_n += 1;

        if gray_bits == 1 || gray_bits == 4 {
            let mut en = BlueprintEntity::new(curr_ent_n, Arc::clone(&ARI_CB), get_pos("*"));
            let conds =
                arithmetic_conds!(SIG_EACH * if gray_bits == 1 { 255 } else { 17 } => SIG_EACH);
            en = en.with_tag("* comb").with_direction(get_dir("*"));
            other_ents.push(en.with_control_behavior(ControlBehavior::arithmetic(conds)));
            wires.push(mkwires!(R other_ents; OUT "AND comb" => IN "* comb"));
            curr_ent_n += 1;
        }
    } else if first_connection_ent_n.is_none() {
        first_connection_ent_n = Some(curr_ent_n);
    }

    let in_ent_n = first_connection_ent_n.unwrap();
    let out_ent_n = if gray_bits > 0 {
        (curr_ent_n - 1).max(base_ent_n)
    } else {
        entity_idx_by_tag!(other_ents, "memory comb")
            .unwrap_or_else(|| (curr_ent_n - 1).max(base_ent_n))
    };
    let cb_midpoint = if use_const_cbs {
        const { dvec2(0.5, -0.5) }
    } else if use_compact_layout {
        const { dvec2(0.5, -1.0) } // Base position for vertical layout
    } else {
        const { dvec2(1.0, -0.5) } // Base position for horizontal layout
    };
    let mut offset: DVec2;
    let mut curr_pos: DVec2;
    let mut grid_pos = IVec2::ZERO; // Current grid position (not absolute, multiplied by combinator's bounding box)
    let mut prev_ents_n: Vec<Option<u32>> = vec![None; max_cols_per_grp as usize];
    let mut prev_prev_ents_n: Vec<Option<u32>> = vec![None; max_cols_per_grp as usize];

    // Calculate the offsets to test for collisions. In Factorio, all collision boxes are
    // rectangular.
    let size = cb_midpoint.abs() * 2.0;
    let x_offsets = (0..size.x as usize).map(|v| v as f64 + 0.5 - size.x / 2.0);
    let y_offsets = (0..size.y as usize).map(|v| v as f64 + 0.5 - size.y / 2.0);
    let y_offsets = y_offsets.collect::<Vec<_>>();
    let hitbox_offsets = x_offsets.flat_map(|x| y_offsets.iter().map(move |&y| dvec2(x, y)));
    let hitbox_offsets = hitbox_offsets.collect::<Vec<_>>();
    let target_name = Arc::clone(if use_const_cbs {
        &constants::CONSTANT_COMB
    } else {
        &DEC_CB
    });
    let mut positions = Vec::<(u32, DVec2)>::new();

    // Generates combinators up to down, then left to right.
    for tick_i in 0..ticks_per_group as usize {
        loop {
            offset = cb_midpoint * (1 + 2 * grid_pos).as_dvec2(); // Set target midpoint
            curr_pos = base_pos + offset;

            if offset.x + cb_midpoint.x > f64::from(max_cols_per_grp) {
                grid_pos.x = 0;
                grid_pos.y += 1;
            } else if (hitbox_offsets.iter()).any(|o| !occupied.test(curr_pos + *o)) {
                grid_pos.x += 1; // Note we test if powerage is needed, don't necessarily request it
            } else {
                break;
            }
        }

        if tick_i == 0 {
            first_data_cb_ent_n = Some(curr_ent_n);

            if use_delta_comp {
                let ent_n = entity_idx_by_tag!(other_ents, "memory comb").expect("No memory comb!");
                wires.push([curr_ent_n, WIRE_OUT_G, ent_n, WIRE_G]);
            } else if gray_bits > 0 {
                let ent_n = entity_idx_by_tag!(other_ents, ">> comb").expect("No >> comb!");
                wires.push([curr_ent_n, WIRE_OUT_G, ent_n, WIRE_G]);
            }
        }

        positions.push((curr_ent_n, curr_pos));

        let x_i = grid_pos.x as usize;
        let prefer_horiz = false;
        let mut cands: Vec<usize> = Vec::new();

        if grid_pos.y > 0 && (!prefer_horiz || grid_pos.x == 0) {
            cands.push(x_i); // Vertical connection
        }
        if grid_pos.x > 0
            && (prefer_horiz || prev_ents_n[x_i].is_none() || prev_prev_ents_n[x_i - 1].is_none())
        {
            cands.push(x_i - 1); // Horizontal connection
        }
        for i in cands {
            if let Some(prev) = prev_ents_n[i] {
                wires.push([prev, WIRE_R, curr_ent_n, WIRE_R]); // Timing signals wire
                wires.push([prev, WIRE_OUT_G, curr_ent_n, WIRE_OUT_G]); // Lamp data wire
            }
        }

        prev_prev_ents_n[x_i] = prev_ents_n[x_i];
        prev_ents_n[x_i] = Some(curr_ent_n);
        curr_ent_n += 1;
        grid_pos.x += 1;
    }
    wires.push([
        in_ent_n,
        WIRE_R,
        first_data_cb_ent_n.expect("No first data combinator"),
        WIRE_R,
    ]);

    GenerateCombinatorsOutput {
        data_cbs: LazyCombinators {
            positions: positions.into_boxed_slice(),
            target_name,
            direction: if use_const_cbs {
                constants::DIR_D
            } else if use_compact_layout {
                constants::DIR_U
            } else {
                constants::DIR_R
            },
        },
        other_ents,
        metadata: ObjectGeneratorMetadata { wires, next_ent_n: curr_ent_n },
        in_ent_n,
        out_ent_n,
    }
}

pub struct LazyCombinators {
    positions:   Box<[(u32, DVec2)]>,
    target_name: Arc<str>,
    direction:   u32,
}
impl LazyCombinators {
    #[expect(clippy::unnecessary_wraps, reason = "Matches API of Vec<T>::get")]
    pub fn get(&self, index: usize) -> Option<BlueprintEntity> {
        let (curr_ent_n, pos) = self.positions[index];
        let mut entity = BlueprintEntity::new(curr_ent_n, Arc::clone(&self.target_name), pos);
        if index == 0 {
            entity = entity.with_tag("first data comb");
        }
        Some(entity.with_direction(self.direction))
    }
}

use super::{power_pole::PowerPoleOccupied, prelude::*};
use crate::image_processing::rgb_to_int;
use crate::{arithmetic_conds, decider_cond, entity_idx_by_tag, mkwires};
use crate::{boxed_slice, prelude::*};
use glam::{DVec2, IVec2, dvec2};
use std::{cell::RefCell, collections::BTreeMap, sync::Arc};

thread_local! {
    static COMB_POS_DATA: RefCell<Option<Box<[CombinatorPositionData]>>> = const { RefCell::new(None) };
}

#[derive(serde::Serialize, Clone)]
struct CombinatorPositionData {
    compression:    Option<SignalCompression>,
    dim:            DVec2,
    grayscale_bits: Box<[u32]>,
    blueprint:      Blueprint,
}

pub fn load_comb_pos_json(data_json_reader: impl std::io::Read) -> Result<(), JsError> {
    #[derive(serde::Deserialize)]
    struct CombinatorPositionJson {
        compression: Option<SignalCompression>,
        bits:        OneOrMany<u32>,
        blueprints:  OneOrMany<PositionEntry>,
    }
    #[derive(Clone, serde::Deserialize)]
    struct PositionEntry {
        blueprint: String,
    }

    fn get_bounding_box(en: &BlueprintEntity) -> std::ops::Range<DVec2> {
        let offset = DVec2::from(match en.name.as_ref() {
            name if name == &**constants::ARI_CB || name == &**constants::DEC_CB => {
                match en.direction.unwrap_or(0) {
                    constants::DIR_U | constants::DIR_D => (1.0, 2.0),
                    constants::DIR_L | constants::DIR_R => (2.0, 1.0),
                    _ => panic!("Unsupported entity direction"),
                }
            }
            name if name == &**constants::CONSTANT_COMB => (1.0, 1.0),
            _ => panic!("Unsupported entity type"),
        });
        (en.position - offset / 2.0)..(en.position + offset / 2.0)
    }

    let parsed: Box<[CombinatorPositionJson]> = serde_json::from_reader(data_json_reader)?;
    let data_iter = parsed.into_iter().flat_map(|cb| {
        cb.blueprints.into_vec().into_iter().map(move |entry| {
            let mut blueprint = Blueprint::from_blueprint(entry.blueprint)?;
            let mut bounds = DVec2::INFINITY..DVec2::NEG_INFINITY;

            // Extend bounds to fit all entities
            for en in &blueprint.blueprint.entities {
                let en_bounds = get_bounding_box(en);
                bounds.start = bounds.start.min(en_bounds.start);
                bounds.end = bounds.end.max(en_bounds.end);
            }
            for en in &mut blueprint.blueprint.entities {
                en.position -= bounds.start;
            }

            Ok(CombinatorPositionData {
                compression: cb.compression,
                dim: bounds.end - bounds.start,
                grayscale_bits: cb.bits.to_vec().into_boxed_slice(),
                blueprint,
            })
        })
    });

    COMB_POS_DATA.replace(Some(data_iter.collect::<Result<_, JsError>>()?));
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
    base_ent_n: u32,
    start_pos: DVec2,
    max_cols_per_grp: u32,
    args: &BlueprintArgs,
    occupied: &PowerPoleOccupied,
    palette: Option<&ColorPalette>,
) -> GenerateCombinatorsOutput {
    // Positions for combinators, for different widths.
    // TODO: Don't recalculate this for every group
    let mut best_pos_data: Option<CombinatorPositionData> = None;
    let mut pos_dict = BTreeMap::<&str, &BlueprintEntity>::new();
    let use_compact_layout = max_cols_per_grp < 2;
    let use_delta_comp = args.signal_compression == Some(SignalCompression::Delta);
    let bits = args.image_args.bits();

    // Find the most suitable combinator position data
    COMB_POS_DATA.with_borrow(|data| {
        let mut best_data: Option<&CombinatorPositionData> = None;
        for d in data.as_ref().expect("No combinator positions data") {
            if d.dim.x <= f64::from(max_cols_per_grp)
                && use_delta_comp == matches!(d.compression, Some(SignalCompression::Delta))
                && d.grayscale_bits.contains(&bits)
                && d.dim.x > best_pos_data.as_ref().map_or(0.0, |v| v.dim.x)
            {
                best_data = Some(d);
            }
        }
        best_pos_data = best_data.cloned();
    });

    if let Some(d) = best_pos_data.as_ref() {
        for en in &d.blueprint.blueprint.entities {
            if let Some(tag) = en.player_description.as_ref() {
                pos_dict.insert(tag, en);
            }
        }
    }
    let mut in_ent_n: Option<u32> = None;
    let mut curr_ent_n = base_ent_n;
    let mut other_ents = Vec::new(); // shifters / etc
    let mut wires = Wires::new();
    let mut out_ent_n: Option<u32> = None;

    let use_const_cbs = matches!(args.image_args.mode, Mode::Static { const_cb: true, .. });
    let mut height = best_pos_data.as_ref().map_or(0.0, |v| v.dim.y);
    let base_offset = start_pos; // Add a constant offset on dimensions
    let cb_midpoint = if use_const_cbs {
        const { dvec2(0.5, -0.5) }
    } else if use_compact_layout {
        const { dvec2(0.5, -1.0) } // Base position for vertical layout
    } else {
        const { dvec2(1.0, -0.5) } // Base position for horizontal layout
    };

    let mut palette_in_ent_n: Option<u32> = None;
    let mut ent_to_palette_ent_n: Option<u32> = None;
    let mut data_cbs = LazyCombinators {
        base_pos: start_pos,
        cb_midpoint,
        out_ent_n: None,
        curr_pos: DVec2::ZERO,
        direction: if use_const_cbs {
            constants::DIR_D
        } else if use_compact_layout {
            constants::DIR_U
        } else {
            constants::DIR_R
        },
        first_ent_n: (curr_ent_n == base_ent_n).then_some(curr_ent_n),
        grid_pos: IVec2::ZERO,
        // Calculate the offsets to test for collisions. In Factorio, all collision boxes are rectangular.
        hitbox_offsets: {
            let size = cb_midpoint.abs() * 2.0;
            let dx = (0..size.x as usize).map(|v| v as f64 + 0.5 - size.x / 2.0);
            let dy = (0..size.y as usize).map(|v| v as f64 + 0.5 - size.y / 2.0);
            let dy = dy.collect::<Vec<_>>();
            let deltas = dx.flat_map(|x| dy.iter().map(move |&y| dvec2(x, y)));
            deltas.collect::<Box<_>>()
        },
        in_ent_n: curr_ent_n,
        offset: DVec2::ZERO,
        max_cols_per_grp,
        prev_ents_n: vec![None; max_cols_per_grp as usize].into_boxed_slice(),
        prev_prev_ents_n: vec![None; max_cols_per_grp as usize].into_boxed_slice(),
        target_name: Arc::clone(if use_const_cbs {
            &constants::CONSTANT_COMB
        } else {
            &DEC_CB
        }),
        tick_i: 0usize,
        wires: Wires::new(),
    };

    if let Some(palette) = palette {
        struct EntityCand {
            grid_x: i32,
            ent_n:  u32,
        }

        let mut best_first_connection_cand: Option<EntityCand> = None;
        let prev_name = Arc::clone(&data_cbs.target_name);
        data_cbs.target_name = DEC_CB.clone();

        for (i, &[r, g, b, ..]) in palette.colors.iter().enumerate().skip(1) {
            in_ent_n.get_or_insert(curr_ent_n); // The input timing wire should connect to the first palette combinator
            out_ent_n.get_or_insert(curr_ent_n);

            // Generate next entity without including wires
            let (next_ent_n, mut en) = data_cbs.next_entity(curr_ent_n, occupied, true);
            let color = rgb_to_int(r, g, b);
            let grid_x = data_cbs.grid_pos.x;

            en.control_behavior = Some(ControlBehavior::decider(DeciderCombinatorParameters {
                conditions: boxed_slice![
                    decider_cond!(SIG_EACH, ==, i32::try_from(i).unwrap())
                        .with_first_signal_networks("g"),
                ],
                outputs:    boxed_slice![DeciderCombinatorOutput::new(
                    &SIG_EACH,
                    Some(color.cast_signed()),
                )],
            }));

            if args.detailed_descriptions {
                en = en.with_description(&[
                    // Note: not i + 1 because we already skip the first colour (black)
                    format!("[font=var]Index combinator {} / {}.", i, palette.colors.len() - 1),
                    format!("\nMaps the index '{i}' to the colour [color=#{color:06X}]●[/color] #{color:06X}.[/font]"),
                ].join("\n"));
            }
            other_ents.push(en);

            if best_first_connection_cand.as_ref().is_none_or(|c| grid_x <= c.grid_x) {
                best_first_connection_cand = Some(EntityCand { grid_x, ent_n: curr_ent_n });
            }

            curr_ent_n = next_ent_n;
        }

        // Offset the height by the size of the generated palette combinators
        height -= 2.0 * f64::from(1 + data_cbs.grid_pos.y) * cb_midpoint.y;

        data_cbs.reset();
        data_cbs.grid_pos.x = 0;
        data_cbs.grid_pos.y = 0;
        data_cbs.target_name = prev_name;

        if let Some(cand) = best_first_connection_cand {
            palette_in_ent_n = Some(cand.ent_n);
            data_cbs.out_ent_n = Some(cand.ent_n); // Maybe overridden by delta / grayscale
        }
    }

    // Finds the position for the combinator with the given tag, based off the blueprint string we
    // decoded earlier
    let get_pos = |tag: &str| -> DVec2 {
        let entry = pos_dict.get(tag).unwrap_or_else(|| panic!("No '{tag}'! ({bits} bits)"));
        let mut pos = entry.position + base_offset;
        pos.y -= height;
        pos
    };
    let get_dir = |tag: &str| -> u32 { pos_dict.get(tag).unwrap().direction.unwrap_or(0) };

    {
        let mut used_tags: Vec<&str> = Vec::new();
        if use_delta_comp {
            used_tags.extend(["delay", "memory"]);
        }
        if bits > 0 {
            used_tags.extend([">>", "AND"]);
            if palette.is_none() && matches!(bits, 1 | 4) {
                used_tags.push("*");
            }
        }

        for tag in used_tags {}
    }

    // Note: signal compression is auto-disabled if generating a static image
    if use_delta_comp {
        {
            let mut en = BlueprintEntity::new(curr_ent_n, &DEC_CB, get_pos("delay"));
            en = en.with_tag("delay comb").with_direction(get_dir("delay"));
            en = en.with_control_behavior(ControlBehavior::decider(DeciderCombinatorParameters {
                conditions: boxed_slice![decider_cond!(SIG_EACH, !=, 0)],
                outputs:    boxed_slice![DeciderCombinatorOutput::new(&SIG_EACH, None)],
            }));
            if args.detailed_descriptions {
                en = en.with_description("Utility 1-tick delay combinator.");
            }
            other_ents.push(en);

            if let Some(palette_in_ent_n) = palette_in_ent_n {
                wires.push([palette_in_ent_n, WIRE_R, curr_ent_n, WIRE_R]);
            }
            in_ent_n.get_or_insert(curr_ent_n);
            data_cbs.in_ent_n = curr_ent_n; // Data combinator timing wires should share the same timing wires as this combinator
            curr_ent_n += 1;
        }
        {
            let mut en = BlueprintEntity::new(curr_ent_n, &DEC_CB, get_pos("memory"));
            en = en.with_direction(get_dir("memory")).with_tag("memory comb");
            en = en.with_control_behavior(ControlBehavior::decider(DeciderCombinatorParameters {
                conditions: boxed_slice![
                    decider_cond!(SIG_EACH, !=, 0).with_first_signal_networks("g"),
                    decider_cond!(args.timing_signals.p, !=, 0)
                        .with_first_signal_networks("r")
                        .with_compare_type("&"),
                ],
                outputs:    boxed_slice![
                    DeciderCombinatorOutput::new(&SIG_EACH, None).with_networks("g")
                ],
            }));
            if args.detailed_descriptions {
                en = en.with_description("Memory cell for delta compression. This holds all the signal values, then allows us to set any signal value by applying a delta. We can reach any number in one tick using overflow logic.");
            }
            other_ents.push(en);
            data_cbs.out_ent_n = Some(curr_ent_n); // data combinators should output to the memory combinator

            ent_to_palette_ent_n = Some(curr_ent_n);

            // Self connection for memory
            wires.push([curr_ent_n, WIRE_G, curr_ent_n, WIRE_OUT_G]);
            wires.push(mkwires!(R other_ents; OUT "delay comb" => IN "memory comb"));
            curr_ent_n += 1;
        }
    }

    if bits > 0 {
        {
            let mut en = BlueprintEntity::new(curr_ent_n, &ARI_CB, get_pos(">>"));
            en = en.with_tag(">> comb").with_direction(get_dir(">>"));
            if args.detailed_descriptions {
                en = en.with_description(
                    "Shifts the input numbers until they are in the range of the current frame.",
                );
            }
            other_ents.push(en.with_control_behavior(ControlBehavior::arithmetic(
                ArithmeticCombinatorParameters {
                    first_signal:           Some(Arc::clone(&SIG_EACH)),
                    first_signal_networks:  "g".into(),
                    second_signal:          Some(Arc::clone(&args.timing_signals.f)),
                    second_signal_networks: "r".into(),
                    second_constant:        0,
                    operation:              ArithmeticCombinatorParameterOperation::Shr,
                    output_signal:          Some(Arc::clone(&SIG_EACH)),
                },
            )));

            data_cbs.out_ent_n = Some(curr_ent_n);
            in_ent_n.get_or_insert(curr_ent_n);
            curr_ent_n += 1;
        }
        {
            let mut en = BlueprintEntity::new(curr_ent_n, &ARI_CB, get_pos("AND"));
            en = en.with_tag("AND comb").with_direction(get_dir("AND"));
            if args.detailed_descriptions {
                en = en.with_description("Filters out the bits that are not relevant for the current frame, after bit-shifting. (The value of each signal encodes multiple frames)");
            }
            other_ents.push(en.with_control_behavior(ControlBehavior::arithmetic(
                arithmetic_conds!(SIG_EACH AND ((1u32 << bits) - 1).cast_signed() => SIG_EACH),
            )));

            ent_to_palette_ent_n = Some(curr_ent_n);
            if use_delta_comp {
                wires.push(mkwires!(G other_ents; OUT "memory comb" => IN ">> comb"));
                wires.push(mkwires!(R other_ents; OUT "delay comb" => IN ">> comb"));
            }
            wires.push(mkwires!(G other_ents; OUT ">> comb" => IN "AND comb"));
            curr_ent_n += 1;
        }

        if palette.is_none() && matches!(bits, 1 | 4) {
            let mut en = BlueprintEntity::new(curr_ent_n, &ARI_CB, get_pos("*"));
            en = en.with_tag("* comb").with_direction(get_dir("*"));
            other_ents.push(en.with_control_behavior(ControlBehavior::arithmetic(
                arithmetic_conds!(SIG_EACH * if bits == 1 { 255 } else { 17 } => SIG_EACH),
            )));
            wires.push(mkwires!(G other_ents; OUT "AND comb" => IN "* comb"));
            curr_ent_n += 1;
        }
    }

    if let Some((a, b)) = palette_in_ent_n.zip(ent_to_palette_ent_n) {
        wires.push([a, WIRE_G, b, WIRE_OUT_G]);
    }

    let in_ent_n = in_ent_n.unwrap_or(curr_ent_n);
    let out_ent_n = out_ent_n.unwrap_or_else(|| (curr_ent_n - 1).max(base_ent_n));

    data_cbs.base_pos.y -= height;
    data_cbs.first_ent_n = (curr_ent_n == base_ent_n).then_some(curr_ent_n);

    GenerateCombinatorsOutput {
        data_cbs,
        in_ent_n,
        out_ent_n,
        other_ents,
        metadata: ObjectGeneratorMetadata {
            wires,
            next_ent_n: curr_ent_n + u32::from(curr_ent_n == base_ent_n),
        },
    }
}

pub struct LazyCombinators {
    base_pos:         DVec2,
    cb_midpoint:      DVec2,
    curr_pos:         DVec2,
    direction:        u32,
    /// Forces the first entity number for e.g. when ``LazyCombinators.next()`` returns
    /// the first entity a wire should connect to
    first_ent_n:      Option<u32>,
    grid_pos:         IVec2,
    hitbox_offsets:   Box<[DVec2]>,
    max_cols_per_grp: u32,
    offset:           DVec2,
    out_ent_n:        Option<u32>,
    prev_ents_n:      Box<[Option<u32>]>,
    prev_prev_ents_n: Box<[Option<u32>]>,
    target_name:      Arc<str>,
    tick_i:           usize,
    in_ent_n:         u32,
    pub wires:        Wires,
}
impl LazyCombinators {
    pub fn next(
        &mut self,
        curr_ent_n: u32,
        occupied: &PowerPoleOccupied,
    ) -> (u32, BlueprintEntity) {
        self.next_entity(curr_ent_n, occupied, false)
    }

    fn next_entity(
        &mut self,
        mut curr_ent_n: u32,
        occupied: &PowerPoleOccupied,
        connect_inputs: bool,
    ) -> (u32, BlueprintEntity) {
        if let Some(ent_n) = self.first_ent_n {
            curr_ent_n = ent_n; // Forcibly use a custom ent_n for wiring if first
        }

        let (mut curr_pos, mut offset): (DVec2, DVec2);
        let (cb_mid, mut grid_pos) = (self.cb_midpoint, self.grid_pos);

        loop {
            offset = cb_mid * (1 + 2 * grid_pos).as_dvec2(); // Set target midpoint
            curr_pos = self.base_pos + offset;

            if offset.x + self.cb_midpoint.x > f64::from(self.max_cols_per_grp) {
                grid_pos.x = 0;
                grid_pos.y += 1;
            } else if (self.hitbox_offsets.iter()).any(|o| !occupied.test(curr_pos + *o)) {
                grid_pos.x += 1; // Note we test if powerage is needed, don't necessarily request it
            } else {
                break;
            }
        }

        if self.tick_i == 0 {
            if let Some(data_in_ent_n) = self.out_ent_n {
                self.wires.push([data_in_ent_n, WIRE_G, curr_ent_n, WIRE_OUT_G]);
            }
            self.wires.push([self.in_ent_n, WIRE_R, curr_ent_n, WIRE_R]);
        }

        let x_i = grid_pos.x as usize;
        {
            let prefer_horiz = false;
            let mut cands: Vec<usize> = Vec::new();

            if grid_pos.y > 0 && (!prefer_horiz || grid_pos.x == 0) {
                cands.push(x_i); // Vertical connection
            }
            if grid_pos.x > 0
                && (prefer_horiz
                    || self.prev_ents_n[x_i].is_none()
                    || self.prev_prev_ents_n[x_i - 1].is_none())
            {
                cands.push(x_i - 1); // Horizontal connection
            }
            for i in cands {
                if let Some(prev) = self.prev_ents_n[i] {
                    self.wires.push([prev, WIRE_R, curr_ent_n, WIRE_R]); // Timing signals wire
                    self.wires.push([prev, WIRE_OUT_G, curr_ent_n, WIRE_OUT_G]); // Lamp data wire

                    if connect_inputs {
                        self.wires.push([prev, WIRE_G, curr_ent_n, WIRE_G]); // Lamp data wire
                    }
                }
            }
        }

        grid_pos.x += 1;

        self.curr_pos = curr_pos;
        self.grid_pos = grid_pos;
        self.offset = offset;
        self.prev_ents_n[x_i] = Some(curr_ent_n);
        self.prev_prev_ents_n[x_i] = self.prev_ents_n[x_i];
        self.tick_i += 1;

        let en = BlueprintEntity::new(curr_ent_n, &self.target_name, self.curr_pos)
            .with_direction(self.direction);

        self.first_ent_n.take(); // Discard ``first_ent_n`

        (curr_ent_n + 1, en)
    }

    fn reset(&mut self) {
        self.tick_i = 0;
        self.prev_ents_n.fill(None);
        self.prev_prev_ents_n.fill(None);
    }
}

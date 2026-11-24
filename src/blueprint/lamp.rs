use super::power_pole::PowerPoleOccupied;
use super::prelude::*;
use crate::prelude::*;
use alloc::sync::Arc;
use core::ops::Not as _;
use glam::{DVec2, UVec2, dvec2};

type GenerateLampsOutput = (
    Vec<BlueprintEntity>,
    ObjectGeneratorMetadata,
    (Option<u32>, Option<u32>),
);

/// Generates a grid of lamp entities for the blueprint.
///
/// # Arguments
///
/// * `signals` - Signals to assign to each lamp.
/// * `grid_dim` - Number of lamps horizontally and vertically.
/// * `curr_frame` - Current frame.
/// * `occupied` - Tracks cells occupied by power poles.
/// * `base_ent_n` - Starting entity number for lamps.
/// * `start_pos` - Starting X and Y coordinates.
/// * `args` - Args for generating the blueprint.
///
/// # Returns
///
/// A tuple with lamp entities, lamp wires, the next entity number, and the top-right lamp entity.
#[expect(clippy::needless_pass_by_value, reason = "Arc")]
pub fn generate_lamps(
    signals: &[SignalID],
    grid_dim: UVec2,
    curr_frame: Option<Arc<image::DynamicImage>>,
    occupied: &mut PowerPoleOccupied,
    base_ent_n: u32,
    start_pos: DVec2,
    args: &BlueprintArgs,
) -> Result<GenerateLampsOutput, JsError> {
    let mut ents = Vec::new();
    let mut wires = Wires::new();
    let mut top_left_ent_n: Option<u32> = None;
    let mut top_right_ent_n: Option<u32> = None;

    // Auto-rotate wires based on image rotation
    let prefer_horizontal_wires =
        args.prefer_horizontal_wires ^ args.image_args.rotate.swaps_dimensions();
    let base_pos = start_pos + const { dvec2(0.5, 0.5) }; // Lamps are centered on the middle of a tile
    let mode = args.image_args.mode;
    let get_ent_num = |r: u32, c: u32| -> u32 { base_ent_n + r * grid_dim.x + c };
    let mut prev_ent_ns: Vec<Option<u32>> = vec![None; grid_dim.x as usize];
    let curr_frame = curr_frame.as_ref().map(|f| f.to_rgba8());

    for r in 0..grid_dim.y as usize {
        let mut prev_ent_n: Option<u32> = None; // Used for horizontal wires
        let mut did_connect_row = false; // Track whether we've connected a row yet
        let mut was_top = false;

        for c in (0..grid_dim.x as usize).rev() {
            let pos = base_pos + dvec2(c as f64, r as f64);
            if !occupied.request(pos) {
                continue;
            }

            let curr_ent_n = get_ent_num(r as u32, c as u32);
            let mut lamp =
                BlueprintEntity::new(curr_ent_n, &constants::ENT_LAMP, pos).with_always_on(true);
            let index = r * grid_dim.x as usize + c;

            if unlikely(top_right_ent_n.is_none()) {
                top_right_ent_n = Some(curr_ent_n); // Handle case of only one column
            }

            if matches!(args.image_args.mode, Mode::Static { combs: false, .. }) {
                let frame = curr_frame.as_ref();
                let frame =
                    frame.ok_or_js_err("StaticImage without combintors requires a frame")?;
                let p = frame.get_pixel(c as u32, r as u32);
                lamp = lamp.with_color(Color {
                    r: f64::from(p[0]) / 255.0,
                    g: f64::from(p[1]) / 255.0,
                    b: f64::from(p[2]) / 255.0,
                    a: f64::from(p[3]) / 255.0,
                });
            } else {
                let sig = &signals[index];
                let grayscale_bits = args.image_args.grayscale_bits;
                lamp = lamp.with_control_behavior(ControlBehavior::Lamp {
                    use_colors:   true,
                    color_mode:   if grayscale_bits > 0 { 1 } else { 2 },
                    red_signal:   (grayscale_bits > 0).then(|| sig.clone()),
                    green_signal: (grayscale_bits > 0).then(|| sig.clone()),
                    blue_signal:  (grayscale_bits > 0).then(|| sig.clone()),
                    rgb_signal:   (grayscale_bits > 0).not().then(|| sig.clone()),
                });
            }

            if !matches!(mode, Mode::Static { combs: false, .. }) {
                let is_top = unlikely(prev_ent_ns[c].is_none());

                // prev_ents_n tracks the entity numbers for the rows above
                // if it's none then we never connected the top row
                if !matches!(mode, Mode::LampGrid | Mode::Static { timer: false, .. })
                    && (is_top && (r == 0 || top_left_ent_n.is_none()))
                    && let Some(prev) = prev_ent_n
                {
                    wires.push([curr_ent_n, WIRE_R, prev, WIRE_R]); // horizontal timing wire
                }
                if (is_top || prefer_horizontal_wires || unlikely(was_top))
                    && let Some(prev) = prev_ent_n
                {
                    wires.push([curr_ent_n, WIRE_G, prev, WIRE_G]); // horizontal data wire
                }
                if (!did_connect_row || !prefer_horizontal_wires)
                    && let Some(prev) = prev_ent_ns[c]
                {
                    wires.push([curr_ent_n, WIRE_G, prev, WIRE_G]); // vertical data wire
                    did_connect_row = true;
                }
                was_top = is_top;
            }

            ents.push(lamp);
            prev_ent_n = Some(curr_ent_n);
            prev_ent_ns[c] = prev_ent_n;
        }

        if unlikely(top_left_ent_n.is_none()) {
            top_left_ent_n = prev_ent_n;
        }
    }
    let output = ObjectGeneratorMetadata {
        wires,
        next_ent_n: get_ent_num(grid_dim.y - 1, grid_dim.x - 1) + 1,
    };

    Ok((ents, output, (top_left_ent_n, top_right_ent_n)))
}

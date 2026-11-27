use std::io;

use glam::{DVec2, UVec2, dvec2, uvec2};
use image::GenericImageView as _;

use super::combinator::{GenerateCombinatorsOutput, LazyCombinators, generate_combinators};
use super::lamp::generate_lamps;
use super::power_pole::PowerPoleOccupied;
use super::prelude::*;
use super::timer::generate_timer;
use crate::blueprint::data::with_prototype_data;
use crate::callbacks::report_lamp_group;
use crate::image_processing::FrameData;
use crate::models::RawJsonString;
use crate::prelude::*;
use crate::util::format_duration;
use crate::{boxed_slice, decider_cond, image_util};

/// How many entities to encode in one go. Encoding more entities at once is
/// faster, but uses more memory.
const ENCODE_CHUNK_SIZE: usize = 100;

/// Number of wires to encode per chunk. Since wires generate much less data,
/// we can batch more wires together for better speed.
const WIRE_ENCODE_CHUNK_SIZE: usize = 5000;

type SignalDataResult = (Box<[RC<SignalID>]>, RC<[RawJsonString]>);

/// Precomputed transformation for flipping entities.
#[derive(Clone, Copy)]
struct PositionTransform {
    swap:   bool,
    x_sign: f64,
    y_sign: f64,
}
impl PositionTransform {
    const fn new(args: &BlueprintArgs) -> Self {
        let (mut x_sign, mut y_sign, swap) = match args.image_args.rotate {
            ImageRotate::Deg90 => (1.0, -1.0, true),
            ImageRotate::Deg180 => (-1.0, -1.0, false),
            ImageRotate::Deg270 => (-1.0, 1.0, true),
            _ => (1.0, 1.0, false),
        };

        if args.image_args.flip.x() {
            x_sign = -x_sign;
        }
        if args.image_args.flip.y() {
            y_sign = -y_sign;
        }

        Self { swap, x_sign, y_sign }
    }

    #[must_use]
    #[inline]
    const fn apply(self, pos: DVec2) -> DVec2 {
        if self.swap {
            dvec2(pos.y * self.x_sign, pos.x * self.y_sign)
        } else {
            dvec2(pos.x * self.x_sign, pos.y * self.y_sign)
        }
    }
}

/// Data for blueprint generation.
pub struct BlueprintGenerator<'a> {
    /// Copy of the parsed arguments passed in from JS.
    args:               &'a BlueprintArgs,
    description:        Option<String>,
    /// Stores entities, wires, and next entity number.
    entity_data:        EntityData,
    /// Stores the iterator that generates the next frame, for streaming output.
    /// May be None if only generating the lamp grid.
    frame_info:         Option<FrameInfo<'a>>,
    /// Whether to generate any combinators or not.
    generate_cbs:       bool,
    /// Target dimensions of the image or video.
    dim:                UVec2,
    /// Maximum number of lamp columns per group. Groups are always divided
    /// split vertically.
    max_cols_per_grp:   u32,
    /// Total number of groups to generate. Each group is a vertical split of
    /// the lamps, with no data connections. Timing wires still connect each
    /// group.
    n_groups:           u32,
    /// Keeps track of positions currently occupied by power poles, and used to
    /// request powerage for generated entities.
    occupied:           PowerPoleOccupied,
    position_transform: PositionTransform,
    /// List of unique signals to use for the blueprint.
    signals:            Box<[RC<SignalID>]>,
    /// Stores the JSON representation of each signal for efficient JSON
    /// reproduction
    signals_json:       RC<[RawJsonString]>,
    state:              BlueprintState,
}

/// Data only used when generating from an image or a video.
struct FrameInfo<'a> {
    curr_frame:       RC<image::DynamicImage>,
    frame_data:       FrameData<'a>,
    frames_per_cb:    u32,
    group_data_combs: Box<[LazyCombinators]>,
    n_buf_frames:     u32,
    next_frame:       Option<RC<image::DynamicImage>>,
    patch_states:     Box<[GroupPatchState]>,
    ticks_per_frame:  u8,
    use_delta_comp:   bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BlueprintState {
    Start,
    Entities,
    Data,
    Wires,
    Finished,
}

#[derive(Default)]
struct EntityData {
    ents:       Vec<BlueprintEntity>,
    wires:      Wires,
    next_ent_n: u32,
}
struct GroupPatchState {
    chunk_i:   u32, // which *chunk* we’re patching (not frame)
    frame_buf: Vec<image::SubImage<RC<image::DynamicImage>>>,
    sig_buf:   Vec<Box<[i32]>>,
}

impl<'a> BlueprintGenerator<'a> {
    pub async fn new(
        data: Option<FrameData<'a>>, args: &'a BlueprintArgs,
    ) -> Result<Self, JsError> {
        let (signals, signals_json) = Self::get_signal_data(args)?;
        let comb_comp = args.signal_compression;
        let time_comp_win = match comb_comp {
            Some(SignalCompression::Temporal { window }) => window,
            _ => 0,
        };
        let mut frame_info: Option<FrameInfo> = None;
        let dim: UVec2;

        if let Some(mut frame_data) = data {
            let frames_per_cb = 32_u32.checked_div(args.image_args.bits()).unwrap_or(1);
            let n_buf_frames = if time_comp_win > 0 {
                (time_comp_win * u32::from(args.image_args.target_fps))
                    .div_ceil(1000 * frames_per_cb)
            } else {
                1
            };
            dim = frame_data.out_dim();
            frame_info = Some(FrameInfo {
                curr_frame: frame_data.next().await?.ok_or_js_err("Expected at least 1 frame")?,
                frame_data,
                group_data_combs: boxed_slice![],
                frames_per_cb,
                n_buf_frames,
                next_frame: None,
                patch_states: boxed_slice![],
                ticks_per_frame: 60_u8.div_euclid(args.image_args.target_fps),
                use_delta_comp: comb_comp.is_some_and(|c| c == SignalCompression::Delta),
            });
        } else {
            dim = args.custom_dim.ok_or_js_err("Must have custom_dim or frame_data.")?;
        }
        let use_const_cbs = matches!(args.image_args.mode, Mode::Static { const_cb: true, .. });
        let max_lamp_cols_per_grp = match args.image_args.mode {
            Mode::Static { combs: false, .. } => dim.x,
            // Constant combinators can only hold up to 1000 signals
            _ => ((signals.len() as u32)
                .min(if use_const_cbs { 255_000 } else { u32::MAX })
                .div_euclid(dim.y))
            .min(args.max_group_size.map_or(dim.x, u32::from)),
        };
        assert!(
            max_lamp_cols_per_grp >= 1,
            "Not enough signals for even one lamp column!"
        );
        let n_groups = dim.x.div_ceil(max_lamp_cols_per_grp);

        if let Some(frame_info) = frame_info.as_mut() {
            let states_iter = std::iter::repeat_with(|| GroupPatchState {
                chunk_i:   0,
                frame_buf: Vec::with_capacity(frame_info.frames_per_cb as usize),
                sig_buf:   Vec::with_capacity(frame_info.n_buf_frames as usize),
            })
            .take(n_groups as usize);
            frame_info.patch_states = states_iter.collect();
        }

        Ok(Self {
            args,
            description: None,
            dim,
            entity_data: EntityData::default(),
            frame_info,
            generate_cbs: !matches!(
                args.image_args.mode,
                Mode::Static { combs: false, .. } | Mode::Lamps | Mode::LampGrid
            ),
            max_cols_per_grp: max_lamp_cols_per_grp,
            n_groups,
            occupied: PowerPoleOccupied::new(dvec2(0.0, -f64::from(args.display_margin_y)), args)?,
            position_transform: PositionTransform::new(args),
            signals,
            signals_json,
            state: BlueprintState::Start,
        })
    }

    #[inline]
    fn transform_entity(
        args: &BlueprintArgs, position_transform: &PositionTransform, en: &mut BlueprintEntity,
    ) {
        if !args.prefer_green_wires
            && let Some(cb) = en.control_behavior_mut()
        {
            cb.maybe_swap_networks();
        }

        en.position = position_transform.apply(en.position);

        if let Some(dir) = en.direction_mut() {
            let mut v = *dir;
            match args.image_args.rotate {
                ImageRotate::Deg90 => v += 12,
                ImageRotate::Deg180 => v += 8,
                ImageRotate::Deg270 => v += 4,
                _ => (),
            }
            match args.image_args.flip {
                ImageFlip::X if (v + 4).rem_euclid(8) == 0 => v += 8,
                ImageFlip::Y if v.rem_euclid(8) == 0 => v += 8,
                ImageFlip::Both => v += 8,
                _ => (),
            }
            *dir = v.rem_euclid(16);
        }
    }

    fn get_signal_data(args: &BlueprintArgs) -> Result<SignalDataResult, JsError> {
        let mut signals = super::data::with_prototype_data(|data| {
            let mut blacklisted_sigs = data.blacklist_signals.clone();

            if !matches!(
                args.image_args.mode,
                Mode::LampGrid | Mode::Lamps | Mode::Static { timer: false, .. }
            ) {
                blacklisted_sigs.extend(args.timing_signals.values().cloned());
            }

            let filtered = data.signals.iter().filter(|&v| !blacklisted_sigs.contains(v));
            Ok::<_, JsError>(filtered.cloned().collect::<Box<_>>())
        })?;

        match args.signal_sorting {
            // Only sort by name (compressor really likes this)
            SignalSorting::Compression => {
                signals.sort_by(|a, b| a.type_.cmp(&b.type_).then_with(|| a.name.cmp(&b.name)));
                signals.sort_by_key(|s| s.type_.len());
            },
            // Sort by total length of the type + name + quality (actual smallest JSON)
            SignalSorting::Json => signals.sort_by_key(|s| {
                (match s.quality.as_ref() {
                    DEFAULT_QUALITY => 0,
                    q => q.len() + const { r#""quality":"","#.len() },
                }) + (match s.type_.as_ref() {
                    DEFAULT_SIGNAL_TYPE => 0,
                    t => t.len() + const { r#""type":"","#.len() },
                }) + s.name.len()
            }),
            _ => {},
        }
        signals.sort_by_key(|s| s.name.contains("parameter")); // Push "parameter" signals to end

        let signals_str = (signals.iter().map(|sig| -> Result<_, JsError> {
            let s = serde_json::to_string(sig).map_err(|e| {
                js_err!("Could not convert input signal '{sig:?}' to string: {e:?}")
            })?;
            serde_json::value::RawValue::from_string(s)
                .map_js_err("Could not convert the string into a serde_json::value::RawValue")
        }))
        .collect::<Result<RC<_>, _>>()?;

        Ok((signals, signals_str))
    }

    #[inline]
    #[must_use]
    pub fn done(&self) -> bool {
        self.state == BlueprintState::Finished
    }

    /// Generates the next chunk of the blueprint and writes it to the given
    /// writer.
    pub async fn next_chunk(&mut self, writer: &mut impl io::Write) -> Result<(), JsError> {
        let old_state = self.state;
        match old_state {
            BlueprintState::Start => {
                write_to!(writer, br#"{"blueprint":{"#)?;

                let bp_color = match self.args.image_args.grayscale_bits {
                    0 => "#8888ff",
                    _ => "#888888",
                };
                let name = self.args.name.clone();
                let name = format!("{name} ({}x{})", self.dim.x, self.dim.y);
                let bp = BlueprintInner {
                    description: None,
                    icons:       Some(boxed_slice![Icon {
                        signal: SignalID::intern("item", constants::LAMP)?,
                        index:  1,
                    }]),
                    item:        constants::BLUEPRINT.to_owned(),
                    label:       Some(match self.args.image_args.mode {
                        Mode::Full => name,
                        Mode::Lamps => format!("{}x{} Lamps", self.dim.x, self.dim.y),
                        Mode::LampGrid => format!("{}x{} Lamp Grid", self.dim.x, self.dim.y),
                        Mode::Static { .. } => self.args.name.clone(),
                    }),
                    label_color: Color::from_hex(bp_color),
                    version:     constants::BLUEPRINT_VERSION,
                    entities:    boxed_slice![],     // don't serialize
                    wires:       Some(Wires::new()), // don't serialize
                };

                write_json_trimmed_to!(writer, &bp)?;
                self.state = BlueprintState::Entities;
            },
            BlueprintState::Entities => self.place_entities(writer)?,
            BlueprintState::Data => {
                if self.generate_cbs {
                    self.populate_data(writer).await?;
                } else {
                    self.state = BlueprintState::Wires;
                }
            },
            BlueprintState::Wires => self.make_wires(writer)?,
            BlueprintState::Finished => err_js!("Already processed all chunks"),
        }

        // Properly delimit the JSON stream before the next chunks
        if old_state != self.state {
            match self.state {
                BlueprintState::Entities => write_to!(writer, br#","entities":["#)?,
                BlueprintState::Wires => {
                    // Set description of constant combinator
                    if true
                        && let Some(incrementer) = self.entity_data.ents.first()
                        && incrementer.has_custom_tag("incrementer")
                    {
                        let description = self.get_description()?;
                        self.entity_data.ents[0].set_player_description(&description);
                    }

                    for en in &self.entity_data.ents {
                        self.occupied.request(en.position); // Request power poles to power entities
                    }
                    let (ents, wires, _) =
                        self.occupied.generate_power_poles(self.entity_data.next_ent_n, self.args);
                    self.entity_data.ents.extend(ents);
                    self.entity_data.wires.extend(wires);

                    for en in &mut self.entity_data.ents {
                        Self::transform_entity(self.args, &self.position_transform, en);
                    }

                    // Write remaining entities. Data combinators were NOT included in
                    // self.entity_data.ents
                    for ents in self.entity_data.ents.chunks(ENCODE_CHUNK_SIZE) {
                        write_to!(writer, b",")?;
                        write_json_trimmed_to!(writer, &ents)?;
                    }

                    write_to!(writer, br#"],"wires":["#)?;
                },
                BlueprintState::Finished => {
                    write_to!(writer, b"]")?;
                    write_to!(writer, br#","description":""#)?;
                    write_json_trimmed_to!(writer, &self.get_description()?)?;
                    write_to!(writer, br#"""#)?;
                    write_to!(writer, b"}}")?;
                },
                _ => {},
            }
        }
        Ok(())
    }

    #[inline]
    pub fn get_description(&mut self) -> Result<String, JsError> {
        Ok(if let Some(ref description) = self.description {
            description.clone()
        } else {
            let description = self.generate_description()?;
            self.description = Some(description.clone());
            description
        })
    }

    /// Gets the description to attach to the blueprint.
    #[allow(clippy::unnecessary_wraps, reason = "API may change in the future")]
    fn generate_description(&self) -> Result<String, JsError> {
        // Note: we use the `var` font for bold text as the word "var" is short and
        // takes up less space.
        let name = self.args.name.clone();
        let info = self.frame_info.as_ref();
        let dim = self.dim;

        let mut output: Vec<String> = Vec::new();
        let name = match self.args.image_args.mode {
            Mode::LampGrid => format!("{}x{} Lamp Grid", dim.x, dim.y),
            Mode::Lamps => format!("{}x{} Lamps", dim.x, dim.y),
            _ => name,
        };

        output.extend([
            format!("[img=item.small-lamp]  [font=compi]{name}[/font]"),
            "\n".to_owned(),
            format!("\n[font=var]Dimensions[/font]:    {}x{}", dim.x, dim.y),
        ]);

        if let Some(fd) = info.map(|x| &x.frame_data) {
            let color_format = match self.args.image_args.grayscale_bits {
                0 => match self.args.image_args.color_palette_size {
                    0 => "Full Color (RGB8)",
                    x => &format!("{x} Color Shades"),
                },
                1 => "1-bit Grayscale (2 shades)",
                4 => "4-bit Grayscale (16 shades)",
                8 => "8-bit Grayscale (256 shades)",
                x => &format!("{x}"),
            };
            output.push(format!("\n[font=var]Color Format[/font]:   {color_format}"));

            if let Some(compression) = self.args.signal_compression {
                let compression = match compression {
                    SignalCompression::Delta => "Delta",
                    SignalCompression::Temporal { window } => &format!("Temporal ({window}ms)"),
                };
                output.push(format!("\n[font=var]Compression[/font]:  {compression}"));
            }

            let fps = self.args.image_args.target_fps;
            let n_frames = fd.curr_total_frames();
            let duration = format_duration((1000 * n_frames as u64).div_euclid(u64::from(fps)));
            output.extend([
                format!("\n[font=var]Duration[/font]:   {duration}"),
                format!("\n[font=var]Frames[/font]:      {n_frames} ({fps} FPS)"),
            ]);
        }

        if with_prototype_data(|v| v.signals.iter().any(|s| s.type_.as_ref() == "space-location")) {
            output.push(
                "\n\n [space-age][font=scenario-message-dialog]  Uses the Space Age DLC.[/font]"
                    .to_owned(),
            );
        }

        let footer = "Generated using Anitorio.";
        output.push(format!("\n\n[font=default-small-semibold]{footer}[/font]"));

        Ok(output.join(""))
    }

    fn place_entities(&mut self, writer: &mut impl io::Write) -> Result<(), JsError> {
        let args = self.args;
        let info = &mut self.frame_info;
        let occupied = &mut self.occupied;
        let signals = &mut self.signals;
        let ent_data = &mut self.entity_data;

        if self.generate_cbs && !matches!(args.image_args.mode, Mode::Static { timer: false, .. }) {
            let info = info.as_mut().ok_or_js_err("Expected frame_info if generate_cbs is true")?;
            let stop = info.frame_data.total_frames() * info.ticks_per_frame as usize;
            let (mut ents, wires) =
                generate_timer(stop, info.ticks_per_frame, info.frames_per_cb, args)?;
            ent_data.next_ent_n = ents.iter().map(|e| e.entity_number).max().unwrap_or(0);
            ent_data.next_ent_n += 1;

            for en in &mut ents {
                en.position.y -= f64::from(args.display_margin_y);
            }

            ent_data.ents.extend(ents);
            ent_data.wires.extend(wires);
        }
        let mut group_data_combs = Vec::<LazyCombinators>::new();
        let mut prev_top_right_lamp_ent_n: Option<u32> = None;
        let n_groups = self.n_groups;

        for group_i in 0..n_groups {
            let is_static = matches!(args.image_args.mode, Mode::Static { .. });
            let thresh = if is_static { 0.60 } else { 0.10 };
            let progress = f64::from(group_i + 1) / f64::from(n_groups);

            set_progress(
                0.00,
                thresh,
                progress,
                &format!("Generating lamps (group {} / {n_groups})", group_i + 1),
            );
            report_lamp_group(progress);

            let grp_left = group_i * self.max_cols_per_grp;
            let grp_width = (self.max_cols_per_grp).min(self.dim.x - grp_left);
            let (mut cb_in_ent_n, mut cb_out_ent_n) = (0, 0);
            let curr_frame =
                &info.as_ref().ok_or_js_err("Expected frame_info if generate_cbs")?.curr_frame;

            let (
                mut grp_lamps,
                ObjectGeneratorMetadata {
                    wires: mut grp_lamp_wires,
                    next_ent_n: new_next_ent_n,
                },
                io_ents_n,
            ) = generate_lamps(
                signals,
                uvec2(grp_width, self.dim.y),
                matches!(self.args.image_args.mode, Mode::Static { combs: false, .. })
                    .then(|| RC::clone(curr_frame)), // Pass image data to lamp generation
                occupied,
                ent_data.next_ent_n,
                dvec2(f64::from(grp_left), 0.0),
                args,
            )?;
            let (top_left_lamp_ent_n, top_right_lamp_ent_n) = io_ents_n;
            ent_data.next_ent_n = new_next_ent_n;

            if self.generate_cbs {
                let info = info.as_mut().ok_or_js_err("Expected frame_info if generate_cbs")?;
                let GenerateCombinatorsOutput {
                    metadata: ObjectGeneratorMetadata { wires, next_ent_n: new_base_ent_n },
                    data_cbs: data_combs,
                    other_ents,
                    in_ent_n,
                    out_ent_n,
                } = generate_combinators(
                    ent_data.next_ent_n,
                    dvec2(f64::from(grp_left), -f64::from(args.display_margin_y)),
                    self.max_cols_per_grp,
                    args,
                    occupied,
                    info.frame_data.get_palette_colors(),
                )?;
                ent_data.next_ent_n = new_base_ent_n;
                if unlikely(group_i == 0) {
                    ent_data.wires.push([3, WIRE_OUT_R, in_ent_n, WIRE_R]); // Connect timer
                }
                group_data_combs.push(data_combs);
                ent_data.ents.extend(other_ents);
                ent_data.wires.extend(wires.into_iter());

                (cb_in_ent_n, cb_out_ent_n) = (in_ent_n, out_ent_n);
            }

            if self.generate_cbs {
                let wires = &mut ent_data.wires;
                if let Some(top_left_lamp_ent_n) = top_left_lamp_ent_n {
                    if !matches!(args.image_args.mode, Mode::Static { timer: false, .. }) {
                        wires.push([top_left_lamp_ent_n, WIRE_R, cb_in_ent_n, WIRE_R]); // Timing wire
                    }

                    // Data wire
                    if args.image_args.grayscale_bits == 0
                        && matches!(args.image_args.mode, Mode::Static { const_cb: true, .. })
                    {
                        wires.push([top_left_lamp_ent_n, WIRE_G, cb_out_ent_n, WIRE_G]);
                    } else {
                        wires.push([top_left_lamp_ent_n, WIRE_G, cb_out_ent_n, WIRE_OUT_G]);
                    }
                }
            }

            // Connect previous lamps together
            if let (Some(prev_right), Some(prev_left)) =
                (prev_top_right_lamp_ent_n, top_left_lamp_ent_n)
            {
                grp_lamp_wires.push([prev_left, WIRE_R, prev_right, WIRE_R]);
            }
            prev_top_right_lamp_ent_n = top_right_lamp_ent_n;

            for lamp in &grp_lamps {
                occupied.request(lamp.position); // Request powerage for lamps
            }
            if likely(group_i > 0) {
                write_to!(writer, b",")?;
            }

            for en in &mut grp_lamps {
                Self::transform_entity(self.args, &self.position_transform, en);
            }

            write_json_trimmed_to!(writer, &grp_lamps)?;
            ent_data.wires.extend(grp_lamp_wires.into_iter());
        }

        if self.generate_cbs {
            info.as_mut()
                .ok_or_js_err("Expected frame_info if generate_cbs")?
                .group_data_combs = group_data_combs.into_boxed_slice();
        }
        ent_data.ents.sort_by_key(|e| e.entity_number);
        self.state = BlueprintState::Data;

        Ok(())
    }

    #[crate::perf("")]
    async fn populate_data(&mut self, writer: &mut impl io::Write) -> Result<(), JsError> {
        let info = self.frame_info.as_mut().ok_or_js_err("Expected frame_info if generate_cbs")?;

        let group_data_combs = &mut info.group_data_combs;
        let frame_data = &mut info.frame_data;
        let signals = &self.signals;

        let frames_per_cb = info.frames_per_cb;
        let gray_bits = self.args.image_args.grayscale_bits;
        let max_cols_per_grp = self.max_cols_per_grp;
        let n_buf_frames = info.n_buf_frames;
        let n_groups = self.n_groups;
        let ticks_per_frame = info.ticks_per_frame;
        let use_delta_comp = info.use_delta_comp;
        let ticks_per_grp = u32::from(ticks_per_frame) * frames_per_cb;
        let mode = self.args.image_args.mode;
        let sig_p = RC::clone(&self.args.timing_signals["p"]);
        let mut output_data: Vec<BlueprintEntity> = Vec::new();

        let mk_control_behavior = |range: UVec2, indices: Box<[(usize, i32)]>| {
            #[crate::perf("mk_control_behavior")]
            Ok::<_, JsError>(if matches!(mode, Mode::Static { const_cb: true, .. }) {
                let sections = (indices.chunks(1000).enumerate().map(|(i, chunk)| {
                    let filters = (chunk.iter().enumerate().map(|(j, &(sig_i, count))| {
                        BlueprintLogisticFilter::new(j as u16 + 1, &signals[sig_i], "==", count)
                    }))
                    .collect::<Result<_, JsError>>()?;
                    Ok(LogisticSection { index: i as u8 + 1, filters })
                }))
                .collect::<Result<_, JsError>>()?;
                let sections = LogisticSections { sections };
                ControlBehavior::ConstantCombinator { sections }
            } else {
                let signal_list = RC::clone(&self.signals_json);
                let conditions = match range.y - range.x {
                    1 => boxed_slice![decider_cond!(RC::clone(&sig_p), ==, range.x.cast_signed())],
                    _ => boxed_slice![
                        decider_cond!(RC::clone(&sig_p), >=, range.x.cast_signed()),
                        decider_cond!(RC::clone(&sig_p), <, range.y.cast_signed())
                            .with_compare_type("&"),
                    ],
                };
                let decider_conditions =
                    DeciderCombinatorCustomParameters { conditions, indices, signal_list };
                ControlBehavior::DeciderCombinatorCustom { decider_conditions }
            })
        };

        loop {
            info.next_frame = frame_data.next().await?;

            let frame = &info.curr_frame;
            let is_last_frame = info.next_frame.is_none();
            let image_args = &self.args.image_args;

            for group_i in 0..n_groups {
                if matches!(image_args.mode, Mode::Static { const_cb: true, .. }) {
                    let thresh = 0.60;
                    set_progress(
                        thresh,
                        0.70,
                        f64::from(group_i + 1) / f64::from(n_groups),
                        &format!("Generating data (group {} / {n_groups})", group_i + 1),
                    );
                }
                let state = &mut info.patch_states[group_i as usize]; // Individual state for each group of lamps
                let data_combs = &mut group_data_combs[group_i as usize];
                let frame_sigs: Box<[i32]>;

                let target_outputs_len: usize;
                let has_palette = frame_data.get_palette_colors().is_some();

                {
                    let grp_left = group_i * max_cols_per_grp;
                    let grp_right = ((group_i + 1) * max_cols_per_grp).min(self.dim.x);
                    let (x, y) = (grp_left, 0);
                    let (w, h) = (grp_right - grp_left, self.dim.y);
                    target_outputs_len = (w * h) as usize;

                    if has_palette || gray_bits > 0 {
                        state.frame_buf.push(image::SubImage::new(RC::clone(frame), x, y, w, h));
                        if state.frame_buf.len() < frames_per_cb as usize && !is_last_frame {
                            continue; // wait until we have a full chunk for this group
                        }
                        let frames = (state.frame_buf.iter())
                            .map(|f| f.inner().view(x, y, w, h))
                            .collect::<Box<_>>();

                        frame_sigs = #[crate::perf("Frames to outputs")]
                        {
                            if has_palette {
                                frame_data.palette_frames_to_outputs(&frames)?
                            } else {
                                image_util::grayscale_frames_to_outputs(&frames, gray_bits.into())?
                            }
                        };

                        state.frame_buf.clear();
                    } else {
                        frame_sigs =
                            image_util::color_frame_to_outputs(&frame.as_ref().view(x, y, w, h))?;
                    }
                }

                assert!(
                    frame_sigs.len() == target_outputs_len,
                    "Outputs length ({}) does not match frame size ({target_outputs_len}).",
                    frame_sigs.len(),
                );

                if use_delta_comp && state.sig_buf.is_empty() {
                    state.sig_buf.push(boxed_slice![0_i32; target_outputs_len]); // Populate first frame with zeros
                } else if !use_delta_comp {
                    state.sig_buf.push(frame_sigs.clone()); // If we're using delta compression we only compare aginst the previous frame
                }

                if state.sig_buf.len() < n_buf_frames as usize && !is_last_frame {
                    continue; // Accumulate until we have `compression_level` outputs,
                }

                if use_delta_comp {
                    let prev_outputs = &state.sig_buf[0];
                    let target_outputs = frame_sigs.iter().enumerate().filter_map(|(i, &v)| {
                        let v = v.wrapping_sub(prev_outputs[i]);
                        (v != 0).then_some((i, v))
                    });
                    let target_outputs = target_outputs.collect::<Box<_>>();
                    if !target_outputs.is_empty() {
                        let (next_ent_n, en) =
                            data_combs.next(self.entity_data.next_ent_n, &self.occupied);
                        self.entity_data.next_ent_n = next_ent_n;

                        let range = uvec2(1, 2) + (ticks_per_grp * state.chunk_i);
                        let cb = mk_control_behavior(range, target_outputs)?;
                        output_data.push(en.with_control_behavior(cb));
                        state.sig_buf[0] = frame_sigs; // cache absolute output
                    }
                } else {
                    // Temporal compression branch
                    let mut changed_mask: Vec<bool>; // to find what changed
                    let outputs_0 = &state.sig_buf[0];
                    let base_chunk_i = state.chunk_i * n_buf_frames;

                    // If we cached more than 1 frame, we can apply temporal compression
                    if state.sig_buf.len() > 1 {
                        changed_mask = vec![false; target_outputs_len];
                        for comb_outputs in state.sig_buf.iter().skip(1) {
                            for i in 0..target_outputs_len {
                                if !changed_mask[i] && (comb_outputs[i] != outputs_0[i]) {
                                    changed_mask[i] = true;
                                }
                            }
                        }
                    } else {
                        changed_mask = vec![true; target_outputs_len];
                    }

                    // Handle changed pixels
                    for (cb_i, cb_outputs) in state.sig_buf.iter().enumerate() {
                        let target_outputs = (cb_outputs.iter().enumerate())
                            .filter_map(|(i, &v)| changed_mask[i].then_some((i, v)))
                            .collect::<Box<_>>();

                        if !target_outputs.is_empty() {
                            let (next_ent_n, en) =
                                data_combs.next(self.entity_data.next_ent_n, &self.occupied);
                            self.entity_data.next_ent_n = next_ent_n;

                            let range =
                                (uvec2(0, 1) + (base_chunk_i + cb_i as u32)) * ticks_per_grp;
                            let cb = mk_control_behavior(range, target_outputs)?;
                            output_data.push(en.with_control_behavior(cb));
                        }
                    }

                    // Handle unchanged pixels
                    if state.sig_buf.len() > 1
                        && let target_outputs = (changed_mask.iter().enumerate())
                            .filter_map(|(i, &c)| (!c).then_some((i, outputs_0[i])))
                            .collect::<Box<_>>()
                        && !target_outputs.is_empty()
                    {
                        let (next_ent_n, en) =
                            data_combs.next(self.entity_data.next_ent_n, &self.occupied);
                        self.entity_data.next_ent_n = next_ent_n;

                        let range = ticks_per_grp
                            * uvec2(base_chunk_i, base_chunk_i + state.sig_buf.len() as u32);
                        let cb = mk_control_behavior(range, target_outputs)?;
                        output_data.push(en.with_control_behavior(cb));
                    }
                    state.sig_buf.clear();
                }
                state.chunk_i += 1;
            }

            // Note: Always advance before checking if we have max entities
            if let Some(next_frame) = info.next_frame.take() {
                info.curr_frame = next_frame;
            } else {
                self.state = BlueprintState::Wires;
                break;
            }
            if unlikely(output_data.len() >= ENCODE_CHUNK_SIZE) {
                break;
            }
        }

        for en in &mut output_data {
            self.occupied.request(en.position); // Request powerage for generated combinators
            Self::transform_entity(self.args, &self.position_transform, en); // Translate positions

            if unlikely(en.name().as_ref() == CONSTANT_CB) {
                en.set_player_description(&self.get_description()?);
            }
        }
        if !output_data.is_empty() {
            write_to!(writer, b",")?;

            if self.args.low_mem_usage {
                let mut buffered = io::BufWriter::with_capacity(512 * 1024, writer);
                let mut trimmed = crate::ioutil::TrimmedWriter::new(&mut buffered);
                serde_json::to_writer(&mut trimmed, &output_data)?;
                trimmed.finish()?;
            } else {
                write_json_trimmed_to!(writer, &output_data)?;
            }
        }

        Ok(())
    }

    fn make_wires(&mut self, writer: &mut impl io::Write) -> Result<(), JsError> {
        let all_wires = &mut self.entity_data.wires;

        if let Some(frame_info) = self.frame_info.take() {
            for cb in frame_info.group_data_combs {
                all_wires.extend(cb.wires.into_iter());
            }
        }

        // Swap all wires if requested (default uses red wires, so swap all for green).
        // Circuit network filters are already handled.
        if !self.args.prefer_green_wires {
            all_wires.swap_networks();
        }

        let n_wires = all_wires.len();
        let mut is_first_write = true;
        let mut buf: Vec<[u32; 4]> = Vec::with_capacity(WIRE_ENCODE_CHUNK_SIZE);

        for (i, wires) in std::mem::take(all_wires).into_iter().enumerate() {
            buf.push(wires);
            if likely(buf.len() < WIRE_ENCODE_CHUNK_SIZE && i < n_wires - 1) {
                continue;
            }

            set_progress(
                0.90,
                1.00,
                i as f64 / n_wires as f64,
                &format!("Generating wires ({i} / {n_wires})..."),
            );

            if likely(!is_first_write) {
                write_to!(writer, b",")?;
            }
            write_json_trimmed_to!(writer, &buf)?;

            buf.clear();
            is_first_write = false;
        }
        self.state = BlueprintState::Finished;

        Ok(())
    }
}

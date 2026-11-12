use super::{
    combinator::{GenerateCombinatorsOutput, LazyCombinators, generate_combinators},
    lamp::generate_lamps,
    power_pole::PowerPoleOccupied,
    prelude::*,
    timer::generate_timer,
    util::{
        color_frame_to_outputs, grayscale_frames_to_outputs, invert_wires, write_json_trimmed_to,
        write_to,
    },
};
use crate::util::format_duration;
use crate::{callbacks::report_lamp_group, prelude::*};
use crate::{image_processing::FrameData, models::RawJsonString};
use glam::{UVec2, dvec2, uvec2};
use std::{io, sync::Arc};

/// How many entities to encode in one go. Encoding more entities at once is faster, but uses more
/// memory.
const ENCODE_CHUNK_SIZE: usize = 100;

/// Number of wires to encode per chunk. Since wires generate much less data,
/// we can batch more wires together for better speed.
const WIRE_ENCODE_CHUNK_SIZE: usize = 5000;

type SignalDataResult = (Box<[Arc<SignalID>]>, Arc<[RawJsonString]>);

/// Data for blueprint generation.
pub struct BlueprintGenerator<'a> {
    /// Copy of the parsed arguments passed in from JS.
    args: &'a BlueprintArgs,

    /// Stores entities, wires, and next entity number.
    entity_data: EntityData,

    /// Stores the iterator that generates the next frame, for streaming output.
    /// May be None if only generating the lamp grid.
    frame_info: Option<FrameInfo<'a>>,

    /// Whether to generate any combinators or not.
    generate_cbs: bool,

    /// Target dimensions of the image or video.
    dim: UVec2,

    /// Maximum number of lamp columns per group. Groups are always divided split vertically.
    max_cols_per_grp: u32,

    /// Total number of groups to generate. Each group is a vertical split of the lamps, with no
    /// data connections. Timing wires still connect each group.
    n_groups: u32,

    /// Keeps track of positions currently occupied by power poles, and used to request powerage
    /// for generated entities.
    occupied: PowerPoleOccupied,
    palette:  Option<Vec<[u8; 4]>>,

    /// List of unique signals to use for the blueprint.
    signals: Box<[Arc<SignalID>]>,

    /// Stores the JSON representation of each signal for efficient JSON reproduction
    signals_json: Arc<[RawJsonString]>,
    state:        BlueprintState,
}

/// Data only used when generating from an image or a video.
struct FrameInfo<'a> {
    curr_frame:       Arc<image::DynamicImage>,
    frame_data:       FrameData<'a>,
    frames_per_cb:    u32,
    group_data_combs: Vec<LazyCombinators>,
    n_buf_frames:     u32,
    n_scaled_frames:  u32,
    next_frame:       Option<Arc<image::DynamicImage>>,
    patch_states:     Vec<GroupPatchState>,
    ticks_per_frame:  u32,
    use_delta_comp:   bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum BlueprintState {
    Start,
    Entities,
    Data,
    Wires,
    Finished,
}

struct EntityData {
    ents:       Vec<BlueprintEntity>,
    wires:      Vec<Wire>,
    next_ent_n: u32,
}
struct GroupPatchState {
    chunk_i:   u32, // which *chunk* we’re patching (not frame)
    frame_buf: Vec<image::DynamicImage>,
    sig_buf:   Vec<Vec<i32>>,
}

impl<'a> BlueprintGenerator<'a> {
    pub async fn new(
        data: Option<FrameData<'a>>,
        args: &'a BlueprintArgs,
    ) -> Result<Self, JsError> {
        let (signals, signals_json) = Self::get_signal_data(args)?;
        let gray_bits = args.image_args.grayscale_bits;
        let comb_comp = args.signal_compression;
        let time_comp_win = match comb_comp {
            Some(SignalCompression::Temporal { window }) => window,
            _ => 0,
        };
        let mut frame_info: Option<FrameInfo> = None;
        let dim: UVec2;

        if let Some(mut frame_data) = data {
            let n_scaled_frames = frame_data.total_frames() * if time_comp_win > 0 { 2 } else { 1 };
            let frames_per_cb = 32u32.checked_div(gray_bits).unwrap_or(1);
            let n_buf_frames = match time_comp_win {
                w if w > 0 => (w * args.image_args.target_fps).div_ceil(1000 * frames_per_cb),
                _ => 1,
            };
            dim = frame_data.out_dim();
            frame_info = Some(FrameInfo {
                curr_frame: frame_data.next().await?.unwrap(),
                frame_data,
                group_data_combs: Vec::new(),
                frames_per_cb,
                n_buf_frames,
                n_scaled_frames,
                next_frame: None,
                patch_states: Vec::new(),
                ticks_per_frame: 60 / args.image_args.target_fps,
                use_delta_comp: comb_comp.is_some_and(|c| c == SignalCompression::Delta),
            });
        } else {
            let w = args.custom_width.expect("Must have width or frame data.");
            let h = args.custom_height.expect("Must have height or frame data.");
            dim = uvec2(w, h);
        }
        let use_const_cbs = matches!(args.image_args.mode, Mode::Static { const_cb: true, .. });
        let max_lamp_cols_per_grp = match args.image_args.mode {
            Mode::Static { combs: false, .. } => dim.x,
            // Constant combinators can only hold up to 1000 signals
            _ => ((signals.len() as u32).min(if use_const_cbs { 255_000 } else { u32::MAX })
                / dim.y)
                .min(args.max_group_size.unwrap_or(dim.x)),
        };
        if max_lamp_cols_per_grp < 1 {
            return Err(JsError::new("Not enough signals for even one lamp column!"));
        }
        let n_groups = dim.x.div_ceil(max_lamp_cols_per_grp);

        if let Some(frame_info) = frame_info.as_mut() {
            let states_iter = (0..n_groups).map(|_| GroupPatchState {
                chunk_i:   0,
                frame_buf: Vec::with_capacity(frame_info.frames_per_cb as usize),
                sig_buf:   Vec::with_capacity(frame_info.n_buf_frames as usize),
            });
            frame_info.patch_states = states_iter.collect();
        }

        Ok(Self {
            args,
            dim,
            entity_data: const {
                EntityData {
                    ents:       Vec::new(),
                    wires:      Vec::new(),
                    next_ent_n: 0u32,
                }
            },
            frame_info,
            generate_cbs: !matches!(
                args.image_args.mode,
                Mode::Static { combs: false, .. } | Mode::Lamps | Mode::LampGrid
            ),
            max_cols_per_grp: max_lamp_cols_per_grp,
            n_groups,
            occupied: PowerPoleOccupied::new(dvec2(0.0, -f64::from(args.display_margin_y)), args),
            palette: None,
            signals,
            signals_json,
            state: BlueprintState::Start,
        })
    }

    pub fn with_palette(mut self, palette: Option<Vec<[u8; 4]>>) -> Self {
        self.palette = palette;
        self
    }

    /// Calculates the required position for the given entity to counteract the rotation of the
    /// image.
    fn transform_entity(args: &BlueprintArgs, en: &mut BlueprintEntity) {
        let mut pos = en.position;
        pos = match args.image_args.flip {
            ImageFlip::X => dvec2(-pos.x, pos.y),
            ImageFlip::Y => dvec2(pos.x, -pos.y),
            ImageFlip::Both => dvec2(-pos.x, -pos.y),
            _ => pos,
        };
        pos = match args.image_args.rotate {
            ImageRotate::Deg90 => dvec2(pos.y, -pos.x),
            ImageRotate::Deg180 => dvec2(-pos.x, -pos.y),
            ImageRotate::Deg270 => dvec2(-pos.y, pos.x),
            _ => pos,
        };

        en.position = pos;
        en.direction = en.direction.map(|v| {
            let mut v = i64::from(v);
            v += match args.image_args.rotate {
                ImageRotate::Deg90 => 12,
                ImageRotate::Deg180 => 8,
                ImageRotate::Deg270 => 4,
                _ => 0,
            };
            v += match args.image_args.flip {
                ImageFlip::X if (v + 4) % 8 == 0 => 8,
                ImageFlip::Y if v % 8 == 0 => 8,
                ImageFlip::Both => 8,
                _ => 0,
            };
            (v % 16) as u32
        });
    }

    fn get_signal_data(args: &BlueprintArgs) -> Result<SignalDataResult, JsError> {
        let mut signals = crate::blueprint::data::PROTOTYPE_DATA.with_borrow(|v| {
            let signals = &v.as_ref().expect("No signal data loaded yet!").signals;
            if matches!(
                args.image_args.mode,
                Mode::LampGrid | Mode::Lamps | Mode::Static { timer: false, .. }
            ) {
                return signals.clone();
            }
            let blacklist_sigs = [
                Arc::clone(&args.timing_signals.f),
                Arc::clone(&args.timing_signals.p),
                Arc::clone(&args.timing_signals.s),
                Arc::clone(&args.timing_signals.t),
            ]; // Disallow timing signals from being used
            let filtered = signals.iter().filter(|&v| !blacklist_sigs.contains(v));
            filtered.cloned().collect::<Box<_>>()
        });
        match args.signal_sorting {
            // Only sort by name (compressor really likes this)
            SignalSorting::Compression => {
                signals.sort_by(|a, b| a.type_.cmp(&b.type_).then_with(|| a.name.cmp(&b.name)));
            }
            // Sort by total length of the type + name + quality (actual smallest JSON)
            SignalSorting::Json => signals.sort_by_key(|s| {
                s.quality.as_ref().map_or(0, |q| match q.as_ref() {
                    "normal" => const { -(r#","quality":"normal""#.len() as i128) },
                    _ => q.len() as i128,
                }) + match s.type_.as_ref() {
                    "item" => const { -(r#","type":"item""#.len() as i128) }, // type defaults to "item" if not specified
                    _ => s.type_.len() as i128,
                } + s.name.len() as i128
            }),
            _ => {}
        }
        let signals_str = (signals.iter())
            .map(|s| serde_json::value::RawValue::from_string(serde_json::to_string(s).unwrap()))
            .collect::<Result<Arc<_>, _>>()?;
        Ok((signals, signals_str))
    }

    pub fn done(&self) -> bool {
        self.state == BlueprintState::Finished
    }

    /// Generates the next chunk of the blueprint and writes it to the given writer.
    pub async fn next_chunk(&mut self, writer: &mut impl io::Write) -> Result<(), JsError> {
        let old_state = self.state;
        match old_state {
            BlueprintState::Start => {
                write_to(writer, br#"{"blueprint":{"#)?;

                let bp_color = match self.args.image_args.grayscale_bits {
                    0 => "#8888ff",
                    _ => "#888888",
                };
                let name = self.args.name.clone();
                let name = name.map(|n| format!("{n} ({}x{})", self.dim.x, self.dim.y));
                let bp = BlueprintInner {
                    description: None,
                    icons:       Some(vec![Icon {
                        signal: Arc::from(SignalID {
                            type_:   Arc::from("item"),
                            name:    Arc::clone(&constants::LAMP),
                            quality: None,
                        }),
                        index:  1,
                    }]),
                    item:        constants::BLUEPRINT.to_string(),
                    label:       Some(match self.args.image_args.mode {
                        Mode::Full => name.unwrap_or_else(|| "GIF".to_owned()),
                        Mode::Lamps => format!("{}x{} Lamps", self.dim.x, self.dim.y),
                        Mode::LampGrid => format!("{}x{} Lamp Grid", self.dim.x, self.dim.y),
                        Mode::Static { .. } => {
                            self.args.name.clone().unwrap_or_else(|| "Image".to_owned())
                        }
                    }),
                    label_color: Color::from_hex(bp_color),
                    version:     constants::BLUEPRINT_VERSION,
                    entities:    Vec::new(),       // don't serialize
                    wires:       Some(Vec::new()), // don't serialize
                };

                write_json_trimmed_to(writer, &bp)?;
                self.state = BlueprintState::Entities;
            }
            BlueprintState::Entities => self.place_entities(writer)?,
            BlueprintState::Data => {
                if self.generate_cbs {
                    self.populate_data(writer).await?;
                } else {
                    self.state = BlueprintState::Wires;
                }
            }
            BlueprintState::Wires => self.make_wires(writer)?,
            BlueprintState::Finished => panic!("Already processed all chunks"),
        }

        // Properly delimit the JSON stream before the next chunks
        if old_state != self.state {
            match self.state {
                BlueprintState::Entities => write_to(writer, br#","entities":["#)?,
                BlueprintState::Wires => {
                    // Set description of constant combinator
                    if true
                        && let Some(incrementer) = self.entity_data.ents.first()
                        && incrementer.has_tag("incrementer".to_owned())
                    {
                        let desc = &self.get_description();
                        self.entity_data.ents[0] = incrementer.clone().with_description(desc);
                    }

                    for en in &self.entity_data.ents {
                        self.occupied.request(en.position); // Request power poles to power entities
                    }
                    let (ents, wires, _) =
                        self.occupied.generate_power_poles(self.entity_data.next_ent_n, self.args);
                    self.entity_data.ents.extend(ents);
                    self.entity_data.wires.extend(wires);

                    for en in &mut self.entity_data.ents {
                        Self::transform_entity(self.args, en);
                    }

                    // Write remaining entities. Data combinators were NOT included in
                    // self.entity_data.ents
                    for ents in self.entity_data.ents.chunks(ENCODE_CHUNK_SIZE) {
                        write_to(writer, b",")?;
                        write_json_trimmed_to(writer, &ents)?;
                    }

                    write_to(writer, br#"],"wires":["#)?;
                }
                BlueprintState::Finished => {
                    write_to(writer, b"]")?;
                    write_to(writer, br#","description":""#)?;
                    write_json_trimmed_to(writer, &self.get_description())?;
                    write_to(writer, br#"""#)?;
                    write_to(writer, b"}}")?;
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Gets the description to attach to the blueprint.
    fn get_description(&self) -> String {
        // Note: we use the `var` font for bold text as the word "var" is short and takes up less
        // space.

        let name = self.args.name.clone();
        let info = self.frame_info.as_ref();
        let dim = self.dim;

        let mut output: Vec<String> = Vec::new();
        let name = match self.args.image_args.mode {
            Mode::LampGrid => format!("{}x{} Lamp Grid", dim.x, dim.y),
            Mode::Lamps => format!("{}x{} Lamps", dim.x, dim.y),
            _ => name.unwrap_or_else(|| "Blueprint".to_owned()),
        };

        output.extend([
            format!("\n[img=item.small-lamp]  [font=compi]{name}[/font]"),
            String::new(),
            format!("\n[font=var]Dimensions[/font]:    {}x{}", dim.x, dim.y),
        ]);

        if let Some(fd) = info.map(|x| &x.frame_data) {
            let duration = format_duration(fd.curr_duration_us().div_ceil(1000));
            let color_format = match self.args.image_args.grayscale_bits {
                0 => "Full Color (RGB8)",
                1 => "Black & White",
                4 => "4-bit Grayscale (16 colors)",
                8 => "8-bit Grayscale (256 colors)",
                x => &format!("{x}"),
            };
            let n_frames = fd.curr_total_frames();
            let fps = self.args.image_args.target_fps;

            output.push(format!("\n[font=var]Color Format[/font]:   {color_format}"));
            if let Some(compression) = self.args.signal_compression {
                let compression = match compression {
                    SignalCompression::Delta => "Delta",
                    SignalCompression::Temporal { window } => &format!("Temporal ({window}ms)"),
                };
                output.push(format!("\n[font=var]Compression[/font]:  {compression}"));
            }
            output.extend([
                format!("\n[font=var]Duration[/font]:   {duration}"),
                format!("\n[font=var]Frames[/font]:     {n_frames} ({fps} FPS)"),
            ]);
        }

        let footer = "Generated using Anitorio.";
        output.push(format!("\n\n[font=default-small-semibold]{footer}[/font]"));

        output.join("")
    }

    fn place_entities(&mut self, writer: &mut impl io::Write) -> Result<(), JsError> {
        let args = self.args;
        let info = &mut self.frame_info;
        let signals = &mut self.signals;
        let ent_data = &mut self.entity_data;

        if self.generate_cbs && !matches!(args.image_args.mode, Mode::Static { timer: false, .. }) {
            let info = info.as_mut().unwrap();
            let stop = info.frame_data.total_frames() * info.ticks_per_frame;
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
                    .then(|| Arc::clone(&info.as_ref().unwrap().curr_frame)) // Pass image data to lamp generation
                    .as_ref(),
                &mut self.occupied,
                ent_data.next_ent_n,
                dvec2(f64::from(grp_left), 0.0),
                args,
            );
            let (top_left_lamp_ent_n, top_right_lamp_ent_n) = io_ents_n;
            ent_data.next_ent_n = new_next_ent_n;

            if self.generate_cbs {
                let info = info.as_mut().unwrap();
                let GenerateCombinatorsOutput {
                    metadata: ObjectGeneratorMetadata { wires, next_ent_n: new_base_ent_n },
                    data_cbs: data_combs,
                    other_ents,
                    in_ent_n,
                    out_ent_n,
                } = generate_combinators(
                    info.n_scaled_frames.div_ceil(info.frames_per_cb),
                    &self.occupied,
                    ent_data.next_ent_n,
                    dvec2(f64::from(grp_left), -f64::from(args.display_margin_y)),
                    self.max_cols_per_grp,
                    args,
                );
                ent_data.next_ent_n = new_base_ent_n;
                if group_i == 0 {
                    ent_data.wires.push([3, WIRE_OUT_R, in_ent_n, WIRE_R]); // Connect timer
                }
                info.group_data_combs.push(data_combs);
                ent_data.ents.extend(other_ents);
                ent_data.wires.extend(wires);

                (cb_in_ent_n, cb_out_ent_n) = (in_ent_n, out_ent_n);
            }

            if self.generate_cbs {
                let wires = &mut ent_data.wires;
                if let Some(top_left_lamp_ent_n) = top_left_lamp_ent_n {
                    if !matches!(args.image_args.mode, Mode::Static { timer: false, .. }) {
                        // Timing wire
                        wires.push([top_left_lamp_ent_n, WIRE_R, cb_in_ent_n, WIRE_R]);
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
                self.occupied.request(lamp.position); // Request powerage for lamps
            }
            if group_i > 0 {
                write_to(writer, b",")?;
            }

            for en in &mut grp_lamps {
                Self::transform_entity(self.args, en);
            }

            write_json_trimmed_to(writer, &grp_lamps)?;
            ent_data.wires.extend(grp_lamp_wires);
        }

        ent_data.ents.sort_by_key(|e| e.entity_number);
        self.state = BlueprintState::Data;

        Ok(())
    }

    async fn populate_data(&mut self, writer: &mut impl io::Write) -> Result<(), JsError> {
        let info = self.frame_info.as_mut().unwrap();

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
        let ticks_per_grp = ticks_per_frame * frames_per_cb;
        let mode = self.args.image_args.mode;
        let sig_p = self.args.timing_signals.p.clone();
        let mut output_data: Vec<BlueprintEntity> = Vec::new();

        let mk_cb = |start: u32, end: u32, indices: Box<[(usize, i32)]>| -> ControlBehavior {
            if matches!(mode, Mode::Static { const_cb: true, .. }) {
                let sections = indices.chunks(1000).enumerate().map(|(i, chunk)| {
                    let filters = chunk.iter().enumerate().map(|(j, &(sig_i, count))| {
                        let sig = &signals[sig_i];
                        BlueprintLogisticFilter {
                            index: j as u16 + 1,
                            type_: Arc::clone(&sig.type_),
                            name: Arc::clone(&sig.name),
                            quality: sig.quality.clone(),
                            comparator: Some(CompStr::Eq),
                            count,
                        }
                    });
                    LogisticSection {
                        index:   i as u8 + 1,
                        filters: filters.collect::<Vec<_>>(),
                    }
                });
                let sections = sections.collect::<Vec<_>>();
                let sections = LogisticSections { sections };
                ControlBehavior::ConstantCombinator { sections }
            } else {
                let conditions = vec![
                    DeciderCombinatorCondition::new(
                        Arc::clone(&sig_p),
                        start.cast_signed(),
                        CompStr::Ge,
                    ),
                    DeciderCombinatorCondition::new(
                        Arc::clone(&sig_p),
                        end.cast_signed(),
                        CompStr::Lt,
                    )
                    .with_compare_type(CompareTypeString::And),
                ];
                let decider_conditions = DeciderCombinatorCustomParameters {
                    conditions,
                    indices,
                    signal_list: Arc::clone(&self.signals_json),
                };
                ControlBehavior::DeciderCombinatorCustom { decider_conditions }
            }
        };

        loop {
            info.next_frame = frame_data.next().await?;

            let frame = &info.curr_frame;
            let is_last_frame = info.next_frame.is_none();

            for group_i in 0..n_groups {
                if matches!(
                    self.args.image_args.mode,
                    Mode::Static { const_cb: true, .. }
                ) {
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
                let frame_sigs: Vec<i32>;

                let grp_left = group_i * max_cols_per_grp;
                let grp_right = ((group_i + 1) * max_cols_per_grp).min(self.dim.x);

                let img = frame.crop_imm(grp_left, 0, grp_right - grp_left, self.dim.y);
                let target_outputs_len = (img.width() * img.height()) as usize;

                if gray_bits > 0 {
                    state.frame_buf.push(img);
                    if state.frame_buf.len() < frames_per_cb as usize && !is_last_frame {
                        continue; // wait until we have a full chunk for this group
                    }
                    frame_sigs = grayscale_frames_to_outputs(&state.frame_buf, gray_bits)?;
                    state.frame_buf.clear();
                } else {
                    frame_sigs = color_frame_to_outputs(&img);
                }

                if frame_sigs.len() != target_outputs_len {
                    return Err(JsError::new(&format!(
                        "Outputs length ({}) does not match frame size ({target_outputs_len}).",
                        frame_sigs.len(),
                    )));
                }

                if use_delta_comp && state.sig_buf.is_empty() {
                    state.sig_buf.push(vec![0i32; target_outputs_len]); // Populate first frame with zeros
                } else if !use_delta_comp {
                    state.sig_buf.push(frame_sigs.clone()); // If we're using delta compression we only compare aginst the previous frame
                }

                if state.sig_buf.len() < n_buf_frames as usize && !is_last_frame {
                    continue; // Accumulate until we have `compression_level` outputs,
                }

                if use_delta_comp {
                    let prev_outputs = &state.sig_buf[0];
                    let target_outputs = (frame_sigs.iter().enumerate())
                        .filter_map(|(i, &v)| {
                            let v = v.wrapping_sub(prev_outputs[i]);
                            (v != 0).then_some((i, v))
                        })
                        .collect::<Box<_>>();
                    if !target_outputs.is_empty() {
                        let en = data_combs.next();
                        let range = uvec2(1, 2) + (ticks_per_grp * state.chunk_i);
                        let cb = mk_cb(range.x, range.y, target_outputs);
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
                            let target_i = base_chunk_i + cb_i as u32;
                            let en = data_combs.next();
                            let range = (uvec2(0, 1) + target_i) * ticks_per_grp;
                            let cb = mk_cb(range.x, range.y, target_outputs);
                            output_data.push(en.with_control_behavior(cb));
                        }
                    }

                    // Handle unchanged pixels
                    if state.sig_buf.len() > 1
                        && let target_outputs = (changed_mask.iter().enumerate())
                            .filter_map(|(i, &changed)| (!changed).then_some((i, outputs_0[i])))
                            .collect::<Box<_>>()
                        && !target_outputs.is_empty()
                    {
                        let en = data_combs.next();
                        let range = ticks_per_grp
                            * uvec2(base_chunk_i, base_chunk_i + state.sig_buf.len() as u32);
                        let cb = mk_cb(range.x, range.y, target_outputs);
                        output_data.push(en.with_control_behavior(cb));
                    }
                    state.sig_buf.clear();
                }
                state.chunk_i += 1;
            }

            // Note: Always advance before checking if we have max entities
            if info.next_frame.is_some() {
                info.curr_frame = info.next_frame.take().unwrap();
            } else {
                self.state = BlueprintState::Wires;
                break;
            }
            if output_data.len() >= ENCODE_CHUNK_SIZE {
                break;
            }
        }

        for en in &mut output_data {
            self.occupied.request(en.position); // Request powerage for generated combinators
            Self::transform_entity(self.args, en);
        }
        if !output_data.is_empty() {
            write_to(writer, b",")?;

            if self.args.low_mem_usage {
                let mut buffered = io::BufWriter::with_capacity(512 * 1024, writer);
                let mut trimmed = crate::ioutil::TrimmedWriter::new(&mut buffered);
                serde_json::to_writer(&mut trimmed, &output_data)?;
                trimmed.finish()?;
            } else {
                write_json_trimmed_to(writer, &output_data)?;
            }
        }

        Ok(())
    }

    fn make_wires(&mut self, writer: &mut impl io::Write) -> Result<(), JsError> {
        let all_ents = &mut self.entity_data.ents;
        let all_wires = &mut self.entity_data.wires;
        let n_wires = all_wires.len();

        // Swap all wires if requested (default uses red wires, so swap all for green).
        // Circuit network filters are already handled.
        if !self.args.prefer_green_wires {
            invert_wires(all_ents, all_wires);
        }
        for (i, wires) in all_wires.chunks(WIRE_ENCODE_CHUNK_SIZE).enumerate() {
            let display_i = (i + 1) * WIRE_ENCODE_CHUNK_SIZE;
            set_progress(
                0.90,
                1.00,
                display_i as f64 / n_wires as f64,
                &format!("Generating wires ({display_i} / {n_wires})..."),
            );
            if i > 0 {
                write_to(writer, b",")?;
            }
            write_json_trimmed_to(writer, wires)?;
        }
        self.state = BlueprintState::Finished;

        Ok(())
    }
}

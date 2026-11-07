use super::prelude::*;
use crate::prelude::*;
use glam::{DVec2, IVec2, dvec2, ivec2};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

/// Tracks cells occupied by power poles.
pub struct PowerPoleOccupied {
    bounding_box: DVec2,
    coverage: f64,
    ref_entity: BlueprintEntity,
    requests: BTreeSet<(i32, i32)>,
    start_pos: DVec2,
}
impl PowerPoleOccupied {
    pub fn new(mut start_pos: DVec2, args: &BlueprintArgs) -> Self {
        let quality = args.power_pole_quality.clone();
        let quality = quality.filter(|q| *q != *constants::DEFAULT_QUAL_KEY);
        let name = args.power_pole_name.as_ref();
        let name = name.map_or_else(|| constants::DEFAULT_POWER_POLE_NAME, |v| v.as_str());

        let data = crate::blueprint::data::PROTOTYPE_DATA.with_borrow(|data| {
            let data = &data.as_ref().expect("No power pole data loaded yet!");
            let poles = &data.electric_poles;
            let p = poles.iter().find(|p| *p.name == *name);
            p.unwrap_or_else(|| panic!("No power pole with name '{name}'!"))
                .clone()
        });
        let bounding_box = dvec2(f64::from(data.tile_width), f64::from(data.tile_height));
        let dists = &data.supply_area_distances;
        let half_cov = dists.iter().find(|d| quality == Some(d.quality.clone()));
        let cov = 2.0 * half_cov.unwrap_or(&dists[0]).supply_area_distance;

        start_pos += bounding_box * const { dvec2(-0.5, 0.5) };

        Self {
            bounding_box,
            coverage: cov * f64::from(i32::from(args.include_power_poles)),
            ref_entity: BlueprintEntity::new(0, Arc::from(name), start_pos).with_quality(quality),
            requests: BTreeSet::new(),
            start_pos,
        }
    }

    /// Generates power pole entities and wires for powering the blueprint.
    ///
    /// # Arguments
    ///
    /// * `lamp_dim` - Width and height of the lamp grid.
    /// * `n_frames` - Number of frames (affects vertical coverage).
    /// * `base_ent_n` - Starting entity number.
    /// * `args` - Args for generating the blueprint.
    ///
    /// # Returns
    ///
    /// A tuple with power pole entities, their wires, occupied grid cells, and the next entity
    /// number.
    pub fn generate_power_poles(
        &self,
        base_ent_n: u32,
        args: &BlueprintArgs,
    ) -> (Vec<BlueprintEntity>, Vec<Wire>, u32) {
        let mut poles: Vec<BlueprintEntity> = Vec::new();
        let mut wires: Vec<Wire> = Vec::new();
        let mut curr_ent_n = base_ent_n;
        let cov = self.coverage;

        if cov == 0.0 {
            return (Vec::new(), wires, base_ent_n);
        }

        let horiz = args.prefer_horizontal_wires ^ matches!(args.image_args.rotate, Deg90 | Deg270);
        let mut pole_pos_map = BTreeMap::<(i32, i32), u32>::new();

        for &req in {
            let mut requests: Vec<_> = self.requests.iter().collect();
            requests.sort_by(|&a, &b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
            requests
        } {
            pole_pos_map.insert(req, curr_ent_n);
            curr_ent_n += 1;
        }

        let offset_below = ivec2(0, cov as i32);
        let offset_left = ivec2(-cov as i32, 0);

        for (&pos, &ent_n) in &pole_pos_map {
            let mut en = self.ref_entity.clone();
            let pos = IVec2::from(pos);

            if (horiz || !pole_pos_map.contains_key(&(pos + offset_below).into()))
                && let Some(&prev) = pole_pos_map.get(&(pos + offset_left).into())
            {
                wires.push([prev, WIRE_C, ent_n, WIRE_C]);
            }
            if (!horiz || !pole_pos_map.contains_key(&(pos + offset_left).into()))
                && let Some(&prev) = pole_pos_map.get(&(pos + offset_below).into())
            {
                wires.push([prev, WIRE_C, ent_n, WIRE_C]);
            }

            en.entity_number = ent_n;
            en.position += pos.as_dvec2();
            poles.push(en);
        }

        (poles, wires, curr_ent_n)
    }

    /// Tests if it is possible to place an entity at the given point, without overlapping with a
    /// power pole.
    ///
    /// # Returns
    ///
    /// A `bool` indicating whether the power pole would NOT occupy the given point.
    pub fn test(&self, point: DVec2) -> bool {
        self.test_center(point).0
    }

    fn test_center(&self, mut point: DVec2) -> (bool, DVec2) {
        if self.coverage == 0.0 {
            return (true, DVec2::ZERO);
        }
        point -= self.start_pos;

        let center = (point / self.coverage).round() * self.coverage;
        let delta = (point - center).abs();
        let did_hit = delta.x >= self.bounding_box.x / 2.0 || delta.y >= self.bounding_box.y / 2.0;

        (did_hit, center)
    }

    /// Requests that a power pole be placed to power the given point.
    ///
    /// # Returns
    ///
    /// A `bool` indicating whether the power pole would NOT occupy the given point.
    pub fn request(&mut self, point: DVec2) -> bool {
        let (hit, center) = self.test_center(point);
        self.requests.insert(center.as_ivec2().into());
        hit
    }
}

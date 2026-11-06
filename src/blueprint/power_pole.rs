use super::prelude::*;
use crate::prelude::*;
use glam::{Vec2, vec2};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

#[derive(Clone, Copy, Debug)]
struct OrderedVec2(pub Vec2);
impl Ord for OrderedVec2 {
    #[inline]
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        let (a, b) = (self.0, other.0);
        a.y.total_cmp(&b.y).then_with(|| a.x.total_cmp(&b.x))
    }
}
impl PartialOrd for OrderedVec2 {
    #[inline]
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl PartialEq for OrderedVec2 {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other).is_eq()
    }
}
impl Eq for OrderedVec2 {}

/// Tracks cells occupied by power poles.
pub struct PowerPoleOccupied {
    bounding_box: Vec2,
    coverage: f32,
    ref_entity: BlueprintEntity,
    requests: BTreeSet<OrderedVec2>,
    start_pos: Vec2,
}
impl PowerPoleOccupied {
    pub fn new(mut start_pos: Vec2, args: &BlueprintArgs) -> Self {
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
        let bounding_box = vec2(data.tile_width as f32, data.tile_height as f32);
        let dists = &data.supply_area_distances;
        let half_cov = dists.iter().find(|d| quality == Some(d.quality.clone()));
        let cov = 2.0 * half_cov.unwrap_or(&dists[0]).supply_area_distance;

        start_pos += bounding_box * vec2(-0.5, 0.5);

        Self {
            bounding_box,
            coverage: (cov * f64::from(i32::from(args.include_power_poles))) as f32,
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
            return (poles, wires, base_ent_n);
        }

        let horiz = args.prefer_horizontal_wires ^ matches!(args.image_args.rotate, Deg90 | Deg270);
        let mut pole_map = BTreeMap::<OrderedVec2, u32>::new();

        let mut requests: Vec<_> = self.requests.iter().collect();
        requests.sort_by(|OrderedVec2(a), OrderedVec2(b)| {
            a.y.total_cmp(&b.y).then_with(|| a.x.total_cmp(&b.x))
        });

        // Create all power poles first. This lets us determine the topology
        // from the actual generated positions without making assumptions
        // about the shape of the layout.
        for req in &self.requests {
            let mut en = self.ref_entity.clone();
            en.entity_number = curr_ent_n;
            en.position += req.0;

            pole_map.insert(OrderedVec2(en.position), curr_ent_n);
            poles.push(en);
            curr_ent_n += 1;
        }

        for pole in &poles {
            let pos = pole.position;
            if (horiz || !pole_map.contains_key(&OrderedVec2(pos + vec2(0.0, cov))))
                && let Some(&prev) = pole_map.get(&OrderedVec2(pos + vec2(-cov, 0.0)))
            {
                wires.push([prev, WIRE_C, pole.entity_number, WIRE_C]);
            }
            if (!horiz || !pole_map.contains_key(&OrderedVec2(pos + vec2(-cov, 0.0))))
                && let Some(&prev) = pole_map.get(&OrderedVec2(pos + vec2(0.0, cov)))
            {
                wires.push([prev, WIRE_C, pole.entity_number, WIRE_C]);
            }
        }

        (poles, wires, curr_ent_n)
    }

    /// Tests if it is possible to place an entity at the given point, without overlapping with a
    /// power pole.
    ///
    /// # Returns
    ///
    /// A `bool` indicating whether the power pole would NOT occupy the given point.
    pub fn test(&self, point: Vec2) -> bool {
        self.test_center(point).0
    }

    fn test_center(&self, mut point: Vec2) -> (bool, Vec2) {
        if self.coverage == 0.0 {
            return (true, Vec2::ZERO);
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
    pub fn request(&mut self, point: Vec2) -> bool {
        let (hit, center) = self.test_center(point);
        self.requests.insert(OrderedVec2(center));
        hit
    }
}

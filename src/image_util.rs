/// Copied from image crate
pub fn resize_dimensions(
    width: u32,
    height: u32,
    nwidth: u32,
    nheight: u32,
    fill: bool,
) -> (u32, u32) {
    let wratio = nwidth as f64 / width as f64;
    let hratio = nheight as f64 / height as f64;

    let ratio = if fill {
        f64::max(wratio, hratio)
    } else {
        f64::min(wratio, hratio)
    };

    let nw = std::cmp::max((width as f64 * ratio).round() as u64, 1);
    let nh = std::cmp::max((height as f64 * ratio).round() as u64, 1);

    if nw > u64::from(u32::MAX) {
        let ratio = u32::MAX as f64 / width as f64;
        (
            u32::MAX,
            std::cmp::max((height as f64 * ratio).round() as u32, 1),
        )
    } else if nh > u64::from(u32::MAX) {
        let ratio = u32::MAX as f64 / height as f64;
        (
            std::cmp::max((width as f64 * ratio).round() as u32, 1),
            u32::MAX,
        )
    } else {
        (nw as u32, nh as u32)
    }
}

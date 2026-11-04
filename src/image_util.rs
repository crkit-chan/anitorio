/// Copied from image crate
pub const fn resize_dimensions(
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

    let nw = u64_max((width as f64 * ratio).round() as u64, 1);
    let nh = u64_max((height as f64 * ratio).round() as u64, 1);

    if nw > u32::MAX as u64 {
        let ratio = u32::MAX as f64 / width as f64;
        (u32::MAX, u32_max((height as f64 * ratio).round() as u32, 1))
    } else if nh > u32::MAX as u64 {
        let ratio = u32::MAX as f64 / height as f64;
        (u32_max((width as f64 * ratio).round() as u32, 1), u32::MAX)
    } else {
        (nw as u32, nh as u32)
    }
}

const fn u64_max(a: u64, b: u64) -> u64 {
    if a > b { a } else { b }
}
const fn u32_max(a: u32, b: u32) -> u32 {
    if a > b { a } else { b }
}

use crate::progress_window::layout::Placement;

pub(super) fn placement(
    work: &Placement,
    scale: f64,
    logical_height: f64,
    obstacle: Option<&Placement>,
) -> Placement {
    let scale = if scale.is_finite() && scale > 0.0 {
        scale.clamp(0.5, 8.0)
    } else {
        1.0
    };
    let margin = ((12.0 * scale).round() as u32)
        .min(work.width.saturating_sub(1) / 2)
        .min(work.height.saturating_sub(1) / 2);
    let width = ((352.0 * scale).round() as u32)
        .min(work.width.saturating_sub(margin * 2))
        .max(1);
    let height = ((logical_height.clamp(64.0, 480.0) * scale).ceil() as u32)
        .min(work.height.saturating_sub(margin * 2))
        .max(1);
    let right = i64::from(work.x) + i64::from(work.width) - i64::from(margin);
    let bottom = i64::from(work.y) + i64::from(work.height) - i64::from(margin);
    let min_x = i64::from(work.x) + i64::from(margin);
    let min_y = i64::from(work.y) + i64::from(margin);
    let mut x = right - i64::from(width);
    let mut y = bottom - i64::from(height);
    if let Some(other) = obstacle {
        let ox = i64::from(other.x);
        let oy = i64::from(other.y);
        let oright = ox + i64::from(other.width);
        let obottom = oy + i64::from(other.height);
        if x < oright && right > ox && y < obottom && bottom > oy {
            let above = oy - i64::from(margin) - i64::from(height);
            let beside = ox - i64::from(margin) - i64::from(width);
            if above >= min_y {
                y = above;
            } else if beside >= min_x {
                x = beside;
            } else {
                y = min_y;
            }
        }
    }
    let coordinate = |value: i64| value.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32;
    Placement {
        x: coordinate(x.max(min_x)),
        y: coordinate(y.max(min_y)),
        width,
        height,
    }
}

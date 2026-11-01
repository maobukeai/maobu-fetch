//! Work-area geometry is physical pixels; only desired dimensions use logical pixels.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Placement {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

pub(super) fn placement(
    left: i32,
    top: i32,
    work_width: u32,
    work_height: u32,
    scale: f64,
    count: usize,
    expanded: bool,
    revealed: bool,
) -> Placement {
    placement_with_completions(
        left,
        top,
        work_width,
        work_height,
        scale,
        count,
        expanded,
        revealed,
        false,
        false,
    )
}

pub(super) fn placement_with_completions(
    left: i32,
    top: i32,
    work_width: u32,
    work_height: u32,
    scale: f64,
    count: usize,
    expanded: bool,
    revealed: bool,
    mixed: bool,
    completed_only: bool,
) -> Placement {
    let scale = if scale.is_finite() && scale > 0.0 {
        scale.clamp(0.5, 8.0)
    } else {
        1.0
    };
    let desired_width = (224.0 * scale).round() as u32;
    let rows = count.min(if expanded { 6 } else { 3 }).max(1);
    // The 28px summary stays at the bottom as details unfold above it. Resize
    // the HWND itself, rather than leaving a large transparent click rectangle.
    let logical_height = if revealed {
        // Completed-only details share the summary's stable action bar instead
        // of repeating the old 24px footer. Mixed/active views keep that footer.
        let base = if completed_only && !mixed { 58 } else { 82 };
        base + rows * 36 + if mixed { 24 } else { 0 }
    } else {
        28
    };
    let desired_height = (logical_height as f64 * scale).round() as u32;
    let work_width = work_width.max(1);
    let work_height = work_height.max(1);
    let margin = ((12.0 * scale).round() as u32)
        .min(work_width.saturating_sub(1) / 2)
        .min(work_height.saturating_sub(1) / 2);
    let width = desired_width
        .min(work_width.saturating_sub(2 * margin))
        .max(1);
    let height = desired_height
        .min(work_height.saturating_sub(2 * margin))
        .max(1);
    let coordinate = |origin: i32, extent: u32, size: u32| {
        (i64::from(origin) + i64::from(extent) - i64::from(margin) - i64::from(size))
            .clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
    };
    Placement {
        x: coordinate(left, work_width, width),
        y: coordinate(top, work_height, height),
        width,
        height,
    }
}

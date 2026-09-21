use crate::ir::{DiagramKind, EdgeArrowhead};

/// Distance the explicit arrowhead extends past the routed path endpoint.
///
/// This belongs in shared edge geometry instead of the SVG renderer because the
/// layout router must shorten routed paths before arrowheads are painted; the
/// renderer uses this same distance for the final head geometry.
pub(crate) fn arrowhead_inset(kind: DiagramKind, arrow_kind: Option<EdgeArrowhead>) -> f32 {
    match kind {
        DiagramKind::Class => match arrow_kind {
            Some(EdgeArrowhead::OpenTriangle) => 17.0,
            Some(EdgeArrowhead::ClassDependency) => 5.0,
            _ => 4.0,
        },
        _ => 0.0,
    }
}

/// Apply start/end arrowhead insets to an already routed path.
///
/// The operation is deliberately conservative: endpoints are only moved when the
/// adjacent segment is longer than the requested inset. Very short segments are
/// left intact to avoid reversing or collapsing route geometry.
pub(crate) fn apply_endpoint_insets(
    mut path: Vec<(f32, f32)>,
    start_inset: f32,
    end_inset: f32,
) -> Vec<(f32, f32)> {
    if start_inset > 0.0 && path.len() >= 2 {
        let (sx, sy) = path[0];
        let (nx, ny) = path[1];
        let dx = sx - nx;
        let dy = sy - ny;
        let len = (dx * dx + dy * dy).sqrt();
        if len > start_inset {
            let r = start_inset / len;
            path[0] = (sx - dx * r, sy - dy * r);
        }
    }

    if end_inset > 0.0 && path.len() >= 2 {
        let n = path.len();
        let (px, py) = path[n - 2];
        let (ex, ey) = path[n - 1];
        let dx = ex - px;
        let dy = ey - py;
        let len = (dx * dx + dy * dy).sqrt();
        if len > end_inset {
            let r = end_inset / len;
            path[n - 1] = (ex - dx * r, ey - dy * r);
        }
    }

    path
}

/// Angle, in degrees, of the path tangent at the requested endpoint.
///
/// Arrowheads must follow the final routed segment, not a guessed side of the
/// endpoint node. This helper is shared so renderers and future validators agree
/// on marker direction.
pub(crate) fn edge_endpoint_angle(points: &[(f32, f32)], start: bool) -> f32 {
    if points.len() < 2 {
        return 0.0;
    }
    let (p0, p1) = if start {
        (points[0], points[1])
    } else {
        (points[points.len() - 2], points[points.len() - 1])
    };
    let dx = p1.0 - p0.0;
    let dy = p1.1 - p0.1;
    dy.atan2(dx).to_degrees()
}

/// Conservative world-space bounds of the explicit endpoint geometry. Labels
/// must avoid these shapes even though the carrying path is allowed underneath.
pub(crate) fn endpoint_obstacles(
    edge: &crate::layout::EdgeLayout,
    kind: DiagramKind,
) -> Vec<(f32, f32, f32, f32)> {
    use crate::ir::EdgeDecoration::*;
    let mut obstacles = Vec::new();
    for at_start in [true, false] {
        let point = if at_start {
            edge.points.first()
        } else {
            edge.points.last()
        };
        let Some(&point) = point else {
            continue;
        };
        let tangent = edge_endpoint_angle(&edge.points, at_start);
        let (arrow, arrow_kind, decoration) = if at_start {
            (
                edge.arrow_start,
                edge.arrow_start_kind,
                edge.start_decoration,
            )
        } else {
            (edge.arrow_end, edge.arrow_end_kind, edge.end_decoration)
        };
        let pad = edge.override_style.stroke_width.unwrap_or(2.0) / 2.0 + 2.0;
        if arrow {
            let (rect, angle) = if kind == DiagramKind::Requirement && at_start {
                ((0.0, -10.0, 20.0, 20.0), tangent)
            } else {
                let rect = match kind {
                    DiagramKind::Requirement => (-20.0, -10.0, 20.0, 20.0),
                    DiagramKind::Class => {
                        let inset = arrowhead_inset(kind, arrow_kind);
                        (inset - 17.0, -6.0, 17.0, 12.0)
                    }
                    _ => (-10.0, -6.0, 10.0, 12.0),
                };
                (rect, tangent + if at_start { 180.0 } else { 0.0 })
            };
            obstacles.push(rotated_bounds(point, angle, rect, pad));
        }
        if let Some(decoration) = decoration {
            let (rect, reverse) = match decoration {
                Circle | Cross => ((-5.0, -5.0, 10.0, 10.0), false),
                Diamond | DiamondFilled => ((0.0, -6.0, 18.0, 12.0), !at_start),
                CrowsFootOne | CrowsFootZeroOne | CrowsFootMany | CrowsFootZeroMany => {
                    ((0.0, -6.0, 20.0, 12.0), !at_start)
                }
            };
            obstacles.push(rotated_bounds(
                point,
                tangent + if reverse { 180.0 } else { 0.0 },
                rect,
                pad,
            ));
        }
    }
    obstacles
}

fn rotated_bounds(
    origin: (f32, f32),
    angle: f32,
    rect: (f32, f32, f32, f32),
    pad: f32,
) -> (f32, f32, f32, f32) {
    let (sin, cos) = angle.to_radians().sin_cos();
    let mut min = (f32::INFINITY, f32::INFINITY);
    let mut max = (f32::NEG_INFINITY, f32::NEG_INFINITY);
    for x in [rect.0, rect.0 + rect.2] {
        for y in [rect.1, rect.1 + rect.3] {
            let p = (origin.0 + x * cos - y * sin, origin.1 + x * sin + y * cos);
            min.0 = min.0.min(p.0);
            min.1 = min.1.min(p.1);
            max.0 = max.0.max(p.0);
            max.1 = max.1.max(p.1);
        }
    }
    (
        min.0 - pad,
        min.1 - pad,
        max.0 - min.0 + 2.0 * pad,
        max.1 - min.1 + 2.0 * pad,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_insets_shorten_long_segments_without_collapsing_short_segments() {
        let path = vec![(0.0, 0.0), (10.0, 0.0), (20.0, 0.0)];
        let inset = apply_endpoint_insets(path, 3.0, 4.0);
        assert_eq!(inset[0], (3.0, 0.0));
        assert_eq!(inset[2], (16.0, 0.0));

        let short = vec![(0.0, 0.0), (2.0, 0.0)];
        assert_eq!(apply_endpoint_insets(short.clone(), 3.0, 3.0), short);
    }

    #[test]
    fn endpoint_angle_uses_requested_endpoint_tangent() {
        let points = vec![(0.0, 0.0), (10.0, 0.0), (20.0, 10.0)];
        assert_eq!(edge_endpoint_angle(&points, true), 0.0);
        assert_eq!(edge_endpoint_angle(&points, false), 45.0);
    }
}

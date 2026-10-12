//! Hardware-style painting for the EQ editor: knobs with a knurled skirt and a metal cap, screws,
//! backlit switches and glass over the displays. Everything is drawn in code from the theme's
//! tokens; there are no bitmaps. Light comes from the top left and never moves, so a
//! knob's highlight stays put while its skirt and pointer turn.

use crate::theme::Tokens;
use egui::{Color32, Mesh, Painter, Pos2, Rect, Stroke, Vec2, pos2, vec2};

/// Segments in a painted circle.
const SEGMENTS: usize = 64;

/// `c` lightened (`by` > 0) towards white or darkened (`by` < 0) towards black.
fn shade(c: Color32, by: f32) -> Color32 {
    let to = if by >= 0.0 { 255.0 } else { 0.0 };
    let k = by.abs().clamp(0.0, 1.0);
    let ch = |v: u8| (f32::from(v) + (to - f32::from(v)) * k).round() as u8;
    Color32::from_rgba_unmultiplied(ch(c.r()), ch(c.g()), ch(c.b()), c.a())
}

fn white(alpha: f32) -> Color32 {
    Color32::from_white_alpha((alpha.clamp(0.0, 1.0) * 255.0) as u8)
}

fn black(alpha: f32) -> Color32 {
    Color32::from_black_alpha((alpha.clamp(0.0, 1.0) * 255.0) as u8)
}

/// A convex outline filled as a fan from `center`, each vertex coloured by `color` (the centre
/// gets `center_color`). Gives linear, radial and conic gradients from one routine.
fn fan(p: &Painter, center: Pos2, center_color: Color32, outline: &[Pos2], color: impl Fn(Pos2, usize) -> Color32) {
    if outline.len() < 3 || !center.is_finite() {
        return;
    }
    let mut mesh = Mesh::default();
    mesh.colored_vertex(center, center_color);
    for (i, &q) in outline.iter().enumerate() {
        mesh.colored_vertex(q, color(q, i));
    }
    let n = outline.len() as u32;
    for i in 0..n {
        mesh.add_triangle(0, 1 + i, 1 + (i + 1) % n);
    }
    p.add(mesh);
}

fn circle_points(c: Pos2, r: f32) -> Vec<Pos2> {
    (0..SEGMENTS).map(|i| c + Vec2::angled(i as f32 / SEGMENTS as f32 * std::f32::consts::TAU) * r).collect()
}

/// A rounded rectangle's outline, clockwise from the top left corner.
fn rounded_points(r: Rect, radius: f32) -> Vec<Pos2> {
    let radius = radius.min(r.width() / 2.0).min(r.height() / 2.0).max(0.0);
    let corners = [
        (pos2(r.left() + radius, r.top() + radius), std::f32::consts::PI),
        (pos2(r.right() - radius, r.top() + radius), std::f32::consts::PI * 1.5),
        (pos2(r.right() - radius, r.bottom() - radius), 0.0),
        (pos2(r.left() + radius, r.bottom() - radius), std::f32::consts::FRAC_PI_2),
    ];
    let mut pts = Vec::with_capacity(4 * 7);
    for (c, a0) in corners {
        for i in 0..=6 {
            pts.push(c + Vec2::angled(a0 + i as f32 / 6.0 * std::f32::consts::FRAC_PI_2) * radius);
        }
    }
    pts
}

/// Linear blend from `a` (at 0) to `b` (at 1).
fn lerp(a: Color32, b: Color32, f: f32) -> Color32 {
    let f = f.clamp(0.0, 1.0);
    let ch = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * f).round() as u8;
    Color32::from_rgba_premultiplied(ch(a.r(), b.r()), ch(a.g(), b.g()), ch(a.b(), b.b()), ch(a.a(), b.a()))
}

/// A rounded rectangle with a vertical gradient from `top` to `bottom`.
pub fn vgradient(p: &Painter, r: Rect, radius: f32, top: Color32, bottom: Color32) {
    let pts = rounded_points(r, radius);
    let at = |q: Pos2| lerp(top, bottom, (q.y - r.top()) / r.height().max(1.0));
    fan(p, r.center(), at(r.center()), &pts, |q, _| at(q));
}

/// The faceplate behind the whole editor: brushed metal (fine horizontal streaks over a soft
/// gradient), a bevelled edge and a screw in each corner. `margin` is the rim the screws sit in.
pub fn faceplate(p: &Painter, r: Rect, margin: f32) {
    let t = Tokens::current();
    vgradient(p, r, 6.0, t.eq_plate[0], t.eq_plate[1]);
    // Brushed streaks: a fixed pseudo-random pattern, so it doesn't shimmer between frames.
    let mut seed = 0x9e37_79b9u32;
    let mut y = r.top() + 1.0;
    while y < r.bottom() - 1.0 {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        let a = (seed % 100) as f32 / 100.0;
        let col = if seed & 1 == 0 { white(0.012 + a * 0.02) } else { black(0.02 + a * 0.03) };
        p.line_segment([pos2(r.left() + 2.0, y), pos2(r.right() - 2.0, y)], Stroke::new(1.0, col));
        y += 1.0 + (seed % 3) as f32 * 0.5;
    }
    bevel(p, r, 6.0, false);
    let d = margin / 2.0;
    for (i, c) in
        [pos2(r.left() + d, r.top() + d), pos2(r.right() - d, r.top() + d), pos2(r.left() + d, r.bottom() - d), pos2(r.right() - d, r.bottom() - d)]
            .into_iter()
            .enumerate()
    {
        screw(p, c, (margin * 0.32).clamp(3.0, 7.0), 0.35 + i as f32 * 0.9);
    }
}

/// Light top-left edge and dark bottom-right edge; `inset` swaps them (a recess, not a bump).
pub fn bevel(p: &Painter, r: Rect, radius: f32, inset: bool) {
    let (a, b) = if inset { (black(0.45), white(0.10)) } else { (white(0.16), black(0.40)) };
    let pts = rounded_points(r.shrink(0.5), radius);
    let n = pts.len();
    for i in 0..n {
        let (q0, q1) = (pts[i], pts[(i + 1) % n]);
        // Edges facing up or left catch the light.
        let normal = (q1 - q0).rot90().normalized();
        let lit = (-normal.x - normal.y) * std::f32::consts::FRAC_1_SQRT_2;
        let col = if lit >= 0.0 { lerp(Color32::TRANSPARENT, a, lit) } else { lerp(Color32::TRANSPARENT, b, -lit) };
        p.line_segment([q0, q1], Stroke::new(1.0, col));
    }
}

/// A raised section panel: soft gradient over the section tone and a bevelled edge.
pub fn section(p: &Painter, r: Rect) {
    let t = Tokens::current();
    // Drop shadow under the panel.
    for i in 1..=3 {
        let s = r.translate(vec2(0.0, i as f32)).expand(i as f32 * 0.5);
        p.rect_filled(s, 5.0, black(0.07));
    }
    vgradient(p, r, 4.0, t.eq_section[0], t.eq_section[1]);
    bevel(p, r, 4.0, false);
}

/// A recessed display (a value box, the graph): darker inside with an inner shadow.
pub fn well(p: &Painter, r: Rect, radius: f32, fill: Color32) {
    p.rect_filled(r, radius, fill);
    for i in 0..3 {
        let a = 0.22 - i as f32 * 0.07;
        p.line_segment(
            [pos2(r.left() + radius, r.top() + 0.5 + i as f32), pos2(r.right() - radius, r.top() + 0.5 + i as f32)],
            Stroke::new(1.0, black(a)),
        );
    }
    bevel(p, r, radius, true);
}

/// Glass over a display: a soft sheen across the upper part and a bright rim at the top edge.
pub fn glass(p: &Painter, r: Rect, radius: f32) {
    let sheen = Rect::from_min_max(r.min, pos2(r.right(), r.top() + r.height() * 0.45));
    vgradient(p, sheen.shrink2(vec2(1.0, 0.0)), radius, white(0.10), white(0.0));
    // A diagonal glare, as if a window were reflected in the glass.
    let w = r.width();
    let x0 = r.left() + w * 0.55;
    let glare =
        [pos2(x0, r.top() + 1.0), pos2(x0 + w * 0.18, r.top() + 1.0), pos2(x0 + w * 0.05, r.bottom() - 1.0), pos2(x0 - w * 0.13, r.bottom() - 1.0)];
    let mut mesh = Mesh::default();
    for (q, a) in glare.iter().zip([0.05, 0.05, 0.0, 0.0]) {
        mesh.colored_vertex(*q, white(a));
    }
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    p.with_clip_rect(r.shrink(1.0)).add(mesh);
    p.line_segment([pos2(r.left() + radius, r.top() + 1.0), pos2(r.right() - radius, r.top() + 1.0)], Stroke::new(1.0, white(0.18)));
}

/// A slotted screw head of radius `r` at `c`, its slot at `angle` (radians).
pub fn screw(p: &Painter, c: Pos2, r: f32, angle: f32) {
    let t = Tokens::current();
    let metal = t.eq_screw;
    // Countersink: a dark ring the head sits in.
    p.circle_filled(c + vec2(0.0, 0.6), r + 1.2, black(0.35));
    let pts = circle_points(c, r);
    let light = Vec2::angled(-std::f32::consts::FRAC_PI_4 * 3.0);
    fan(p, c, metal, &pts, |q, _| {
        let f = (q - c).normalized().dot(light);
        shade(metal, f * 0.45)
    });
    let d = Vec2::angled(angle) * r * 0.8;
    p.line_segment([c - d, c + d], Stroke::new((r * 0.35).max(1.0), shade(metal, -0.6)));
    p.line_segment([c - d + vec2(0.0, 0.8), c + d + vec2(0.0, 0.8)], Stroke::new(0.8, white(0.25)));
}

/// A knob of radius `rad` at `c`: shadow, a knurled skirt that turns with the value, a brushed
/// metal cap, a pointer at `angle` (radians) in `accent`, and a fixed highlight.
pub fn knob(p: &Painter, c: Pos2, rad: f32, angle: f32, accent: Color32, hovered: bool) {
    let t = Tokens::current();
    // Shadow, falling down and right.
    for i in 1..=4 {
        let f = i as f32;
        p.circle_filled(c + vec2(f * 0.4, f * 0.8), rad + f * 0.6, black(0.10));
    }
    // Skirt: a dark ring, lit from the top left, with knurls that turn with the knob.
    let skirt = t.eq_knob_skirt;
    let light = Vec2::angled(-std::f32::consts::FRAC_PI_4 * 3.0);
    fan(p, c, skirt, &circle_points(c, rad), |q, _| shade(skirt, (q - c).normalized().dot(light) * 0.35));
    let knurls = 32;
    for i in 0..knurls {
        let a = angle + i as f32 / knurls as f32 * std::f32::consts::TAU;
        let dir = Vec2::angled(a);
        let lit = dir.dot(light);
        let col = if lit > 0.0 { white(0.10 + lit * 0.20) } else { black(0.35 - lit * 0.25) };
        p.line_segment([c + dir * rad * 0.78, c + dir * (rad - 0.6)], Stroke::new(1.3, col));
    }
    p.circle_stroke(c, rad, Stroke::new(1.0, black(0.6)));
    // Cap: brushed metal as a conic gradient, two bright streaks opposite each other.
    let cap = rad * 0.72;
    let metal = t.eq_knob_cap;
    let pts = circle_points(c, cap);
    fan(p, c, metal, &pts, |q, _| {
        let a = (q - c).angle();
        let s = (2.0 * (a + std::f32::consts::FRAC_PI_4)).cos();
        shade(metal, s * 0.34)
    });
    p.circle_stroke(c, cap, Stroke::new(1.0, black(0.55)));
    // The cap's lit upper edge.
    let arc: Vec<Pos2> = (0..=16).map(|i| c + Vec2::angled(std::f32::consts::PI * (1.05 + i as f32 / 16.0 * 0.9)) * (cap - 0.8)).collect();
    p.add(egui::Shape::line(arc, Stroke::new(1.0, white(0.45))));
    // Pointer: an engraved line filled with the band's colour.
    let dir = Vec2::angled(angle);
    let (a, b) = (c + dir * cap * 0.25, c + dir * (rad - 1.5));
    p.line_segment([a, b], Stroke::new(3.2, black(0.6)));
    p.line_segment([a, b], Stroke::new(1.8, if hovered { shade(accent, 0.3) } else { accent }));
    // Fixed specular highlight, top left of the cap.
    let h = c + vec2(-0.3, -0.38) * cap;
    let hr = cap * 0.45;
    fan(p, h, t.eq_glint, &circle_points(h, hr), |_, _| Color32::TRANSPARENT);
}

/// A backlit push switch: recessed and dark when off; when on, `color` glows through the cap.
pub fn switch(p: &Painter, r: Rect, on: bool, hovered: bool, color: Color32) {
    let t = Tokens::current();
    let radius = 4.0;
    if on {
        for i in 1..=3 {
            p.rect_filled(r.expand(i as f32 * 1.2), radius + i as f32, color.gamma_multiply(0.10));
        }
        vgradient(p, r, radius, shade(color, 0.35), shade(color, -0.15));
    } else {
        let base = if hovered { shade(t.eq_switch, 0.08) } else { t.eq_switch };
        vgradient(p, r, radius, shade(base, 0.12), shade(base, -0.18));
    }
    p.line_segment(
        [pos2(r.left() + radius, r.top() + 1.0), pos2(r.right() - radius, r.top() + 1.0)],
        Stroke::new(1.0, white(if on { 0.45 } else { 0.18 })),
    );
    bevel(p, r, radius, false);
    p.rect_stroke(r, radius, Stroke::new(1.0, black(0.5)), egui::StrokeKind::Outside);
}

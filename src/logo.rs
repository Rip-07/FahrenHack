// FahrenHack logo rasterizer, shared by build.rs (exe icon) and the app (window icon).
// The geometry is a 1024×1024 box and matches assets/logo.svg:
// a navy rounded square, a white "F" whose bars tuck under a teal "H" and a teal dot.
// This file is `include!`d by build.rs, so it must not use crate paths.

#[allow(dead_code)]
const NAVY: [f32; 3] = [18.0, 35.0, 58.0];
#[allow(dead_code)]
const TEAL: [f32; 3] = [75.0, 192.0, 192.0];
#[allow(dead_code)]
const WHITE: [f32; 3] = [255.0, 255.0, 255.0];

#[allow(dead_code)]
fn sd_box(px: f32, py: f32, x0: f32, y0: f32, x1: f32, y1: f32, r: f32) -> f32 {
    let (cx, cy) = ((x0 + x1) * 0.5, (y0 + y1) * 0.5);
    let (hx, hy) = ((x1 - x0) * 0.5 - r, (y1 - y0) * 0.5 - r);
    let (qx, qy) = ((px - cx).abs() - hx, (py - cy).abs() - hy);
    let (ox, oy) = (qx.max(0.0), qy.max(0.0));
    (ox * ox + oy * oy).sqrt() + qx.max(qy).min(0.0) - r
}

#[allow(dead_code)]
fn sd_capsule(px: f32, py: f32, ax: f32, ay: f32, bx: f32, by: f32, r: f32) -> f32 {
    let (pax, pay, bax, bay) = (px - ax, py - ay, bx - ax, by - ay);
    let h = ((pax * bax + pay * bay) / (bax * bax + bay * bay)).clamp(0.0, 1.0);
    let (dx, dy) = (pax - bax * h, pay - bay * h);
    (dx * dx + dy * dy).sqrt() - r
}

#[allow(dead_code)]
fn sd_circle(px: f32, py: f32, cx: f32, cy: f32, r: f32) -> f32 {
    ((px - cx).powi(2) + (py - cy).powi(2)).sqrt() - r
}

/// Signed distances to the background, the white strokes and the teal strokes.
#[allow(dead_code)]
fn logo_sdf(x: f32, y: f32) -> (f32, f32, f32) {
    let bg = sd_box(x, y, 0.0, 0.0, 1024.0, 1024.0, 200.0);
    let white = sd_capsule(x, y, 240.0, 280.0, 240.0, 752.0, 48.0)
        .min(sd_box(x, y, 240.0, 232.0, 576.0, 328.0, 0.0))
        .min(sd_box(x, y, 240.0, 456.0, 440.0, 552.0, 0.0));
    let teal = sd_circle(x, y, 440.0, 504.0, 48.0)
        .min(sd_capsule(x, y, 576.0, 280.0, 576.0, 752.0, 48.0))
        .min(sd_capsule(x, y, 800.0, 280.0, 800.0, 752.0, 48.0))
        .min(sd_box(x, y, 576.0, 456.0, 800.0, 552.0, 0.0));
    (bg, white, teal)
}

/// RGBA pixels of the logo, `size`×`size`, anti-aliased.
#[allow(dead_code)]
pub fn logo_rgba(size: u32) -> Vec<u8> {
    let s = size as f32 / 1024.0;
    let mut out = Vec::with_capacity((size * size * 4) as usize);
    for py in 0..size {
        for px in 0..size {
            let (x, y) = ((px as f32 + 0.5) / s, (py as f32 + 0.5) / s);
            let (bg, w, t) = logo_sdf(x, y);
            let cov = |d: f32| (0.5 - d * s).clamp(0.0, 1.0);
            let (a, cw, ct) = (cov(bg), cov(w), cov(t));
            let mut c = NAVY;
            for i in 0..3 {
                c[i] += (WHITE[i] - c[i]) * cw;
                c[i] += (TEAL[i] - c[i]) * ct;
            }
            out.extend_from_slice(&[c[0].round() as u8, c[1].round() as u8, c[2].round() as u8, (a * 255.0).round() as u8]);
        }
    }
    out
}

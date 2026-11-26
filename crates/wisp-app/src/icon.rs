//! The tray icon, drawn in code: a blue drop, anti-aliased by supersampling.

pub const SIZE: u32 = 32;

fn inside(x: f32, y: f32) -> bool {
    // A circle at the bottom joined to a point at the top.
    let (cx, cy, r) = (16.0, 19.5, 9.5);
    let dx = x - cx;
    let dy = y - cy;
    if dx * dx + dy * dy <= r * r {
        return true;
    }
    // Tangent cone from the tip (16, 2) to the circle.
    if y < 2.0 || y > cy {
        return false;
    }
    let half = (y - 2.0) / (cy - 2.0) * r * 0.86;
    dx.abs() <= half
}

pub fn rgba() -> Vec<u8> {
    const SS: u32 = 4;
    let mut out = Vec::with_capacity((SIZE * SIZE * 4) as usize);
    for py in 0..SIZE {
        for px in 0..SIZE {
            let mut hits = 0;
            for sy in 0..SS {
                for sx in 0..SS {
                    let x = px as f32 + (sx as f32 + 0.5) / SS as f32;
                    let y = py as f32 + (sy as f32 + 0.5) / SS as f32;
                    hits += inside(x, y) as u32;
                }
            }
            let a = (hits * 255 / (SS * SS)) as u8;
            // #2F6BFF, lighter towards the top-left for a little depth.
            let shade = 1.0 + 0.18 * (1.0 - (px + py) as f32 / (2 * SIZE) as f32);
            let c = |v: f32| (v * shade).min(255.0) as u8;
            out.extend_from_slice(&[c(47.0), c(107.0), c(255.0), a]);
        }
    }
    out
}

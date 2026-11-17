// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 MOSSLS

//! Separable box blur stacked into an approximate Gaussian.
//!
//! A plane is blurred by alternating horizontal box passes with 90 degree rotations, so every
//! scan stays row-major: horizontal, transposed horizontal (which is the vertical box), then
//! horizontal again. Passes are split across threads, and the wide background blur runs on a
//! downsampled copy — that is what keeps HD and 4K frames cheap.

use std::thread;

/// Widths of `n` box passes that approximate a Gaussian of `sigma` (pixels).
fn boxes_for_gauss(sigma: f32, n: usize) -> Vec<usize> {
    if sigma <= 0.0 {
        return vec![1; n];
    }
    let w_ideal = (12.0 * sigma * sigma / n as f32 + 1.0).sqrt();
    let mut wl = w_ideal.floor() as i32;
    if wl % 2 == 0 {
        wl -= 1;
    }
    let wl = wl.max(1);
    let wu = wl + 2;
    let m_ideal = (12.0 * sigma * sigma
        - (n as f32) * (wl * wl) as f32
        - 4.0 * (n as f32) * wl as f32
        - 3.0 * (n as f32))
        / (-4.0 * wl as f32 - 4.0);
    let m = m_ideal.round().max(0.0) as usize;
    (0..n).map(|i| if i < m { wl as usize } else { wu as usize }).collect()
}

#[inline]
fn clamp_i(i: i64, n: usize) -> usize {
    if i < 0 {
        0
    } else if i as usize >= n {
        n - 1
    } else {
        i as usize
    }
}

/// One clamped box blur along a row, radius `r`.
fn row_box(s: &[f32], d: &mut [f32], w: usize, r: usize) {
    if w == 0 {
        return;
    }
    let k = 1.0 / (2 * r + 1) as f32;
    let mut sum = 0.0f32;
    for i in -(r as i64)..=(r as i64) {
        sum += s[clamp_i(i, w)];
    }
    let mut x = 0usize;
    // x < r: the left edge of the window is clamped to 0.
    let left_end = r.min(w);
    while x < left_end {
        d[x] = sum * k;
        sum += s[(x + 1 + r).min(w - 1)] - s[0];
        x += 1;
    }
    // Interior: no clamping, so the loop stays branch free.
    // Stepping to x + 1 adds s[x + 1 + r], which must stay below w; and x - r must be >= 0.
    let interior_end = (w - 1).saturating_sub(r);
    while x < interior_end {
        d[x] = sum * k;
        sum += s[x + 1 + r] - s[x - r];
        x += 1;
    }
    // Right edge: the window is clamped to w - 1.
    while x < w {
        d[x] = sum * k;
        sum += s[w - 1] - s[(x - r).min(w - 1)];
        x += 1;
    }
}

fn threads_for(h: usize) -> usize {
    let n = thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
    n.min(h.max(1)).max(1)
}

/// Horizontal box blur of a `w x h` plane, rows spread over threads.
fn box_rows(src: &[f32], dst: &mut [f32], w: usize, h: usize, r: usize) {
    if w == 0 || h == 0 {
        return;
    }
    if r == 0 {
        dst[..w * h].copy_from_slice(&src[..w * h]);
        return;
    }
    let threads = threads_for(h);
    if threads <= 1 || h < 96 {
        for y in 0..h {
            row_box(&src[y * w..(y + 1) * w], &mut dst[y * w..(y + 1) * w], w, r);
        }
        return;
    }
    let chunk = h.div_ceil(threads);
    thread::scope(|scope| {
        for (ci, out) in dst.chunks_mut(chunk * w).enumerate() {
            let start = ci * chunk;
            let src = src;
            scope.spawn(move || {
                for (yi, row) in out.chunks_mut(w).enumerate() {
                    let y = start + yi;
                    row_box(&src[y * w..(y + 1) * w], row, w, r);
                }
            });
        }
    });
}

/// `src` is `w x h`, `dst` is `h x w` (`dst[x * h + y] = src[y * w + x]`), tiled for cache.
fn transpose(src: &[f32], dst: &mut [f32], w: usize, h: usize) {
    const BLOCK: usize = 32;
    let mut y0 = 0;
    while y0 < h {
        let y1 = (y0 + BLOCK).min(h);
        let mut x0 = 0;
        while x0 < w {
            let x1 = (x0 + BLOCK).min(w);
            for y in y0..y1 {
                let row = &src[y * w..y * w + w];
                for x in x0..x1 {
                    dst[x * h + y] = row[x];
                }
            }
            x0 = x1;
        }
        y0 = y1;
    }
}
/// Blur a plane through `widths` box passes, rotating the image between passes.
fn blur_plane(src: &[f32], w: usize, h: usize, widths: &[usize]) -> Vec<f32> {
    let n = w * h;
    if n == 0 {
        return Vec::new();
    }
    let mut a = src.to_vec();
    let mut b = vec![0.0f32; n];
    let mut pivot = vec![0.0f32; n];
    let mut cur_w = w;
    let mut cur_h = h;
    let mut transposed = false;
    for (i, &width) in widths.iter().enumerate() {
        let r = width.saturating_sub(1) / 2;
        box_rows(&a, &mut b, cur_w, cur_h, r);
        std::mem::swap(&mut a, &mut b);
        if i + 1 < widths.len() {
            transpose(&a, &mut pivot, cur_w, cur_h);
            std::mem::swap(&mut a, &mut pivot);
            std::mem::swap(&mut cur_w, &mut cur_h);
            transposed = !transposed;
        }
    }
    if transposed {
        transpose(&a, &mut pivot, cur_w, cur_h);
        pivot
    } else {
        a
    }
}

/// Blur one channel plane with a Gaussian of `sigma` pixels.
pub fn gauss_plane(src: &[f32], w: usize, h: usize, sigma: f32) -> Vec<f32> {
    if sigma <= 0.05 || w == 0 || h == 0 {
        return src.to_vec();
    }
    blur_plane(src, w, h, &boxes_for_gauss(sigma, 3))
}

/// Blur `channels` interleaved planes (4 for RGBA) with a Gaussian of `sigma`.
pub fn gauss(src: &[f32], w: usize, h: usize, channels: usize, sigma: f32) -> Vec<f32> {
    let n = w * h;
    if sigma <= 0.05 || n == 0 || channels == 0 {
        return src.to_vec();
    }
    let widths = boxes_for_gauss(sigma, 3);
    let mut out = vec![0.0f32; n * channels];
    let mut plane = vec![0.0f32; n];
    for c in 0..channels {
        for i in 0..n {
            plane[i] = src[i * channels + c];
        }
        let blurred = blur_plane(&plane, w, h, &widths);
        for i in 0..n {
            out[i * channels + c] = blurred[i];
        }
    }
    out
}

/// Downsample 4 interleaved channels by `k` with a box average. Rows are spread over threads.
fn downsample(src: &[f32], w: usize, h: usize, k: usize) -> (Vec<f32>, usize, usize) {
    let dw = (w / k).max(1);
    let dh = (h / k).max(1);
    let mut out = vec![0.0f32; dw * dh * 4];
    let inv = 1.0 / (k * k) as f32;
    let threads = threads_for(dh);
    let chunk = dh.div_ceil(threads.max(1));
    thread::scope(|scope| {
        for (ci, rows) in out.chunks_mut(chunk * dw * 4).enumerate() {
            let start = ci * chunk;
            let count = rows.len() / (dw * 4);
            let src = src;
            scope.spawn(move || {
                for (ri, row) in rows.chunks_mut(dw * 4).enumerate() {
                    let dy = start + ri;
                    let ybase = (dy * k).min(h - 1);
                    for dx in 0..dw {
                        let xbase = (dx * k).min(w - 1);
                        let lim = k.min(w - xbase);
                        let (mut a0, mut a1, mut a2, mut a3) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
                        for sy in 0..k {
                            let y = (ybase + sy).min(h - 1);
                            let mut i = (y * w + xbase) * 4;
                            for _ in 0..lim {
                                a0 += src[i];
                                a1 += src[i + 1];
                                a2 += src[i + 2];
                                a3 += src[i + 3];
                                i += 4;
                            }
                        }
                        let o = dx * 4;
                        row[o] = a0 * inv;
                        row[o + 1] = a1 * inv;
                        row[o + 2] = a2 * inv;
                        row[o + 3] = a3 * inv;
                    }
                    let _ = count;
                }
            });
        }
    });
    (out, dw, dh)
}

/// Bilinear upsample of 4 interleaved channels to `w x h`. The horizontal weights are computed
/// once; rows are spread over threads.
fn upsample(src: &[f32], dw: usize, dh: usize, w: usize, h: usize) -> Vec<f32> {
    let mut out = vec![0.0f32; w * h * 4];
    let mut xs: Vec<(usize, usize, f32)> = Vec::with_capacity(w);
    for x in 0..w {
        let fx = ((x as f32 + 0.5) * (dw as f32 / w as f32) - 0.5).max(0.0);
        let x0 = (fx.floor() as usize).min(dw - 1);
        xs.push((x0, (x0 + 1).min(dw - 1), fx - x0 as f32));
    }
    let threads = threads_for(h);
    let chunk = h.div_ceil(threads.max(1));
    let dy = dh as f32 / h as f32;
    thread::scope(|scope| {
        for (ci, rows) in out.chunks_mut(chunk * w * 4).enumerate() {
            let start = ci * chunk;
            let xs = &xs;
            scope.spawn(move || {
                for (ri, row) in rows.chunks_mut(w * 4).enumerate() {
                    let y = start + ri;
                    let fy = ((y as f32 + 0.5) * dy - 0.5).max(0.0);
                    let y0 = (fy.floor() as usize).min(dh - 1);
                    let y1 = (y0 + 1).min(dh - 1);
                    let wy = fy - y0 as f32;
                    let r0 = y0 * dw * 4;
                    let r1 = y1 * dw * 4;
                    for x in 0..w {
                        let (x0, x1, wx) = xs[x];
                        let p00 = r0 + x0 * 4;
                        let p10 = r0 + x1 * 4;
                        let p01 = r1 + x0 * 4;
                        let p11 = r1 + x1 * 4;
                        let o = x * 4;
                        for c in 0..4 {
                            let top = src[p00 + c] * (1.0 - wx) + src[p10 + c] * wx;
                            let bot = src[p01 + c] * (1.0 - wx) + src[p11 + c] * wx;
                            row[o + c] = top * (1.0 - wy) + bot * wy;
                        }
                    }
                }
            });
        }
    });
    out
}
/// Interleaved blur tuned for the light wrap's background: a wide, soft blur computed on a
/// downsampled copy and interpolated back. Visually identical, much cheaper at HD and 4K.
pub fn gauss_rgba_downsampled(src: &[f32], w: usize, h: usize, sigma: f32) -> Vec<f32> {
    if sigma <= 0.05 || w == 0 || h == 0 {
        return src.to_vec();
    }
    let k = ((sigma / 4.0) as usize).clamp(1, 8).min(w).min(h);
    if k <= 1 {
        return gauss(src, w, h, 4, sigma);
    }
    let (small, dw, dh) = downsample(src, w, h, k);
    let blurred = gauss(&small, dw, dh, 4, sigma / k as f32);
    upsample(&blurred, dw, dh, w, h)
}





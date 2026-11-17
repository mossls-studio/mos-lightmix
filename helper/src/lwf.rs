//! Light Wrap Fantastic: the effect itself.
//!
//! The foreground's alpha edge is grown inwards (a blur of the alpha subtracted from the alpha),
//! the background is blurred by roughly the same radius and its light is blended into that band,
//! which is what makes a keyed subject look like it sits in front of a lit background.

use crate::blur;

/// Indices into the flattened parameter array (declaration order in [`manifest`]).
/// A slider / angle / checkbox / popup / layer takes one value, a point two, a colour four.
pub const I_RENDER: usize = 0;
pub const I_BACKGROUND: usize = 1;
pub const I_BACKGROUND_COLOR: usize = 2; // four values: r, g, b, a
pub const I_BACKGROUND_BLUR: usize = 6;
pub const I_SCALE_BACKGROUND: usize = 7;
pub const I_SIZE: usize = 8;
pub const I_BLEND_MODE: usize = 9;
pub const I_BRIGHTNESS: usize = 10;
pub const I_SHOW_MASK: usize = 11;
pub const I_MASK_STYLE: usize = 12;
/// Point: two floats, x and y in buffer pixels.
pub const I_MASK_CENTER: usize = 13;
pub const I_MASK_WIDTH: usize = 15;
pub const I_MASK_ANGLE: usize = 16;
pub const I_MASK_LAYER: usize = 17;
pub const I_MASK_CHANNEL: usize = 18;
pub const I_MASK_INVERT: usize = 19;
pub const I_USE_GPU: usize = 20;

pub const RENDER_COMPOSITE: i32 = 0;
pub const RENDER_WRAP: i32 = 1;

pub const BLEND_SOFT_LIGHT: i32 = 0;
pub const BLEND_OVERLAY: i32 = 1;
pub const BLEND_ADD: i32 = 2;
pub const BLEND_MULTIPLY: i32 = 3;
pub const BLEND_SCREEN: i32 = 4;
pub const BLEND_COLOR: i32 = 5;

pub const MASK_GRADIENT: i32 = 0;
pub const MASK_LAYER: i32 = 1;

/// The whole control set, in the order of the original plug-in's Effect Controls.
pub fn manifest() -> serde_json::Value {
    serde_json::json!({
        "api": 2,
        "id": "mos.lightwrap",
        "name": "mos_灯光混合",
        "category": "MOS 视效",
        "version": "1.4.0",
        "author": "MOSSLS",
        "description": "让背景的光绕过抠像主体的边缘包裹进来，模拟镜头背光衍射。",
        "params": [
            {"id": "render", "name": "渲染", "type": "popup", "options": ["合成", "仅光晕"], "default": 0},
            {"id": "background", "name": "背景", "type": "layer"},
            {"id": "backgroundColor", "name": "背景色", "type": "color", "default": [1.0, 0.88, 0.74, 1.0]},
            {"id": "backgroundBlur", "name": "背景模糊", "type": "slider", "default": 0, "min": 0, "max": 200, "sliderMax": 40, "decimals": 2},
            {"id": "scaleBackground", "name": "缩放背景", "type": "checkbox", "default": true},
            {"id": "size", "name": "尺寸", "type": "slider", "default": 24, "min": 0, "max": 500, "sliderMax": 80, "decimals": 2},
            {"id": "blendMode", "name": "混合模式", "type": "popup", "options": ["柔光", "叠加", "相加", "正片叠底", "滤色", "颜色"], "default": 4},
            {"id": "brightness", "name": "亮度", "type": "slider", "default": 100, "min": 0, "max": 200, "sliderMax": 200, "decimals": 1},
            {"id": "showMask", "name": "显示遮罩", "type": "checkbox", "default": false},
            {"id": "maskStyle", "name": "遮罩样式", "type": "popup", "options": ["渐变", "图层"], "default": 0},
            {"id": "maskCenter", "name": "中心点", "type": "point", "default": [0.5, 0.5]},
            {"id": "maskWidth", "name": "宽度", "type": "slider", "default": 120, "min": 0, "max": 400, "sliderMax": 200, "decimals": 1},
            {"id": "maskAngle", "name": "角度", "type": "angle", "default": 0},
            {"id": "maskLayer", "name": "遮罩输入", "type": "layer"},
            {"id": "maskChannel", "name": "遮罩通道", "type": "popup", "options": ["亮度", "Alpha"], "default": 0},
            {"id": "maskInvert", "name": "反转遮罩", "type": "checkbox", "default": false},
            {"id": "useGpu", "name": "使用 GPU", "type": "checkbox", "default": false}
        ]
    })
}

/// Parameters pulled out of the flattened array.
pub struct Params {
    pub render: i32,
    pub background: i64,
    pub background_color: [f32; 3],
    pub background_blur: f32,
    pub scale_background: bool,
    pub size: f32,
    pub blend_mode: i32,
    pub brightness: f32,
    pub show_mask: bool,
    pub mask_style: i32,
    pub mask_center: [f32; 2],
    pub mask_width: f32,
    pub mask_angle: f32,
    pub mask_layer: i64,
    pub mask_channel: i32,
    pub mask_invert: bool,
    pub use_gpu: bool,
}

impl Params {
    pub fn parse(flat: &[f64]) -> Params {
        let f = |i: usize, d: f64| flat.get(i).copied().unwrap_or(d);
        Params {
            render: f(I_RENDER, 0.0) as i32,
            background: f(I_BACKGROUND, -1.0) as i64,
            background_color: [
                f(I_BACKGROUND_COLOR, 0.0) as f32,
                f(I_BACKGROUND_COLOR + 1, 0.0) as f32,
                f(I_BACKGROUND_COLOR + 2, 0.0) as f32,
            ],
            background_blur: f(I_BACKGROUND_BLUR, 0.0).max(0.0) as f32,
            scale_background: f(I_SCALE_BACKGROUND, 1.0) != 0.0,
            size: f(I_SIZE, 15.0).max(0.0) as f32,
            blend_mode: f(I_BLEND_MODE, 0.0) as i32,
            brightness: (f(I_BRIGHTNESS, 100.0) / 100.0).max(0.0) as f32,
            show_mask: f(I_SHOW_MASK, 0.0) != 0.0,
            mask_style: f(I_MASK_STYLE, 0.0) as i32,
            mask_center: [f(I_MASK_CENTER, 0.0) as f32, f(I_MASK_CENTER + 1, 0.0) as f32],
            mask_width: f(I_MASK_WIDTH, 120.0).max(0.0) as f32,
            mask_angle: f(I_MASK_ANGLE, 0.0) as f32,
            mask_layer: f(I_MASK_LAYER, -1.0) as i64,
            mask_channel: f(I_MASK_CHANNEL, 0.0) as i32,
            mask_invert: f(I_MASK_INVERT, 0.0) != 0.0,
            use_gpu: f(I_USE_GPU, 0.0) != 0.0,
        }
    }
}

/// How many pixels of padding the render needs around the layer.
pub fn padding(flat: &[f64], scale: f64) -> u32 {
    let p = Params::parse(flat);
    if p.size <= 0.0 {
        return 0;
    }
    // The band and the background blur both reach outside the layer's own rectangle when the
    // background is another layer. Size and Background Blur are layer pixels; padding is asked in
    // buffer pixels, and `scale` is buffer pixels per layer pixel (0.2 in a zoomed-out preview).
    let s = if scale.is_finite() && scale > 0.0 { scale } else { 1.0 };
    let need = if p.background >= 0 {
        1.5 * f64::from(p.size) + 3.0 * f64::from(p.background_blur)
    } else {
        0.75 * f64::from(p.size)
    };
    (need * s).ceil().clamp(0.0, 256.0) as u32
}

/// Where layer images come from during a render (the host's layer parameters).
pub trait Layers {
    /// The layer picked in layer parameter `param` at `time`: (width, height, premultiplied RGBA).
    fn layer_image(&mut self, param: &str, time: f64) -> Option<(usize, usize, Vec<f32>)>;
}

fn resample(src: &[f32], sw: usize, sh: usize, dw: usize, dh: usize) -> Vec<f32> {
    if sw == dw && sh == dh {
        return src.to_vec();
    }
    let mut out = vec![0.0f32; dw * dh * 4];
    if sw == 0 || sh == 0 {
        return out;
    }
    for y in 0..dh {
        let sy = ((y as f32 + 0.5) * sh as f32 / dh as f32 - 0.5).clamp(0.0, (sh - 1) as f32);
        let y0 = sy.floor() as usize;
        let y1 = (y0 + 1).min(sh - 1);
        let fy = sy - y0 as f32;
        for x in 0..dw {
            let sx = ((x as f32 + 0.5) * sw as f32 / dw as f32 - 0.5).clamp(0.0, (sw - 1) as f32);
            let x0 = sx.floor() as usize;
            let x1 = (x0 + 1).min(sw - 1);
            let fx = sx - x0 as f32;
            for c in 0..4 {
                let p00 = src[(y0 * sw + x0) * 4 + c];
                let p10 = src[(y0 * sw + x1) * 4 + c];
                let p01 = src[(y1 * sw + x0) * 4 + c];
                let p11 = src[(y1 * sw + x1) * 4 + c];
                let top = p00 + (p10 - p00) * fx;
                let bot = p01 + (p11 - p01) * fx;
                out[(y * dw + x) * 4 + c] = top + (bot - top) * fy;
            }
        }
    }
    out
}

fn soft_light(f: f32, b: f32) -> f32 {
    if b <= 0.5 {
        f - (1.0 - 2.0 * b) * f * (1.0 - f)
    } else {
        let d = if f <= 0.25 { ((16.0 * f - 12.0) * f + 4.0) * f } else { f.sqrt() };
        f + (2.0 * b - 1.0) * (d - f)
    }
}

fn luma(c: [f32; 3]) -> f32 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

/// Photoshop's Colour blend: the base's luminosity, the blend's hue and saturation.
fn color_blend(f: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    let lf = luma(f);
    let sf = {
        let mx = f[0].max(f[1]).max(f[2]);
        let mn = f[0].min(f[1]).min(f[2]);
        if mx > 0.0 { (mx - mn) / mx } else { 0.0 }
    };
    let mxb = b[0].max(b[1]).max(b[2]);
    let mnb = b[0].min(b[1]).min(b[2]);
    let mut sat = [0.0f32; 3];
    if mxb > 0.0 {
        for c in 0..3 {
            sat[c] = (b[c] - mnb) * sf / (mxb - mnb).max(1e-6);
        }
    }
    let m = luma(sat);
    for c in sat.iter_mut() {
        *c += lf - m;
    }
    sat
}

fn blend(mode: i32, f: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    let mut o = [0.0f32; 3];
    match mode {
        BLEND_ADD => {
            for c in 0..3 {
                o[c] = f[c] + b[c];
            }
        }
        BLEND_MULTIPLY => {
            for c in 0..3 {
                o[c] = f[c] * b[c];
            }
        }
        BLEND_SCREEN => {
            for c in 0..3 {
                o[c] = 1.0 - (1.0 - f[c]) * (1.0 - b[c]);
            }
        }
        BLEND_OVERLAY => {
            for c in 0..3 {
                o[c] = if f[c] <= 0.5 { 2.0 * f[c] * b[c] } else { 1.0 - 2.0 * (1.0 - f[c]) * (1.0 - b[c]) };
            }
        }
        BLEND_COLOR => o = color_blend(f, b),
        // Soft Light is the default.
        _ => {
            for c in 0..3 {
                o[c] = soft_light(f[c], b[c]);
            }
        }
    }
    o
}

/// Render one frame in place. `px` is premultiplied RGBA, `w` x `h`, `origin` is the buffer
/// position of the layer's top-left pixel (padding shifts it) and `layer_size` the layer's own
/// size in layer pixels.
pub fn render(
    layers: &mut dyn Layers,
    px: &mut [f32],
    w: usize,
    h: usize,
    origin: [f64; 2],
    layer_size: [f64; 2],
    scale: f64,
    time: f64,
    flat: &[f64],
) -> Result<(), String> {
    let p = Params::parse(flat);
    let n = w * h;
    if n == 0 || px.len() < n * 4 {
        return Err("frame too small".into());
    }
    let scale_f = if scale.is_finite() && scale > 0.0 { scale as f32 } else { 1.0 };
    let layer_w = if layer_size[0] > 0.0 { layer_size[0] as f32 * scale_f } else { w as f32 };

    // The layer's own rectangle inside the (possibly padded) buffer. Everything outside it is
    // whatever sits below the layer and is passed through untouched.
    let ox = origin[0].round() as isize;
    let oy = origin[1].round() as isize;
    let rw = if layer_size[0] > 0.0 { (layer_size[0] as f32 * scale_f).round() as isize } else { w as isize };
    let rh = if layer_size[1] > 0.0 { (layer_size[1] as f32 * scale_f).round() as isize } else { h as isize };
    let padded = ox > 0 || oy > 0 || ox + rw < w as isize || oy + rh < h as isize;
    let inside = |x: usize, y: usize| -> bool {
        if !padded {
            return true;
        }
        let (x, y) = (x as isize, y as isize);
        x >= ox && x < ox + rw && y >= oy && y < oy + rh
    };

    // Foreground, straight alpha.
    let mut fg = vec![[0.0f32; 3]; n];
    let mut fa = vec![0.0f32; n];
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            if !inside(x, y) {
                continue;
            }
            let a = px[i * 4 + 3].clamp(0.0, 1.0);
            fa[i] = a;
            if a > 1e-5 {
                fg[i] = [px[i * 4] / a, px[i * 4 + 1] / a, px[i * 4 + 2] / a];
            }
        }
    }

    // Background (premultiplied RGBA in the frame's grid).
    let mut bgm = match if p.background >= 0 { layers.layer_image("background", time) } else { None } {
        Some((bw, bh, bpx)) => {
            if bw == w && bh == h {
                bpx
            } else {
                resample(&bpx, bw, bh, w, h)
            }
        }
        None => color_plane(p.background_color, w, h),
    };
    if bgm.len() < n * 4 {
        bgm = color_plane(p.background_color, w, h);
    }

    // Mask.
    let mask = mask_field(&p, layers, w, h, layer_w, time);

    if p.show_mask {
        for i in 0..n {
            let v = mask[i].clamp(0.0, 1.0);
            px[i * 4] = v;
            px[i * 4 + 1] = v;
            px[i * 4 + 2] = v;
            px[i * 4 + 3] = 1.0;
        }
        return Ok(());
    }

    // The wrap band: alpha minus a blurred alpha, times the alpha, peaks at 1 on the edge.
    // Size is in layer pixels, the buffer is in buffer pixels (`scale` per layer pixel), so the
    // blur has to follow it or the look would change with the preview zoom.
    let band = if p.size > 0.0 {
        let sigma = p.size * 0.5 * scale_f;
        let ab = blur::gauss_plane(&fa, w, h, sigma);
        (0..n).map(|i| (2.0 * fa[i] * (1.0 - ab[i])).clamp(0.0, 1.0)).collect::<Vec<f32>>()
    } else {
        vec![0.0f32; n]
    };

    // The light colour: the background blurred (Size blurs it, Background Blur more).
    let sigma_bg = (p.size * 0.5 + p.background_blur) * scale_f;
    let blurred = if sigma_bg > 0.05 { blur::gauss_rgba_downsampled(&bgm, w, h, sigma_bg) } else { bgm.clone() };
    let mut light = vec![[0.0f32; 3]; n];
    for i in 0..n {
        let a = blurred[i * 4 + 3].max(1e-6);
        for c in 0..3 {
            light[i][c] = (blurred[i * 4 + c] / a * p.brightness).clamp(0.0, 8.0);
        }
    }

    match p.render {
        RENDER_WRAP => {
            let base = p.background_color;
            for i in 0..n {
                let wt = (band[i] * mask[i]).clamp(0.0, 1.0);
                for c in 0..3 {
                    px[i * 4 + c] = (base[c] + light[i][c] * wt).clamp(0.0, 1.0);
                }
                px[i * 4 + 3] = 1.0;
            }
        }
        _ => {
            for y in 0..h {
                for x in 0..w {
                    if !inside(x, y) {
                        continue;
                    }
                    let i = y * w + x;
                    let wt = (band[i] * mask[i]).clamp(0.0, 1.0);
                    let a = fa[i];
                    if wt <= 0.0 || a <= 0.0 {
                        for c in 0..3 {
                            px[i * 4 + c] = fg[i][c] * a;
                        }
                        px[i * 4 + 3] = a;
                        continue;
                    }
                    let o = blend(p.blend_mode, fg[i], light[i]);
                    for c in 0..3 {
                        let v = (fg[i][c] * (1.0 - wt) + o[c] * wt).clamp(0.0, 1.0);
                        px[i * 4 + c] = v * a;
                    }
                    px[i * 4 + 3] = a;
                }
            }
        }
    }
    Ok(())
}

fn color_plane(c: [f32; 3], w: usize, h: usize) -> Vec<f32> {
    let n = w * h;
    let mut out = vec![0.0f32; n * 4];
    for i in 0..n {
        out[i * 4] = c[0];
        out[i * 4 + 1] = c[1];
        out[i * 4 + 2] = c[2];
        out[i * 4 + 3] = 1.0;
    }
    out
}

fn mask_field(p: &Params, layers: &mut dyn Layers, w: usize, h: usize, layer_w: f32, time: f64) -> Vec<f32> {
    let n = w * h;
    if p.mask_style == MASK_LAYER && p.mask_layer >= 0 {
        if let Some((mw, mh, mpx)) = layers.layer_image("maskLayer", time) {
            let src = if mw == w && mh == h { mpx } else { resample(&mpx, mw, mh, w, h) };
            let mut out = vec![0.0f32; n];
            for i in 0..n {
                let v = if p.mask_channel == 1 {
                    src[i * 4 + 3]
                } else {
                    let a = src[i * 4 + 3].max(1e-6);
                    luma([src[i * 4] / a, src[i * 4 + 1] / a, src[i * 4 + 2] / a])
                };
                out[i] = if p.mask_invert { 1.0 - v } else { v }.clamp(0.0, 1.0);
            }
            return out;
        }
    }
    // Gradient: white at the centre point (and behind it), fading to black `width` further along
    // the angle.
    let width_px = (p.mask_width / 100.0 * layer_w).max(0.5);
    let (cx, cy) = (p.mask_center[0], p.mask_center[1]);
    let a = p.mask_angle.to_radians();
    let (dx, dy) = (a.cos(), a.sin());
    let mut out = vec![0.0f32; n];
    for y in 0..h {
        for x in 0..w {
            let t = 1.0 - ((x as f32 - cx) * dx + (y as f32 - cy) * dy) / width_px;
            out[y * w + x] = t.clamp(0.0, 1.0);
        }
    }
    out
}



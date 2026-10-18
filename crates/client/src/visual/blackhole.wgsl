// GPU port of the black hole tracer (same model as blackhole.rs; originally s0xDk/ghostty-blackhole, MIT).
// One invocation per output pixel. Output: packed RGBA8 (straight alpha).

struct P {
    size: vec4<f32>,    // w, h, t, audio gain multiplier
    center: vec4<f32>,  // cx, cy (uv, y down), rh (fraction of h), alpha mode (0/1)
    look0: vec4<f32>,   // temp, incl, roll, inner
    look1: vec4<f32>,   // outer, opac, dopp, beam
    look2: vec4<f32>,   // gain, contr, wind, speed
    look3: vec4<f32>,   // expo, star, tint mode (0/1: brightness onto the ramp), quantize levels (0 = off)
    tint: vec4<f32>,    // unused (keeps the layout)
    ramp0: vec4<f32>,   // palette stops (rgb): the Theme's tint, or pixel art's when quantizing untinted
    ramp1: vec4<f32>,
    ramp2: vec4<f32>,
    ramp3: vec4<f32>,
};

@group(0) @binding(0) var<uniform> p: P;
@group(0) @binding(1) var<storage, read_write> outbuf: array<u32>;

const B_CRIT: f32 = 2.5980762;
const TAU: f32 = 6.2831853;

fn hash21(q: vec2<f32>) -> f32 {
    var a = fract(q * vec2<f32>(234.34, 435.345));
    a += dot(a, a + 34.23);
    return fract(a.x * a.y);
}

fn vnoise_wrap_y(q: vec2<f32>, per: f32) -> f32 {
    let i = floor(q);
    var f = fract(q);
    f = f * f * (3.0 - 2.0 * f);
    let y0 = i.y - per * floor(i.y / per);
    let y1 = (i.y + 1.0) - per * floor((i.y + 1.0) / per);
    return mix(mix(hash21(vec2<f32>(i.x, y0)), hash21(vec2<f32>(i.x + 1.0, y0)), f.x),
               mix(hash21(vec2<f32>(i.x, y1)), hash21(vec2<f32>(i.x + 1.0, y1)), f.x), f.y);
}

fn blackbody(tk: f32) -> vec3<f32> {
    let t = clamp(tk, 1500.0, 40000.0) / 100.0;
    var r = 1.0;
    var g = 0.0;
    var b = 1.0;
    if (t > 66.0) {
        r = clamp(1.292936 * pow(t - 60.0, -0.1332047), 0.0, 1.0);
        g = clamp(1.1298909 * pow(t - 60.0, -0.0755148), 0.0, 1.0);
    } else {
        g = clamp(0.3900816 * log(t) - 0.6318414, 0.0, 1.0);
        if (t <= 19.0) { b = 0.0; } else { b = clamp(0.5432068 * log(t - 10.0) - 1.1962540, 0.0, 1.0); }
    }
    return vec3<f32>(r, g, b);
}

fn stars(d: vec3<f32>, t: f32) -> vec3<f32> {
    let g = vec2<f32>(atan2(d.x, -d.z), asin(clamp(d.y, -1.0, 1.0))) * 40.0;
    let id = floor(g);
    let h = hash21(id);
    if (h < 0.92) { return vec3<f32>(0.0); }
    let f = fract(g) - 0.5;
    let off = (vec2<f32>(hash21(id + 17.3), hash21(id + 31.7)) - 0.5) * 0.7;
    let spark = smoothstep(0.10, 0.0, length(f - off));
    let tw = 0.7 + 0.3 * sin(t * (0.5 + 2.0 * hash21(id + 5.1)) + 40.0 * h);
    let tint = mix(vec3<f32>(1.0, 0.82, 0.60), vec3<f32>(0.75, 0.85, 1.0), hash21(id + 2.9));
    return tint * spark * tw * ((h - 0.92) / 0.08);
}

fn bayer4(c: vec2<u32>) -> f32 {
    let m = array<f32, 16>(0.0, 8.0, 2.0, 10.0, 12.0, 4.0, 14.0, 6.0, 3.0, 11.0, 1.0, 9.0, 15.0, 7.0, 13.0, 5.0);
    return (m[(c.y % 4u) * 4u + (c.x % 4u)] + 0.5) / 16.0;
}

fn palette(l: f32) -> vec3<f32> {
    let s = clamp(l, 0.0, 1.0) * 3.0;
    if (s < 1.0) { return mix(p.ramp0.rgb, p.ramp1.rgb, s); }
    if (s < 2.0) { return mix(p.ramp1.rgb, p.ramp2.rgb, s - 1.0); }
    return mix(p.ramp2.rgb, p.ramp3.rgb, s - 2.0);
}

fn rot(v: vec2<f32>, a: f32) -> vec2<f32> {
    let c = cos(a);
    let s = sin(a);
    return vec2<f32>(c * v.x - s * v.y, s * v.x + c * v.y);
}

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let w = u32(p.size.x);
    let hh = u32(p.size.y);
    if (gid.x >= w || gid.y >= hh) { return; }
    let W = p.size.x;
    let H = p.size.y;
    let t = p.size.z;
    let temp = p.look0.x; let incl = p.look0.y; let roll = p.look0.z;
    let rin = max(p.look0.w, 1.6);
    let rout = max(p.look1.x, rin + 0.5);
    let opac = p.look1.y; let dopp = p.look1.z; let beam = p.look1.w;
    let gain = p.look2.x * p.size.w; let contr = p.look2.y; let wind = p.look2.z; let speed = p.look2.w;
    let expo = p.look3.x; let star = p.look3.y;
    let alpha_mode = p.center.w > 0.5;

    let px = (f32(gid.x) + 0.5 - p.center.x * W) / H;
    let py = -((f32(gid.y) + 0.5 - p.center.y * H) / H);
    let wk = B_CRIT / max(p.center.z, 1e-3);
    let pr = rot(vec2<f32>(px, py), roll) * wk;
    let b = length(pr);
    let bmax = rout + 3.0;
    let z0 = max(14.0, rout + 5.0);

    var col = vec3<f32>(0.0);
    var alpha = 0.0;
    if (b >= bmax) {
        if (!alpha_mode && star > 0.0) {
            let d = normalize(vec3<f32>(-(pr / b) * (2.0 / b), -1.0));
            col = stars(d, t) * star;
        }
    } else {
        var x = vec3<f32>(pr, z0);
        var v = vec3<f32>(0.0, 0.0, -1.0);
        let h2 = dot(pr, pr);
        let ci = cos(incl); let si = sin(incl);
        let n = vec3<f32>(0.0, si, ci);
        let e2 = vec3<f32>(0.0, ci, -si);
        let sdir = select(1.0, -1.0, speed < 0.0);
        let spd = abs(speed);
        var emitc = vec3<f32>(0.0);
        var trans = 1.0;
        var captured = false;
        var s_prev = dot(x, n);
        var x_prev = x;
        for (var i = 0; i < 56; i++) {
            var r2 = dot(x, x);
            if (r2 < 1.0) { captured = true; break; }
            if ((x.z < -z0 && v.z < 0.0) || r2 > 4.0 * z0 * z0) { break; }
            var r = sqrt(r2);
            let dt = clamp(0.16 * r, 0.03, 1.5);
            v += -1.5 * h2 * x / (r2 * r2 * r) * (0.5 * dt);
            x += v * dt;
            r2 = dot(x, x);
            r = sqrt(r2);
            v += -1.5 * h2 * x / (r2 * r2 * r) * (0.5 * dt);
            let s = dot(x, n);
            if (s * s_prev < 0.0 && trans > 0.02) {
                let tc = s_prev / (s_prev - s);
                let xc = mix(x_prev, x, tc);
                let rc = length(xc);
                if (rc > rin && rc < rout) {
                    let band = smoothstep(rin, rin * 1.25, rc) * (1.0 - smoothstep(rout * 0.70, rout, rc));
                    let phi = atan2(dot(xc, e2), xc.x);
                    let turns = phi / TAU;
                    let kep = pow(rin / rc, 1.5);
                    let gloc = sqrt(max(1.0 - 1.5 / rc, 0.02));
                    let swirl = rc * wind * 0.12 - t * kep * spd * gloc * sdir;
                    var streaks = vnoise_wrap_y(vec2<f32>(rc * 2.8, turns * 19.0 + swirl * 3.0), 19.0) * 0.65
                                + vnoise_wrap_y(vec2<f32>(rc * 1.0, turns * 9.0 + swirl * 1.5 + 7.0), 9.0) * 0.35;
                    streaks = 0.35 + contr * streaks * streaks;
                    let gas = normalize(cross(n, xc)) * sdir;
                    let beta = clamp(inverseSqrt(max(2.0 * (rc - 1.0), 0.2)), 0.0, 0.99);
                    var g = gloc / max(1.0 + beta * dot(gas, normalize(v)), 0.05);
                    g = mix(1.0, g, dopp);
                    let xpr = max(1.0 - sqrt(rin / rc), 0.0);
                    let tprof = pow(rin / rc, 0.75) * pow(xpr, 0.25) / 0.488;
                    let cbb = blackbody(temp * tprof * g);
                    let boost = pow(g, beam);
                    let density = band * streaks;
                    emitc += trans * cbb * (gain * 2.2 * density * tprof * tprof * boost);
                    trans *= 1.0 - clamp(opac * density, 0.0, 1.0);
                }
            }
            s_prev = s;
            x_prev = x;
        }
        if (!captured && dot(x, x) < 4.0) { captured = true; }
        var bg = vec3<f32>(0.0);
        if (!captured && !alpha_mode && star > 0.0) { bg = stars(normalize(v), t) * star; }
        col = bg * trans + (vec3<f32>(1.0) - exp(-emitc * expo));
        if (alpha_mode) {
            alpha = select(clamp(max(1.0 - trans, max(col.r, max(col.g, col.b))), 0.0, 1.0), 1.0, captured);
        }
    }

    let lum = clamp(dot(col, vec3<f32>(0.2126, 0.7152, 0.0722)), 0.0, 1.0);
    let levels = p.look3.w;
    if (p.look3.z > 0.5 && levels <= 0.5) {
        col = palette(pow(lum, 0.8));
    }
    if (levels > 0.5) {
        // pixel art: ordered dither onto a small palette
        let q = floor(clamp(pow(lum, 0.8) * levels + bayer4(gid.xy) - 0.5, 0.0, levels) ) / levels;
        col = select(palette(q), vec3<f32>(0.0), q <= 0.0);
        if (alpha_mode) { alpha = select(alpha, 0.0, q <= 0.0 && alpha < 0.99); }
    }
    if (!alpha_mode) { alpha = 1.0; }
    let c = clamp(col / max(select(1.0, alpha, alpha_mode), 0.02), vec3<f32>(0.0), vec3<f32>(1.0));
    let o = vec4<u32>(vec4<f32>(c, alpha) * 255.0 + 0.5);
    outbuf[gid.y * w + gid.x] = o.x | (o.y << 8u) | (o.z << 16u) | (o.w << 24u);
}

//! Lumisound "sound eclipse" icon generator: writes the icon SVGs (full icon,
//! tvOS parallax layers, Top Shelf banners) and renders every app-icon PNG for
//! iOS, tvOS and watchOS. Rust port of ios/Branding/make_icon.py + render.sh
//! (which needed Python, rsvg-convert and ImageMagick).
//!
//!   cargo run --release -p lumisound-branding [-- --svg-only] [-- --branding-dir DIR]
//!
//! The star fields use a port of Python's `random` (Mersenne Twister with the
//! same seeding and sampling), so the SVGs are byte-for-byte what the Python
//! script produced (see the tests).

mod pyrandom;

use pyrandom::PyRandom;
use std::f64::consts::PI;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

const W: i64 = 1024;
const H: i64 = 1024;
const CX: i64 = 512;
const CY: i64 = 520;
const R_DISC: i64 = 214;
const DEFS: &str = include_str!("defs.svg");
/// Opaque layers are flattened onto the sky's darkest colour.
const MATTE: (u8, u8, u8) = (0x05, 0x03, 0x0d);

/// Python's `repr(float)` (and str() inside an f-string): shortest round-trip
/// digits, with a trailing `.0` on whole numbers.
fn py(x: f64) -> String {
    if x.is_finite() && x.fract() == 0.0 && x.abs() < 1e16 { format!("{x:.1}") } else { format!("{x}") }
}

fn rays() -> String {
    let n = 96;
    let mut parts = Vec::with_capacity(n);
    for i in 0..n {
        let a = i as f64 / n as f64 * 2.0 * PI - PI / 2.0;
        // A waveform wrapped round the disc: two harmonics + a gentle beat.
        let t = i as f64 / n as f64 * 2.0 * PI;
        let w = 0.55 + 0.28 * (t * 6.0).sin() * (t * 2.0).cos() + 0.17 * (t * 13.0 + 0.7).sin();
        let inner = (R_DISC + 22) as f64;
        let outer = inner + 34.0 + 120.0 * w.max(0.08);
        let (x1, y1) = (CX as f64 + a.cos() * inner, CY as f64 + a.sin() * inner);
        let (x2, y2) = (CX as f64 + a.cos() * outer, CY as f64 + a.sin() * outer);
        parts.push(format!("<line x1=\"{x1:.1}\" y1=\"{y1:.1}\" x2=\"{x2:.1}\" y2=\"{y2:.1}\" />"));
    }
    parts.join("\n")
}

fn corona(blur: bool) -> String {
    let filt = if blur { " filter=\"url(#glow)\" opacity=\"0.85\"" } else { "" };
    format!("<g stroke=\"url(#aurora)\" stroke-width=\"13\" stroke-linecap=\"round\"{filt}>{}</g>", rays())
}

fn pulse_path() -> String {
    let (x0, x1) = (CX - R_DISC + 30, CX + R_DISC - 30);
    let steps = 160;
    let mut pts = Vec::with_capacity(steps as usize + 1);
    for k in 0..=steps {
        let x = x0 as f64 + ((x1 - x0) * k) as f64 / steps as f64;
        let u = (x - CX as f64) / (R_DISC - 30) as f64;
        let env = (u * PI / 2.0).cos().powf(2.0);
        let y = CY as f64 + 70.0 * env * (u * PI * 3.2).sin() * (0.6 + 0.4 * (u * 7.0).cos());
        pts.push(format!("{x:.1},{y:.1}"));
    }
    format!("M{}", pts.join(" L"))
}

fn spark(x: i64, y: i64, s: i64) -> String {
    let q = s as f64 * 0.18;
    let (xf, yf, sf) = (x as f64, y as f64, s as f64);
    format!(
        "<path d=\"M{x},{} Q{},{} {},{y} Q{},{} {x},{} Q{},{} {},{y} Q{},{} {x},{} Z\" fill=\"#fff6e0\"/>\
         <circle cx=\"{x}\" cy=\"{y}\" r=\"{}\" fill=\"#ffd27a\" filter=\"url(#soft)\" opacity=\"0.9\"/>",
        y - s,
        py(xf + q),
        py(yf - q),
        x + s,
        py(xf + q),
        py(yf + q),
        y + s,
        py(xf - q),
        py(yf + q),
        x - s,
        py(xf - q),
        py(yf - q),
        y - s,
        py(sf * 0.55),
    )
}

fn canvas(w: i64, h: i64, body: &str) -> String {
    format!("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w}\" height=\"{h}\" viewBox=\"0 0 {w} {h}\">{DEFS}{body}</svg>")
}

/// Corona + disc + pulse + spark: the transparent mark.
fn mark() -> String {
    let middle = corona(true) + &corona(false);
    let pulse = pulse_path();
    let front = format!(
        "<circle cx=\"{CX}\" cy=\"{CY}\" r=\"{R_DISC}\" fill=\"url(#disc)\"/>\
         <circle cx=\"{CX}\" cy=\"{CY}\" r=\"{R_DISC}\" fill=\"none\" stroke=\"url(#aurora)\" stroke-width=\"5\" opacity=\"0.9\"/>\
         <path d=\"{pulse}\" fill=\"none\" stroke=\"url(#pulse)\" stroke-width=\"30\" stroke-linecap=\"round\" filter=\"url(#glow)\" opacity=\"0.8\"/>\
         <path d=\"{pulse}\" fill=\"none\" stroke=\"url(#pulse)\" stroke-width=\"16\" stroke-linecap=\"round\" stroke-linejoin=\"round\"/>{}",
        spark(CX + 228, CY - 236, 44)
    );
    middle + &front
}

fn icon_svg() -> String {
    let back = format!("<rect width=\"{W}\" height=\"{H}\" fill=\"url(#sky)\"/>");
    // Faint stars.
    let mut rng = PyRandom::seed(7);
    let mut stars = String::new();
    for _ in 0..70 {
        let cx = rng.randint(20, 1000);
        let cy = rng.randint(20, 1000);
        let r = rng.choice(&[1.2, 1.6, 2.2]);
        let op = rng.uniform(0.2, 0.6);
        let _ = write!(stars, "<circle cx=\"{cx}\" cy=\"{cy}\" r=\"{}\" fill=\"#ffffff\" opacity=\"{op:.2}\"/>", py(r));
    }
    canvas(W, H, &(back + &stars + &mark()))
}

/// The mark (with corona) spans ~840 px of the 1024 artboard.
fn placed(cx: &str, cy: &str, size: f64) -> String {
    let s = size / 840.0;
    format!("<g transform=\"translate({cx} {cy}) scale({s:.4}) translate({} {})\">{}</g>", -CX, -CY, mark())
}

fn sky(w: i64, h: i64) -> String {
    let mut rng = PyRandom::seed(11);
    let mut st = String::new();
    for _ in 0..((w * h) as f64 / 14000.0) as i64 {
        let cx = rng.uniform(0.0, w as f64);
        let cy = rng.uniform(0.0, h as f64);
        let r = rng.choice(&[1.2, 1.6, 2.2]);
        let op = rng.uniform(0.2, 0.6);
        let _ = write!(st, "<circle cx=\"{cx:.0}\" cy=\"{cy:.0}\" r=\"{}\" fill=\"#fff\" opacity=\"{op:.2}\"/>", py(r));
    }
    format!(
        "<radialGradient id=\"tvsky\" gradientUnits=\"userSpaceOnUse\" cx=\"{}\" cy=\"{}\" r=\"{}\">\
         <stop offset=\"0\" stop-color=\"#3a1580\"/><stop offset=\"0.45\" stop-color=\"#160a3c\"/><stop offset=\"1\" stop-color=\"#05030d\"/></radialGradient>\
         <rect width=\"{w}\" height=\"{h}\" fill=\"url(#tvsky)\"/>{st}",
        py(w as f64 * 0.5),
        py(h as f64 * 0.55),
        py(w.max(h) as f64 * 0.62)
    )
}

fn wordmark(x: f64, y: i64, size: i64) -> String {
    let s = size as f64;
    format!(
        "<text x=\"{}\" y=\"{y}\" font-family=\"Inter, Manrope, sans-serif\" font-weight=\"700\" font-size=\"{size}\" \
         letter-spacing=\"{:.1}\" fill=\"#ffffff\">Lumisound</text>\
         <text x=\"{}\" y=\"{:.0}\" font-family=\"Inter, Manrope, sans-serif\" font-weight=\"500\" \
         font-size=\"{:.0}\" letter-spacing=\"{:.1}\" fill=\"#c9b8ff\" opacity=\"0.85\">YOUR MUSIC, LIT UP</text>",
        py(x),
        s * 0.02,
        py(x + 4.0),
        y as f64 + s * 0.62,
        s * 0.30,
        s * 0.08
    )
}

/// Every SVG, by file name.
fn all_svgs() -> Vec<(&'static str, String)> {
    let mut v = vec![
        ("icon.svg", icon_svg()),
        // tvOS app icon layers (1280x768 artboard, rendered at 400x240 @1x/@2x).
        ("tv-back.svg", canvas(1280, 768, &sky(1280, 768))),
        ("tv-front.svg", canvas(1280, 768, &placed("640", "384", 700.0))),
    ];
    for (name, w) in [("tv-topshelf.svg", 1920), ("tv-topshelf-wide.svg", 2320)] {
        let x = w as f64 * 0.30;
        let body = sky(w, 720) + &placed(&py(x), "360", 600.0) + &wordmark(x + 380.0, 380, 132);
        v.push((name, canvas(w, 720, &body)));
    }
    v
}

// MARK: - Rendering

struct Renderer {
    opt: resvg::usvg::Options<'static>,
}

impl Renderer {
    fn new() -> Self {
        let mut opt = resvg::usvg::Options::default();
        opt.fontdb_mut().load_system_fonts();
        Renderer { opt }
    }

    fn render(&self, svg: &str, w: u32, h: u32) -> Result<resvg::tiny_skia::Pixmap, String> {
        let tree = resvg::usvg::Tree::from_str(svg, &self.opt).map_err(|e| e.to_string())?;
        let mut pm = resvg::tiny_skia::Pixmap::new(w, h).ok_or("bad size")?;
        let size = tree.size();
        let ts = resvg::tiny_skia::Transform::from_scale(w as f32 / size.width(), h as f32 / size.height());
        resvg::render(&tree, ts, &mut pm.as_mut());
        Ok(pm)
    }

    /// Flattened onto the matte, written as 24-bit RGB (App Store icons
    /// must not have an alpha channel).
    fn opaque(&self, svg: &str, w: u32, h: u32, out: &Path) -> Result<(), String> {
        let pm = self.render(svg, w, h)?;
        let mut rgb = Vec::with_capacity((w * h * 3) as usize);
        for p in pm.pixels() {
            // Premultiplied: out = src + matte * (1 - a).
            let ia = 255 - p.alpha() as u32;
            let mix = |c: u8, m: u8| (c as u32 + (m as u32 * ia + 127) / 255).min(255) as u8;
            rgb.extend_from_slice(&[mix(p.red(), MATTE.0), mix(p.green(), MATTE.1), mix(p.blue(), MATTE.2)]);
        }
        write_png(out, w, h, png::ColorType::Rgb, &rgb)
    }

    fn alpha(&self, svg: &str, w: u32, h: u32, out: &Path) -> Result<(), String> {
        let pm = self.render(svg, w, h)?;
        let mut rgba = Vec::with_capacity((w * h * 4) as usize);
        for p in pm.pixels() {
            let c = p.demultiply();
            rgba.extend_from_slice(&[c.red(), c.green(), c.blue(), c.alpha()]);
        }
        write_png(out, w, h, png::ColorType::Rgba, &rgba)
    }
}

fn write_png(out: &Path, w: u32, h: u32, color: png::ColorType, data: &[u8]) -> Result<(), String> {
    if let Some(d) = out.parent() {
        std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
    }
    let f = std::fs::File::create(out).map_err(|e| format!("{}: {e}", out.display()))?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(f), w, h);
    enc.set_color(color);
    enc.set_depth(png::BitDepth::Eight);
    enc.set_compression(png::Compression::Best);
    let mut wr = enc.write_header().map_err(|e| e.to_string())?;
    wr.write_image_data(data).map_err(|e| e.to_string())
}

fn render_all(branding: &Path, svgs: &[(&str, String)]) -> Result<usize, String> {
    let svg = |name: &str| svgs.iter().find(|(n, _)| *n == name).map(|(_, s)| s.as_str()).expect("generated");
    let ios = branding.join("../Lumisound/Resources/Assets.xcassets");
    let tv = branding.join("../LumisoundTV/Resources/Assets.xcassets/App Icon & Top Shelf Image.brandassets");
    let watch = branding.join("../LumisoundWatch/Resources/Assets.xcassets");
    let r = Renderer::new();
    let mut n = 0;
    let mut opaque = |name: &str, w: u32, h: u32, out: PathBuf| -> Result<(), String> {
        r.opaque(svg(name), w, h, &out)?;
        n += 1;
        Ok(())
    };
    let icon_1024 = branding.join("icon-1024.png");
    opaque("icon.svg", 1024, 1024, icon_1024.clone())?;
    // iOS / iPadOS app icon: each size rendered straight from the vector.
    let set = ios.join("AppIcon.appiconset");
    for (label, px) in [
        ("20@1x", 20), ("20@2x", 40), ("20@3x", 60), ("29@1x", 29), ("29@2x", 58), ("29@3x", 87), ("40@1x", 40),
        ("40@2x", 80), ("40@3x", 120), ("60@2x", 120), ("60@3x", 180), ("76@1x", 76), ("76@2x", 152), ("83.5@2x", 167),
    ] {
        opaque("icon.svg", px, px, set.join(format!("Icon-{label}.png")))?;
    }
    // tvOS: two parallax layers + Top Shelf.
    let content = |stack: &str, layer: &str, file: &str| tv.join(stack).join(format!("{layer}.imagestacklayer/Content.imageset")).join(file);
    opaque("tv-back.svg", 400, 240, content("App Icon.imagestack", "Back", "img.png"))?;
    opaque("tv-back.svg", 800, 480, content("App Icon.imagestack", "Back", "img@2x.png"))?;
    opaque("tv-back.svg", 1280, 768, content("App Icon - App Store.imagestack", "Back", "img.png"))?;
    opaque("tv-topshelf.svg", 1920, 720, tv.join("Top Shelf Image.imageset/img.png"))?;
    opaque("tv-topshelf.svg", 3840, 1440, tv.join("Top Shelf Image.imageset/img@2x.png"))?;
    opaque("tv-topshelf-wide.svg", 2320, 720, tv.join("Top Shelf Image Wide.imageset/img.png"))?;
    opaque("tv-topshelf-wide.svg", 4640, 1440, tv.join("Top Shelf Image Wide.imageset/img@2x.png"))?;
    for (w, h, stack, file) in [
        (400, 240, "App Icon.imagestack", "img.png"),
        (800, 480, "App Icon.imagestack", "img@2x.png"),
        (1280, 768, "App Icon - App Store.imagestack", "img.png"),
    ] {
        r.alpha(svg("tv-front.svg"), w, h, &content(stack, "Front", file))?;
        n += 1;
    }
    // The 1024 copies: iOS marketing icon, in-app display, watchOS (the
    // system masks it to a circle).
    for dest in [set.join("Icon-1024.png"), ios.join("AppIconDisplay.imageset/AppIconDisplay.png"), watch.join("AppIcon.appiconset/Icon-1024.png")] {
        if let Some(d) = dest.parent() {
            std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
        }
        std::fs::copy(&icon_1024, &dest).map_err(|e| format!("{}: {e}", dest.display()))?;
        n += 1;
    }
    Ok(n)
}

fn main() {
    let mut branding = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../ios/Branding");
    let mut svg_only = false;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--svg-only" => svg_only = true,
            "--branding-dir" => branding = PathBuf::from(args.next().unwrap_or_else(|| die("--branding-dir needs a path"))),
            "-h" | "--help" => {
                println!("lumisound-branding [--svg-only] [--branding-dir DIR]   (default DIR: ios/Branding)");
                return;
            }
            other => die(&format!("unknown option {other}")),
        }
    }
    std::fs::create_dir_all(&branding).unwrap_or_else(|e| die(&e.to_string()));
    let svgs = all_svgs();
    for (name, s) in &svgs {
        std::fs::write(branding.join(name), s).unwrap_or_else(|e| die(&format!("{name}: {e}")));
    }
    println!("wrote {} SVGs to {}", svgs.len(), branding.display());
    if !svg_only {
        match render_all(&branding, &svgs) {
            Ok(n) => println!("rendered {n} PNGs"),
            Err(e) => die(&e),
        }
    }
}

fn die(msg: &str) -> ! {
    eprintln!("lumisound-branding: {msg}");
    std::process::exit(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The SVGs match the Python generator's output byte for byte.
    #[test]
    fn svgs_match_the_python_generator() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
        for (name, s) in all_svgs() {
            let want = std::fs::read_to_string(dir.join(name)).unwrap();
            if s != want {
                let at = s.bytes().zip(want.bytes()).position(|(a, b)| a != b).unwrap_or(s.len().min(want.len()));
                let lo = at.saturating_sub(60);
                panic!("{name} differs at byte {at}:\n got  …{}\n want …{}", &s[lo..(at + 60).min(s.len())], &want[lo..(at + 60).min(want.len())]);
            }
        }
    }

    #[test]
    fn renders_opaque_rgb() {
        let dir = std::env::temp_dir().join(format!("lumi-branding-{}", std::process::id()));
        let out = dir.join("i.png");
        Renderer::new().opaque(&icon_svg(), 40, 40, &out).unwrap();
        let dec = png::Decoder::new(std::fs::File::open(&out).unwrap());
        let reader = dec.read_info().unwrap();
        assert_eq!(reader.info().color_type, png::ColorType::Rgb);
        assert_eq!((reader.info().width, reader.info().height), (40, 40));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

//! Colours as DrawingML describes them: a base colour plus transforms.

use std::collections::BTreeMap;

/// Linear-free RGBA colour, components in 0..=1 (sRGB encoded).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rgba {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Rgba {
    pub const BLACK: Rgba = Rgba { r: 0.0, g: 0.0, b: 0.0, a: 1.0 };
    pub const WHITE: Rgba = Rgba { r: 1.0, g: 1.0, b: 1.0, a: 1.0 };
    pub const TRANSPARENT: Rgba = Rgba { r: 0.0, g: 0.0, b: 0.0, a: 0.0 };

    pub fn from_u8(r: u8, g: u8, b: u8) -> Self {
        Rgba { r: r as f32 / 255.0, g: g as f32 / 255.0, b: b as f32 / 255.0, a: 1.0 }
    }

    pub fn to_u8(self) -> [u8; 4] {
        let c = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        [c(self.r), c(self.g), c(self.b), c(self.a)]
    }

    pub fn to_hex(self) -> String {
        let [r, g, b, _] = self.to_u8();
        format!("{r:02X}{g:02X}{b:02X}")
    }

    pub fn from_hex(s: &str) -> Option<Self> {
        let s = s.trim().trim_start_matches('#');
        if s.len() != 6 {
            return None;
        }
        let v = u32::from_str_radix(s, 16).ok()?;
        Some(Rgba::from_u8((v >> 16) as u8, (v >> 8) as u8, v as u8))
    }
}

/// The base of a DrawingML colour.
#[derive(Debug, Clone, PartialEq)]
pub enum ColorBase {
    /// `a:srgbClr` (hex RRGGBB).
    Srgb(String),
    /// `a:schemeClr` (`accent1`, `tx1`, `bg2`, `phClr`, ...).
    Scheme(String),
    /// `a:sysClr` with its `lastClr` cache.
    System { name: String, last: Option<String> },
    /// `a:prstClr` (CSS-like names).
    Preset(String),
    /// `a:scrgbClr` percentages (0..100000).
    ScRgb(i32, i32, i32),
    /// `a:hslClr` hue (60000ths of degree), sat/lum (0..100000).
    Hsl(i32, i32, i32),
}

/// A colour reference with its transforms (`lumMod`, `alpha`, ...), in order.
#[derive(Debug, Clone, PartialEq)]
pub struct Color {
    pub base: ColorBase,
    pub mods: Vec<(String, i32)>,
}

impl Color {
    pub fn srgb(hex: &str) -> Self {
        Color { base: ColorBase::Srgb(hex.to_uppercase()), mods: Vec::new() }
    }
    pub fn scheme(name: &str) -> Self {
        Color { base: ColorBase::Scheme(name.to_string()), mods: Vec::new() }
    }
    pub fn from_rgba(c: Rgba) -> Self {
        let mut col = Color::srgb(&c.to_hex());
        if c.a < 0.999 {
            col.mods.push(("alpha".into(), (c.a * 100_000.0).round() as i32));
        }
        col
    }

    /// Alpha transform, if any (0..=100000).
    pub fn alpha(&self) -> Option<i32> {
        self.mods.iter().find(|(k, _)| k == "alpha").map(|(_, v)| *v)
    }
}

/// The 12 colour slots of a theme (`dk1`, `lt1`, `dk2`, `lt2`, `accent1..6`,
/// `hlink`, `folHlink`), stored as resolved RGB.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ColorScheme {
    pub name: String,
    pub slots: BTreeMap<String, Rgba>,
}

/// Mapping from logical names (`bg1`, `tx1`, ...) to theme slots, from
/// `p:clrMap`.
#[derive(Debug, Clone, PartialEq)]
pub struct ClrMap(pub BTreeMap<String, String>);

impl Default for ClrMap {
    fn default() -> Self {
        let pairs = [
            ("bg1", "lt1"),
            ("tx1", "dk1"),
            ("bg2", "lt2"),
            ("tx2", "dk2"),
            ("accent1", "accent1"),
            ("accent2", "accent2"),
            ("accent3", "accent3"),
            ("accent4", "accent4"),
            ("accent5", "accent5"),
            ("accent6", "accent6"),
            ("hlink", "hlink"),
            ("folHlink", "folHlink"),
        ];
        ClrMap(pairs.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect())
    }
}

/// Context needed to turn a [`Color`] into RGBA.
pub struct ColorCtx<'a> {
    pub scheme: &'a ColorScheme,
    pub map: &'a ClrMap,
    /// Value substituted for `phClr` (style matrix references).
    pub ph_clr: Option<&'a Color>,
}

impl ColorCtx<'_> {
    pub fn resolve(&self, c: &Color) -> Rgba {
        self.resolve_depth(c, 0)
    }

    fn resolve_depth(&self, c: &Color, depth: u8) -> Rgba {
        let base = match &c.base {
            ColorBase::Srgb(h) => Rgba::from_hex(h).unwrap_or(Rgba::BLACK),
            ColorBase::Scheme(name) => {
                if name == "phClr" {
                    match self.ph_clr {
                        Some(p) if depth < 4 => self.resolve_depth(p, depth + 1),
                        _ => Rgba::BLACK,
                    }
                } else {
                    let slot = self.map.0.get(name).map(String::as_str).unwrap_or(name.as_str());
                    // `dk1`/`lt1` may also be referenced directly.
                    self.scheme.slots.get(slot).copied().unwrap_or(Rgba::BLACK)
                }
            }
            ColorBase::System { name, last } => last
                .as_deref()
                .and_then(Rgba::from_hex)
                .unwrap_or(match name.as_str() {
                    "window" => Rgba::WHITE,
                    _ => Rgba::BLACK,
                }),
            ColorBase::Preset(name) => preset_color(name),
            ColorBase::ScRgb(r, g, b) => Rgba {
                r: linear_to_srgb(*r as f32 / 100_000.0),
                g: linear_to_srgb(*g as f32 / 100_000.0),
                b: linear_to_srgb(*b as f32 / 100_000.0),
                a: 1.0,
            },
            ColorBase::Hsl(h, s, l) => {
                hsl_to_rgb(*h as f32 / 60_000.0, *s as f32 / 100_000.0, *l as f32 / 100_000.0)
            }
        };
        apply_mods(base, &c.mods)
    }
}

fn linear_to_srgb(v: f32) -> f32 {
    let v = v.clamp(0.0, 1.0);
    if v <= 0.003_130_8 {
        v * 12.92
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    }
}

fn srgb_to_linear(v: f32) -> f32 {
    if v <= 0.040_45 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

/// Apply DrawingML colour transforms in document order.
pub fn apply_mods(mut c: Rgba, mods: &[(String, i32)]) -> Rgba {
    for (k, v) in mods {
        let f = *v as f32 / 100_000.0;
        match k.as_str() {
            "alpha" => c.a = f.clamp(0.0, 1.0),
            "alphaMod" => c.a = (c.a * f).clamp(0.0, 1.0),
            "alphaOff" => c.a = (c.a + f).clamp(0.0, 1.0),
            "lumMod" | "lumOff" | "satMod" | "satOff" | "hueMod" | "hueOff" | "lum" | "sat" | "hue" => {
                let (mut h, mut s, mut l) = rgb_to_hsl(c);
                match k.as_str() {
                    "lumMod" => l *= f,
                    "lumOff" => l += f,
                    "lum" => l = f,
                    "satMod" => s *= f,
                    "satOff" => s += f,
                    "sat" => s = f,
                    "hueMod" => h *= f,
                    "hueOff" => h += *v as f32 / 60_000.0,
                    "hue" => h = *v as f32 / 60_000.0,
                    _ => {}
                }
                let a = c.a;
                c = hsl_to_rgb(h.rem_euclid(360.0), s.clamp(0.0, 1.0), l.clamp(0.0, 1.0));
                c.a = a;
            }
            // Tint/shade operate in linear RGB, as PowerPoint does.
            "tint" => {
                let t = |x: f32| linear_to_srgb(srgb_to_linear(x) * f + (1.0 - f));
                c = Rgba { r: t(c.r), g: t(c.g), b: t(c.b), a: c.a };
            }
            "shade" => {
                let t = |x: f32| linear_to_srgb(srgb_to_linear(x) * f);
                c = Rgba { r: t(c.r), g: t(c.g), b: t(c.b), a: c.a };
            }
            "comp" => {
                let (h, s, l) = rgb_to_hsl(c);
                let a = c.a;
                c = hsl_to_rgb((h + 180.0).rem_euclid(360.0), s, l);
                c.a = a;
            }
            "inv" => c = Rgba { r: 1.0 - c.r, g: 1.0 - c.g, b: 1.0 - c.b, a: c.a },
            "gray" => {
                let y = 0.2126 * c.r + 0.7152 * c.g + 0.0722 * c.b;
                c = Rgba { r: y, g: y, b: y, a: c.a };
            }
            "redMod" => c.r = (c.r * f).clamp(0.0, 1.0),
            "greenMod" => c.g = (c.g * f).clamp(0.0, 1.0),
            "blueMod" => c.b = (c.b * f).clamp(0.0, 1.0),
            "redOff" => c.r = (c.r + f).clamp(0.0, 1.0),
            "greenOff" => c.g = (c.g + f).clamp(0.0, 1.0),
            "blueOff" => c.b = (c.b + f).clamp(0.0, 1.0),
            "red" => c.r = f.clamp(0.0, 1.0),
            "green" => c.g = f.clamp(0.0, 1.0),
            "blue" => c.b = f.clamp(0.0, 1.0),
            _ => {}
        }
    }
    c
}

pub fn rgb_to_hsl(c: Rgba) -> (f32, f32, f32) {
    let max = c.r.max(c.g).max(c.b);
    let min = c.r.min(c.g).min(c.b);
    let l = (max + min) / 2.0;
    if (max - min).abs() < 1e-6 {
        return (0.0, 0.0, l);
    }
    let d = max - min;
    let s = if l > 0.5 { d / (2.0 - max - min) } else { d / (max + min) };
    let h = if max == c.r {
        (c.g - c.b) / d + if c.g < c.b { 6.0 } else { 0.0 }
    } else if max == c.g {
        (c.b - c.r) / d + 2.0
    } else {
        (c.r - c.g) / d + 4.0
    };
    (h * 60.0, s, l)
}

pub fn hsl_to_rgb(h: f32, s: f32, l: f32) -> Rgba {
    if s <= 0.0 {
        return Rgba { r: l, g: l, b: l, a: 1.0 };
    }
    let q = if l < 0.5 { l * (1.0 + s) } else { l + s - l * s };
    let p = 2.0 * l - q;
    let hk = h / 360.0;
    let t = |mut t: f32| {
        if t < 0.0 {
            t += 1.0;
        }
        if t > 1.0 {
            t -= 1.0;
        }
        if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        }
    };
    Rgba { r: t(hk + 1.0 / 3.0), g: t(hk), b: t(hk - 1.0 / 3.0), a: 1.0 }
}

fn preset_color(name: &str) -> Rgba {
    let hex = match name {
        "black" => "000000",
        "white" => "FFFFFF",
        "red" => "FF0000",
        "green" => "008000",
        "lime" => "00FF00",
        "blue" => "0000FF",
        "yellow" => "FFFF00",
        "cyan" | "aqua" => "00FFFF",
        "magenta" | "fuchsia" => "FF00FF",
        "gray" | "grey" => "808080",
        "silver" => "C0C0C0",
        "maroon" => "800000",
        "navy" => "000080",
        "olive" => "808000",
        "purple" => "800080",
        "teal" => "008080",
        "orange" => "FFA500",
        "ltGray" | "lightGray" => "D3D3D3",
        "dkGray" | "darkGray" => "A9A9A9",
        _ => "000000",
    };
    Rgba::from_hex(hex).unwrap_or(Rgba::BLACK)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lum_mod_off() {
        // accent1 4472C4 with lumMod 60000 lumOff 40000 is PowerPoint's
        // "lighter 40%" ≈ 8FAADC.
        let c = apply_mods(
            Rgba::from_hex("4472C4").unwrap_or(Rgba::BLACK),
            &[("lumMod".into(), 60000), ("lumOff".into(), 40000)],
        );
        let [r, g, b, _] = c.to_u8();
        assert!((r as i32 - 0x8F).abs() <= 2 && (g as i32 - 0xAA).abs() <= 2 && (b as i32 - 0xDC).abs() <= 2);
    }

    #[test]
    fn hex_roundtrip() {
        assert_eq!(Rgba::from_hex("1A2B3C").map(|c| c.to_hex()), Some("1A2B3C".to_string()));
    }
}

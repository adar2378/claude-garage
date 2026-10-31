//! Dev-only: a ratatui `Buffer` as an SVG (`render_frames.rs` is the caller).
//!
//! Every cell sits on a fixed grid, so box-drawing glyphs line up whatever
//! font the viewer ends up with: each text run carries each word is pinned to
//! its cell with `x` and `textLength` instead of trusting the font's advance. Backgrounds are rects
//! merged per run and row. Nothing external is embedded.
use std::fmt::Write as _;

use ratatui::buffer::Buffer;
use ratatui::style::{Color, Modifier};
use ratatui::text::Span;

pub type Rgb = (u8, u8, u8);

/// The terminal the frames are drawn on (a dark theme close to the wall's
/// own look; the wall asks for ANSI names, the terminal decides their RGB).
pub struct Theme {
    pub bg: Rgb,
    pub fg: Rgb,
    pub ansi: [Rgb; 16],
}

impl Theme {
    pub fn dark() -> Theme {
        Theme {
            bg: (0x16, 0x17, 0x1d),
            fg: (0xc8, 0xcc, 0xd8),
            ansi: [
                (0x1d, 0x1f, 0x27), // black
                (0xf7, 0x76, 0x8e), // red
                (0x9e, 0xce, 0x6a), // green
                (0xe0, 0xaf, 0x68), // yellow
                (0x7a, 0xa2, 0xf7), // blue
                (0xbb, 0x9a, 0xf7), // magenta
                (0x7d, 0xcf, 0xff), // cyan
                (0xa4, 0xa9, 0xba), // white (ANSI 7, the wall's "gray")
                (0x5b, 0x61, 0x78), // bright black ("dark gray")
                (0xff, 0x8f, 0xa3), // bright red
                (0xb4, 0xe0, 0x82), // bright green
                (0xf0, 0xc4, 0x84), // bright yellow
                (0x92, 0xb6, 0xff), // bright blue
                (0xcd, 0xb2, 0xff), // bright magenta
                (0x9a, 0xdb, 0xff), // bright cyan
                (0xee, 0xf0, 0xf6), // bright white
            ],
        }
    }

    fn indexed(&self, i: u8) -> Rgb {
        match i {
            0..=15 => self.ansi[i as usize],
            16..=231 => {
                let n = i - 16;
                let level = |v: u8| if v == 0 { 0 } else { 55 + 40 * v };
                (level(n / 36), level((n / 6) % 6), level(n % 6))
            }
            _ => {
                let g = 8 + 10 * (i - 232);
                (g, g, g)
            }
        }
    }

    /// `None` is the terminal default.
    fn resolve(&self, c: Color) -> Option<Rgb> {
        Some(match c {
            Color::Reset => return None,
            Color::Black => self.ansi[0],
            Color::Red => self.ansi[1],
            Color::Green => self.ansi[2],
            Color::Yellow => self.ansi[3],
            Color::Blue => self.ansi[4],
            Color::Magenta => self.ansi[5],
            Color::Cyan => self.ansi[6],
            Color::Gray => self.ansi[7],
            Color::DarkGray => self.ansi[8],
            Color::LightRed => self.ansi[9],
            Color::LightGreen => self.ansi[10],
            Color::LightYellow => self.ansi[11],
            Color::LightBlue => self.ansi[12],
            Color::LightMagenta => self.ansi[13],
            Color::LightCyan => self.ansi[14],
            Color::White => self.ansi[15],
            Color::Rgb(r, g, b) => (r, g, b),
            Color::Indexed(i) => self.indexed(i),
        })
    }
}

pub struct Options {
    pub font_size: u32,
    pub cell_w: u32,
    pub cell_h: u32,
    /// A macOS-style window frame around the frame. Off by default.
    pub chrome: bool,
    pub theme: Theme,
}

impl Default for Options {
    fn default() -> Options {
        // JetBrains Mono's advance is 0.6 em: 15 px gives a 9 px cell, so
        // every coordinate is an integer and rects never leave seams.
        Options {
            font_size: 15,
            cell_w: 9,
            cell_h: 20,
            chrome: false,
            theme: Theme::dark(),
        }
    }
}

const FONT: &str = "JetBrains Mono, SF Mono, Menlo, monospace";

fn hex((r, g, b): Rgb) -> String {
    format!("#{r:02x}{g:02x}{b:02x}")
}

fn esc(s: &str, out: &mut String) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            c if (c as u32) < 0x20 || c == '\u{7f}' => {}
            c => out.push(c),
        }
    }
}

struct Cell<'a> {
    symbol: &'a str,
    fg: Rgb,
    /// `None` is the page background.
    bg: Option<Rgb>,
    modifier: Modifier,
}

fn cell_at<'a>(buf: &'a Buffer, x: u16, y: u16, o: &Options) -> Cell<'a> {
    let c = &buf[(x, y)];
    let t = &o.theme;
    let mut fg = t.resolve(c.fg);
    let mut bg = t.resolve(c.bg);
    if c.modifier.contains(Modifier::REVERSED) {
        let new_bg = fg.or(Some(t.fg));
        fg = bg.or(Some(t.bg));
        bg = new_bg;
    }
    Cell {
        symbol: c.symbol(),
        fg: fg.unwrap_or(t.fg),
        bg,
        modifier: c.modifier,
    }
}

pub fn to_svg(buf: &Buffer, o: &Options) -> String {
    let area = buf.area;
    let (cw, ch) = (o.cell_w, o.cell_h);
    let w = u32::from(area.width) * cw;
    let h = u32::from(area.height) * ch;
    let (pad_top, pad) = if o.chrome { (40, 14) } else { (0, 0) };
    let (tw, th) = (w + 2 * pad, h + pad_top + pad);

    let mut s = String::new();
    let _ = writeln!(
        s,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{tw}\" height=\"{th}\" viewBox=\"0 0 {tw} {th}\" role=\"img\" aria-label=\"claude-garage wall\">"
    );
    let bg = hex(o.theme.bg);
    if o.chrome {
        let _ = write!(
            s,
            "<defs><clipPath id=\"win\"><rect width=\"{tw}\" height=\"{th}\" rx=\"12\"/></clipPath></defs>\n<g clip-path=\"url(#win)\">\n<rect width=\"{tw}\" height=\"{th}\" fill=\"{bg}\"/>\n<rect width=\"{tw}\" height=\"{pad_top}\" fill=\"#23242c\"/>\n<circle cx=\"22\" cy=\"20\" r=\"6\" fill=\"#ff5f57\"/><circle cx=\"42\" cy=\"20\" r=\"6\" fill=\"#febc2e\"/><circle cx=\"62\" cy=\"20\" r=\"6\" fill=\"#28c840\"/>\n<g transform=\"translate({pad} {pad_top})\">\n"
        );
    } else {
        let _ = writeln!(s, "<rect width=\"{tw}\" height=\"{th}\" fill=\"{bg}\"/>");
    }

    // Backgrounds: one rect per run of equal colour in a row.
    s.push_str("<g shape-rendering=\"crispEdges\">\n");
    for y in 0..area.height {
        let mut x = 0;
        while x < area.width {
            let Some(color) = cell_at(buf, x, y, o).bg else {
                x += 1;
                continue;
            };
            let start = x;
            while x < area.width && cell_at(buf, x, y, o).bg == Some(color) {
                x += 1;
            }
            let _ = writeln!(
                s,
                "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{ch}\" fill=\"{}\"/>",
                u32::from(start) * cw,
                u32::from(y) * ch,
                u32::from(x - start) * cw,
                hex(color)
            );
        }
    }
    s.push_str("</g>\n");

    // Box-drawing lines are drawn as strokes edge to edge, so borders are
    // seamless in any font (a font's own glyphs rarely span the full line
    // height). One path per colour for the whole frame.
    let mut strokes: std::collections::BTreeMap<(Rgb, bool, bool), String> = Default::default();
    for y in 0..area.height {
        for x in 0..area.width {
            let c = cell_at(buf, x, y, o);
            let Some((arms, heavy)) = box_arms(c.symbol) else {
                continue;
            };
            let (x0, y0) = (f64::from(u32::from(x) * cw), f64::from(u32::from(y) * ch));
            let (x1, y1) = (x0 + f64::from(cw), y0 + f64::from(ch));
            // Light strokes sit on half pixels, heavy (2 px) ones on whole pixels:
            // both land crisp.
            let off = if heavy { 0.0 } else { 0.5 };
            let (cx, cy) = (
                (x0 + f64::from(cw) / 2.0).floor() + off,
                y0 + f64::from(ch / 2) + off,
            );
            let d = strokes
                .entry((c.fg, c.modifier.contains(Modifier::DIM), heavy))
                .or_default();
            let r = f64::from(cw) / 2.0;
            match arms {
                Arms::Rounded { right, down } => {
                    let (ex, sx) = if right { (cx + r, x1) } else { (cx - r, x0) };
                    let (ey, sy) = if down { (cy + r, y1) } else { (cy - r, y0) };
                    let _ = write!(
                        d,
                        "M{sx} {cy}H{ex}A{r} {r} 0 0 {} {cx} {ey}V{sy}",
                        // sweep: clockwise when the corner turns the same way
                        if right == down { 0 } else { 1 }
                    );
                }
                Arms::Lines { l, r, u, d: dn } => {
                    if l && r {
                        let _ = write!(d, "M{x0} {cy}H{x1}");
                    } else if l {
                        let _ = write!(d, "M{x0} {cy}H{cx}");
                    } else if r {
                        let _ = write!(d, "M{cx} {cy}H{x1}");
                    }
                    if u && dn {
                        let _ = write!(d, "M{cx} {y0}V{y1}");
                    } else if u {
                        let _ = write!(d, "M{cx} {y0}V{cy}");
                    } else if dn {
                        let _ = write!(d, "M{cx} {cy}V{y1}");
                    }
                }
            }
        }
    }
    for ((color, dim, heavy), d) in &strokes {
        let _ = writeln!(
            s,
            "<path d=\"{d}\" fill=\"none\" stroke=\"{}\" stroke-width=\"{}\"{}/>",
            hex(*color),
            if *heavy { 2 } else { 1 },
            if *dim { " stroke-opacity=\"0.6\"" } else { "" }
        );
    }

    // Text: words, split at spaces and style changes.
    let _ = writeln!(
        s,
        "<g font-family=\"{FONT}\" font-size=\"{}\" xml:space=\"preserve\">",
        o.font_size
    );
    let key_mods = Modifier::BOLD
        | Modifier::DIM
        | Modifier::ITALIC
        | Modifier::UNDERLINED
        | Modifier::CROSSED_OUT;
    for y in 0..area.height {
        let baseline = u32::from(y) * ch + ch * 3 / 4 + 1;
        let mut x = 0;
        while x < area.width {
            let first = cell_at(buf, x, y, o);
            let key = (first.fg, first.modifier & key_mods);
            // Words of this run: (start column, text, columns). One element
            // per word, pinned at its first cell and stretched to its exact
            // width, so a font with odd advances cannot push anything off the
            // grid.
            let mut words: Vec<(u32, String, u32)> = Vec::new();
            let mut open: Option<(u32, String, u32)> = None;
            while x < area.width {
                let c = cell_at(buf, x, y, o);
                if (c.fg, c.modifier & key_mods) != key {
                    break;
                }
                let cols = if Span::raw(c.symbol).width() > 1 {
                    2
                } else {
                    1
                };
                if c.symbol.trim().is_empty() || box_arms(c.symbol).is_some() {
                    words.extend(open.take());
                } else {
                    let w = open.get_or_insert((u32::from(x), String::new(), 0));
                    esc(c.symbol, &mut w.1);
                    w.2 += cols;
                }
                x += cols as u16;
            }
            words.extend(open.take());
            let attrs = {
                let mut a = format!(" fill=\"{}\"", hex(key.0));
                if key.1.contains(Modifier::BOLD) {
                    a.push_str(" font-weight=\"700\"");
                }
                if key.1.contains(Modifier::ITALIC) {
                    a.push_str(" font-style=\"italic\"");
                }
                if key.1.contains(Modifier::DIM) {
                    a.push_str(" fill-opacity=\"0.6\"");
                }
                match (
                    key.1.contains(Modifier::UNDERLINED),
                    key.1.contains(Modifier::CROSSED_OUT),
                ) {
                    (true, true) => a.push_str(" text-decoration=\"underline line-through\""),
                    (true, false) => a.push_str(" text-decoration=\"underline\""),
                    (false, true) => a.push_str(" text-decoration=\"line-through\""),
                    _ => {}
                }
                a
            };
            for (col, text, cols) in words {
                let fit = if cols > 1 {
                    format!(" textLength=\"{}\" lengthAdjust=\"spacing\"", cols * cw)
                } else {
                    String::new()
                };
                let _ = writeln!(
                    s,
                    "<text x=\"{}\" y=\"{baseline}\"{fit}{attrs}>{text}</text>",
                    col * cw
                );
            }
        }
    }
    s.push_str("</g>\n");
    if o.chrome {
        s.push_str("</g>\n</g>\n");
    }
    s.push_str("</svg>\n");
    s
}

enum Arms {
    Lines {
        l: bool,
        r: bool,
        u: bool,
        d: bool,
    },
    /// `╭ ╮ ╰ ╯`: which way the horizontal and vertical arms point.
    Rounded {
        right: bool,
        down: bool,
    },
}

/// The box-drawing characters the wall uses (light, rounded and heavy), as
/// arms to draw and whether they are heavy.
fn box_arms(sym: &str) -> Option<(Arms, bool)> {
    let l = |l, r, u, d, h| Some((Arms::Lines { l, r, u, d }, h));
    let rd = |right, down| Some((Arms::Rounded { right, down }, false));
    match sym {
        "─" => l(true, true, false, false, false),
        "│" => l(false, false, true, true, false),
        "┌" => l(false, true, false, true, false),
        "┐" => l(true, false, false, true, false),
        "└" => l(false, true, true, false, false),
        "┘" => l(true, false, true, false, false),
        "├" => l(false, true, true, true, false),
        "┤" => l(true, false, true, true, false),
        "┬" => l(true, true, false, true, false),
        "┴" => l(true, true, true, false, false),
        "┼" => l(true, true, true, true, false),
        "━" => l(true, true, false, false, true),
        "┃" => l(false, false, true, true, true),
        "┏" => l(false, true, false, true, true),
        "┓" => l(true, false, false, true, true),
        "┗" => l(false, true, true, false, true),
        "┛" => l(true, false, true, false, true),
        "┣" => l(false, true, true, true, true),
        "┫" => l(true, false, true, true, true),
        "┳" => l(true, true, false, true, true),
        "┻" => l(true, true, true, false, true),
        "╋" => l(true, true, true, true, true),
        "╭" => rd(true, true),
        "╮" => rd(false, true),
        "╰" => rd(true, false),
        "╯" => rd(false, false),
        _ => None,
    }
}

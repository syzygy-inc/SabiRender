//! SVG 後端。描画命令列を SVG 文書にする。ブラウザに渡す装置非依存の出力で、字形も経路として書く。
//!
//! - 座標: ページ空間（bp、y 上向き）を `viewBox` の中で y 反転して置く。1 利用者単位 = 1 bp
//! - 塗り・線は `<path>`。線は利用者空間で太らせるため、`transform` に CTM を書き、`d` は利用者空間のまま
//! - クリップは `<clipPath>` と入れ子の `<g clip-path>`
//! - 字形は経路にした `<path>`。フォントには依存しない
//! - 画像と未対応の項目は注釈（`<!-- -->`）として残す

use sabirender_display::{
    Color, DisplayList, FillRule, Item, LineCap, LineJoin, Matrix, Path, Segment, StrokeStyle,
};
use std::fmt::Write;

/// 出力の範囲（ページ空間、bp）
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bounds {
    pub xmin: f64,
    pub ymin: f64,
    pub xmax: f64,
    pub ymax: f64,
}

impl Bounds {
    pub fn width(&self) -> f64 {
        self.xmax - self.xmin
    }
    pub fn height(&self) -> f64 {
        self.ymax - self.ymin
    }
    fn include(&mut self, x: f64, y: f64) {
        self.xmin = self.xmin.min(x);
        self.ymin = self.ymin.min(y);
        self.xmax = self.xmax.max(x);
        self.ymax = self.ymax.max(y);
    }
    fn expand(&mut self, d: f64) {
        self.xmin -= d;
        self.ymin -= d;
        self.xmax += d;
        self.ymax += d;
    }
}

/// 描画命令列のインクの範囲（制御点の包絡と線幅から。厳密な極値は取らない）。空なら None
pub fn bounds(list: &DisplayList) -> Option<Bounds> {
    let mut b: Option<Bounds> = None;
    let mut include_path = |path: &Path, ctm: &Matrix, pad: f64| {
        let p = path.transform(ctm);
        for s in &p.segments {
            let pts: &[(f64, f64)] = match *s {
                Segment::MoveTo(x, y) | Segment::LineTo(x, y) => &[(x, y)],
                Segment::CurveTo(a, c, d, e, f, g) => &[(a, c), (d, e), (f, g)],
                Segment::Close => &[],
            };
            for &(x, y) in pts {
                match b.as_mut() {
                    Some(bb) => bb.include(x, y),
                    None => {
                        b = Some(Bounds {
                            xmin: x,
                            ymin: y,
                            xmax: x,
                            ymax: y,
                        })
                    }
                }
            }
        }
        if let Some(bb) = b.as_mut() {
            if pad > 0.0 {
                bb.expand(pad);
            }
        }
    };
    for item in &list.items {
        match item {
            Item::Fill { path, ctm, .. } => include_path(path, ctm, 0.0),
            Item::Stroke {
                path, ctm, style, ..
            } => {
                // 尖り接合まで含めると miter_limit 倍だが、包絡としては線幅の半分で十分
                let pad = style.width
                    * 0.5
                    * ctm.mean_scale()
                    * if style.join == LineJoin::Miter {
                        style.miter_limit.min(4.0)
                    } else {
                        1.0
                    };
                include_path(path, ctm, pad)
            }
            Item::Glyphs { run, ctm, .. } => {
                for g in &run.glyphs {
                    let m = Matrix::scale(run.scale, run.scale)
                        .then(&Matrix::translate(g.x, g.y))
                        .then(ctm);
                    include_path(&g.outline, &m, 0.0);
                }
            }
            Item::Image { ctm, .. } => include_path(&Path::rect(0.0, 0.0, 1.0, 1.0), ctm, 0.0),
            Item::ClipPush { .. } | Item::ClipPop | Item::Unsupported { .. } => {}
        }
    }
    b
}

pub struct SvgOptions {
    /// 出力範囲。None なら `bounds()` に `margin` を足したもの
    pub bounds: Option<Bounds>,
    pub margin: f64,
    /// 小数の桁数
    pub precision: usize,
    /// `<svg>` に書く `width` / `height` の単位。None なら書かない（viewBox だけ）
    pub size_unit: Option<&'static str>,
}

impl Default for SvgOptions {
    fn default() -> Self {
        SvgOptions {
            bounds: None,
            margin: 0.0,
            precision: 3,
            size_unit: Some("pt"),
        }
    }
}

fn num(v: f64, p: usize) -> String {
    let s = format!("{v:.p$}");
    let s = if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        s
    };
    if s == "-0" {
        "0".into()
    } else {
        s
    }
}

fn color(c: &Color) -> String {
    let (r, g, b) = c.to_rgb();
    let q = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{:02x}{:02x}{:02x}", q(r), q(g), q(b))
}

fn path_d(path: &Path, p: usize) -> String {
    let mut d = String::new();
    for s in &path.segments {
        match *s {
            Segment::MoveTo(x, y) => write!(d, "M{} {}", num(x, p), num(y, p)).unwrap(),
            Segment::LineTo(x, y) => write!(d, "L{} {}", num(x, p), num(y, p)).unwrap(),
            Segment::CurveTo(a, b, c, e, f, g) => write!(
                d,
                "C{} {} {} {} {} {}",
                num(a, p),
                num(b, p),
                num(c, p),
                num(e, p),
                num(f, p),
                num(g, p)
            )
            .unwrap(),
            Segment::Close => d.push('Z'),
        }
    }
    d
}

fn matrix(m: &Matrix, p: usize) -> String {
    format!(
        "matrix({} {} {} {} {} {})",
        num(m.a, p),
        num(m.b, p),
        num(m.c, p),
        num(m.d, p),
        num(m.e, p),
        num(m.f, p)
    )
}

fn stroke_attrs(style: &StrokeStyle, p: usize) -> String {
    let mut s = format!(" stroke-width=\"{}\"", num(style.width, p));
    match style.cap {
        LineCap::Butt => {}
        LineCap::Round => s.push_str(" stroke-linecap=\"round\""),
        LineCap::Square => s.push_str(" stroke-linecap=\"square\""),
    }
    match style.join {
        LineJoin::Miter => {
            if (style.miter_limit - 4.0).abs() > 1e-9 {
                write!(s, " stroke-miterlimit=\"{}\"", num(style.miter_limit, p)).unwrap();
            }
        }
        LineJoin::Round => s.push_str(" stroke-linejoin=\"round\""),
        LineJoin::Bevel => s.push_str(" stroke-linejoin=\"bevel\""),
    }
    if !style.dash.is_empty() {
        let d: Vec<String> = style.dash.iter().map(|v| num(*v, p)).collect();
        write!(s, " stroke-dasharray=\"{}\"", d.join(" ")).unwrap();
        if style.dash_phase != 0.0 {
            write!(s, " stroke-dashoffset=\"{}\"", num(style.dash_phase, p)).unwrap();
        }
    }
    s
}

/// 描画命令列を SVG 文書にする。範囲が決まらなければ（何も描かない）空の 1×1 を返す
pub fn to_svg(list: &DisplayList, opts: &SvgOptions) -> String {
    let p = opts.precision;
    let b = opts.bounds.or_else(|| {
        bounds(list).map(|mut b| {
            b.expand(opts.margin);
            b
        })
    });
    let b = b.unwrap_or(Bounds {
        xmin: 0.0,
        ymin: 0.0,
        xmax: 1.0,
        ymax: 1.0,
    });
    let (w, h) = (b.width().max(1e-6), b.height().max(1e-6));
    let mut out = String::new();
    write!(
        out,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {} {}\"",
        num(w, p),
        num(h, p)
    )
    .unwrap();
    if let Some(u) = opts.size_unit {
        write!(
            out,
            " width=\"{}{u}\" height=\"{}{u}\"",
            num(w, p),
            num(h, p)
        )
        .unwrap();
    }
    out.push('>');
    // ページ空間 → SVG: x' = x - xmin, y' = ymax - y
    write!(
        out,
        "<g transform=\"matrix(1 0 0 -1 {} {})\">",
        num(-b.xmin, p),
        num(b.ymax, p)
    )
    .unwrap();
    let mut clip_id = 0usize;
    let mut open_groups = 0usize;
    for item in &list.items {
        match item {
            Item::Fill {
                path,
                ctm,
                rule,
                color: c,
                alpha,
            } => {
                write!(
                    out,
                    "<path d=\"{}\" transform=\"{}\" fill=\"{}\"",
                    path_d(path, p),
                    matrix(ctm, p),
                    color(c)
                )
                .unwrap();
                if *rule == FillRule::EvenOdd {
                    out.push_str(" fill-rule=\"evenodd\"");
                }
                if *alpha < 1.0 {
                    write!(out, " fill-opacity=\"{}\"", num(*alpha, p)).unwrap();
                }
                out.push_str("/>");
            }
            Item::Stroke {
                path,
                ctm,
                style,
                color: c,
                alpha,
            } => {
                write!(
                    out,
                    "<path d=\"{}\" transform=\"{}\" fill=\"none\" stroke=\"{}\"{}",
                    path_d(path, p),
                    matrix(ctm, p),
                    color(c),
                    stroke_attrs(style, p)
                )
                .unwrap();
                if *alpha < 1.0 {
                    write!(out, " stroke-opacity=\"{}\"", num(*alpha, p)).unwrap();
                }
                out.push_str("/>");
            }
            Item::ClipPush { path, ctm, rule } => {
                clip_id += 1;
                write!(
                    out,
                    "<clipPath id=\"c{clip_id}\"><path d=\"{}\" transform=\"{}\"",
                    path_d(path, p),
                    matrix(ctm, p)
                )
                .unwrap();
                if *rule == FillRule::EvenOdd {
                    out.push_str(" clip-rule=\"evenodd\"");
                }
                write!(out, "/></clipPath><g clip-path=\"url(#c{clip_id})\">").unwrap();
                open_groups += 1;
            }
            Item::ClipPop => {
                if open_groups > 0 {
                    out.push_str("</g>");
                    open_groups -= 1;
                }
            }
            Item::Glyphs {
                run,
                ctm,
                color: c,
                alpha,
            } => {
                write!(
                    out,
                    "<g transform=\"{}\" fill=\"{}\"",
                    matrix(ctm, p),
                    color(c)
                )
                .unwrap();
                if *alpha < 1.0 {
                    write!(out, " fill-opacity=\"{}\"", num(*alpha, p)).unwrap();
                }
                out.push('>');
                for g in &run.glyphs {
                    if g.outline.is_empty() {
                        continue;
                    }
                    let m = Matrix::scale(run.scale, run.scale).then(&Matrix::translate(g.x, g.y));
                    write!(
                        out,
                        "<path d=\"{}\" transform=\"{}\"/>",
                        path_d(&g.outline, p),
                        matrix(&m, p)
                    )
                    .unwrap();
                }
                out.push_str("</g>");
            }
            Item::Image { width, height, .. } => {
                write!(out, "<!-- image {width}x{height} not emitted -->").unwrap()
            }
            Item::Unsupported { what } => {
                write!(out, "<!-- unsupported: {} -->", what.replace("--", "- -")).unwrap()
            }
        }
    }
    for _ in 0..open_groups {
        out.push_str("</g>");
    }
    out.push_str("</g></svg>");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_paths_clips_and_glyphs_with_flipped_y() {
        let list = DisplayList {
            items: vec![
                Item::ClipPush {
                    path: Path::rect(0.0, 0.0, 10.0, 10.0),
                    ctm: Matrix::IDENTITY,
                    rule: FillRule::NonZero,
                },
                Item::Fill {
                    path: Path::rect(1.0, 2.0, 3.0, 4.0),
                    ctm: Matrix::IDENTITY,
                    rule: FillRule::EvenOdd,
                    color: Color::Rgb(1.0, 0.0, 0.0),
                    alpha: 0.5,
                },
                Item::Stroke {
                    path: Path {
                        segments: vec![Segment::MoveTo(0.0, 0.0), Segment::LineTo(5.0, 5.0)],
                    },
                    ctm: Matrix::scale(2.0, 1.0),
                    style: StrokeStyle {
                        width: 1.5,
                        cap: LineCap::Round,
                        dash: vec![2.0, 1.0],
                        ..Default::default()
                    },
                    color: Color::BLACK,
                    alpha: 1.0,
                },
                Item::ClipPop,
            ],
        };
        let svg = to_svg(
            &list,
            &SvgOptions {
                size_unit: None,
                ..Default::default()
            },
        );
        assert!(svg.starts_with("<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 "));
        assert!(svg.contains("<clipPath id=\"c1\">"));
        assert!(svg.contains("fill=\"#ff0000\" fill-rule=\"evenodd\" fill-opacity=\"0.5\""));
        assert!(svg.contains("stroke-linecap=\"round\""));
        assert!(svg.contains("stroke-dasharray=\"2 1\""));
        assert!(svg.contains("transform=\"matrix(2 0 0 1 0 0)\""));
        assert!(svg.contains("matrix(1 0 0 -1 "));
        assert_eq!(svg.matches("<g").count(), svg.matches("</g>").count());
    }

    #[test]
    fn bounds_cover_strokes_and_glyphs() {
        let list = DisplayList {
            items: vec![
                Item::Stroke {
                    path: Path {
                        segments: vec![Segment::MoveTo(10.0, 10.0), Segment::LineTo(20.0, 10.0)],
                    },
                    ctm: Matrix::IDENTITY,
                    style: StrokeStyle {
                        width: 2.0,
                        cap: LineCap::Butt,
                        join: LineJoin::Round,
                        ..Default::default()
                    },
                    color: Color::BLACK,
                    alpha: 1.0,
                },
                Item::Glyphs {
                    run: sabirender_display::GlyphRun {
                        glyphs: vec![sabirender_display::PlacedGlyph {
                            outline: Path::rect(0.0, 0.0, 500.0, 700.0),
                            x: 30.0,
                            y: 0.0,
                        }],
                        scale: 0.01,
                    },
                    ctm: Matrix::IDENTITY,
                    color: Color::BLACK,
                    alpha: 1.0,
                },
            ],
        };
        let b = bounds(&list).unwrap();
        assert!((b.xmin - 9.0).abs() < 1e-9 && (b.ymin - 0.0).abs() < 1e-9);
        assert!((b.xmax - 35.0).abs() < 1e-9 && (b.ymax - 11.0).abs() < 1e-9);
    }
}

//! CPU 参照ラスタライザ。描画命令列（`sabirender-display`）を RGBA の画素に描く。
//!
//! 目的は「正しさの基準」であって速さではない。GPU 後端はこの出力と許容誤差内で一致することを検証される。
//!
//! - 経路は三次ベジエを許容誤差付きで折れ線化する（`flatten`）。
//! - 塗りは走査線法。縦方向は画素あたり `SUBSAMPLES` 本の副走査線で標本化し、横方向は交点位置から被覆率を厳密に積む。
//!   非零規則と偶奇規則の両方を扱う。
//! - クリップは被覆率を掛けるのではなく、副走査線ごとに塗りの区間とクリップの区間の交叉を取ってから被覆率にする
//!   （被覆率の積では、同じ形のクリップを重ねるたびに境界が薄くなる）。
//! - 線は利用者空間で太らせて多角形にし（`stroke`）、非零規則で塗る。接合（miter / round / bevel）、端点（butt / round / square）、
//!   破線に対応する。
//! - 合成は source-over。色は sRGB 相当の 0..1 で保持し、出力時に 8 ビットへ丸める。

pub mod flatten;
pub mod png;
pub mod stroke;

use sabirender_display::{DisplayList, FillRule, Item, Matrix, Path};

/// 縦方向の副走査線の数。誤差は 1/SUBSAMPLES 画素以下
pub const SUBSAMPLES: usize = 16;

/// 折れ線化の許容誤差（装置画素）
pub const FLATNESS: f64 = 0.05;

#[derive(Debug, Clone)]
pub struct Canvas {
    pub width: usize,
    pub height: usize,
    /// 行優先、画素あたり RGBA（プリマルチプライしない）0..1
    pub pixels: Vec<[f64; 4]>,
}

impl Canvas {
    pub fn new(width: usize, height: usize) -> Canvas {
        Canvas {
            width,
            height,
            pixels: vec![[0.0, 0.0, 0.0, 0.0]; width * height],
        }
    }

    pub fn filled(width: usize, height: usize, rgba: [f64; 4]) -> Canvas {
        Canvas {
            width,
            height,
            pixels: vec![rgba; width * height],
        }
    }

    pub fn to_rgba8(&self) -> Vec<u8> {
        self.pixels
            .iter()
            .flat_map(|p| p.iter().map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8))
            .collect()
    }

    /// 被覆率と不透明度で色を source-over 合成する
    fn blend(&mut self, x: usize, y: usize, rgb: (f64, f64, f64), a: f64) {
        if a <= 0.0 {
            return;
        }
        let p = &mut self.pixels[y * self.width + x];
        let da = p[3];
        let oa = a + da * (1.0 - a);
        if oa <= 0.0 {
            return;
        }
        let mix = |s: f64, d: f64| (s * a + d * da * (1.0 - a)) / oa;
        *p = [mix(rgb.0, p[0]), mix(rgb.1, p[1]), mix(rgb.2, p[2]), oa];
    }

    /// 全画素のアルファの和（面積の検証用）
    pub fn coverage_sum(&self) -> f64 {
        self.pixels.iter().map(|p| p[3]).sum()
    }
}

/// 被覆率のマスク
#[derive(Debug, Clone)]
pub struct Mask {
    pub width: usize,
    pub height: usize,
    pub values: Vec<f64>,
}

/// 折れ線化した閉多角形の集合（装置空間）。各多角形は頂点列で、最後の頂点から最初へ暗黙に閉じる
pub type Polygons = Vec<Vec<(f64, f64)>>;

/// 経路を装置空間の多角形に折れ線化する。開いた部分経路も塗りのために閉じる
pub fn flatten_path(path: &Path, to_device: &Matrix) -> Polygons {
    flatten::flatten(&path.transform(to_device), FLATNESS)
}

struct Edge {
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
    dir: i32,
}

struct EdgeSet {
    edges: Vec<Edge>,
    rule: FillRule,
    ymin: f64,
    ymax: f64,
}

fn edge_set(polys: &Polygons, rule: FillRule) -> EdgeSet {
    let mut edges = Vec::new();
    for poly in polys {
        let n = poly.len();
        if n < 2 {
            continue;
        }
        for i in 0..n {
            let (ax, ay) = poly[i];
            let (bx, by) = poly[(i + 1) % n];
            if ay == by {
                continue;
            }
            if ay < by {
                edges.push(Edge {
                    x0: ax,
                    y0: ay,
                    x1: bx,
                    y1: by,
                    dir: 1,
                });
            } else {
                edges.push(Edge {
                    x0: bx,
                    y0: by,
                    x1: ax,
                    y1: ay,
                    dir: -1,
                });
            }
        }
    }
    let ymin = edges.iter().map(|e| e.y0).fold(f64::INFINITY, f64::min);
    let ymax = edges.iter().map(|e| e.y1).fold(f64::NEG_INFINITY, f64::max);
    EdgeSet {
        edges,
        rule,
        ymin,
        ymax,
    }
}

/// 副走査線 `sy` 上で内側になる区間（昇順、互いに素）
fn spans(set: &EdgeSet, sy: f64, crossings: &mut Vec<(f64, i32)>, out: &mut Vec<(f64, f64)>) {
    crossings.clear();
    out.clear();
    for e in &set.edges {
        if sy >= e.y0 && sy < e.y1 {
            let t = (sy - e.y0) / (e.y1 - e.y0);
            crossings.push((e.x0 + (e.x1 - e.x0) * t, e.dir));
        }
    }
    if crossings.is_empty() {
        return;
    }
    crossings.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    let mut winding = 0;
    for i in 0..crossings.len() - 1 {
        winding += crossings[i].1;
        let inside = match set.rule {
            FillRule::NonZero => winding != 0,
            FillRule::EvenOdd => winding % 2 != 0,
        };
        if inside && crossings[i + 1].0 > crossings[i].0 {
            // 隣接する区間はつなぐ
            if let Some(last) = out.last_mut() {
                if last.1 >= crossings[i].0 {
                    last.1 = crossings[i + 1].0;
                    continue;
                }
            }
            out.push((crossings[i].0, crossings[i + 1].0));
        }
    }
}

/// 二つの区間列の交叉
fn intersect_spans(a: &[(f64, f64)], b: &[(f64, f64)], out: &mut Vec<(f64, f64)>) {
    out.clear();
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        let l = a[i].0.max(b[j].0);
        let r = a[i].1.min(b[j].1);
        if r > l {
            out.push((l, r));
        }
        if a[i].1 < b[j].1 {
            i += 1;
        } else {
            j += 1;
        }
    }
}

/// 多角形の集合の交叉を被覆率のマスクに描く。`sets[0]` が塗る形、以降がクリップ
pub fn rasterize_intersection(sets: &[(&Polygons, FillRule)], width: usize, height: usize) -> Mask {
    let mut mask = Mask {
        width,
        height,
        values: vec![0.0; width * height],
    };
    if sets.is_empty() {
        return mask;
    }
    let edge_sets: Vec<EdgeSet> = sets.iter().map(|(p, r)| edge_set(p, *r)).collect();
    if edge_sets.iter().any(|s| s.edges.is_empty()) {
        return mask;
    }
    let ymin = edge_sets
        .iter()
        .map(|s| s.ymin)
        .fold(f64::NEG_INFINITY, f64::max)
        .max(0.0);
    let ymax = edge_sets
        .iter()
        .map(|s| s.ymax)
        .fold(f64::INFINITY, f64::min)
        .min(height as f64);
    if ymin >= ymax {
        return mask;
    }
    let row0 = ymin.floor() as usize;
    let row1 = (ymax.ceil() as usize).min(height);
    let weight = 1.0 / SUBSAMPLES as f64;
    let mut crossings: Vec<(f64, i32)> = Vec::new();
    let mut cur: Vec<(f64, f64)> = Vec::new();
    let mut other: Vec<(f64, f64)> = Vec::new();
    let mut tmp: Vec<(f64, f64)> = Vec::new();
    let mut acc = vec![0.0f64; width + 1];
    for row in row0..row1 {
        acc.iter_mut().for_each(|v| *v = 0.0);
        let mut touched = false;
        for s in 0..SUBSAMPLES {
            let sy = row as f64 + (s as f64 + 0.5) * weight;
            spans(&edge_sets[0], sy, &mut crossings, &mut cur);
            for set in &edge_sets[1..] {
                if cur.is_empty() {
                    break;
                }
                spans(set, sy, &mut crossings, &mut other);
                intersect_spans(&cur, &other, &mut tmp);
                std::mem::swap(&mut cur, &mut tmp);
            }
            for &(xa, xb) in &cur {
                let xa = xa.max(0.0);
                let xb = xb.min(width as f64);
                if xb <= xa {
                    continue;
                }
                touched = true;
                // [xa, xb) の区間を画素ごとに厳密に積む
                let ia = xa.floor() as usize;
                let ib = (xb.ceil() as usize).min(width);
                for (px, a) in acc.iter_mut().enumerate().take(ib).skip(ia) {
                    let l = xa.max(px as f64);
                    let r = xb.min(px as f64 + 1.0);
                    if r > l {
                        *a += (r - l) * weight;
                    }
                }
            }
        }
        if touched {
            let base = row * width;
            for (dst, a) in mask.values[base..base + width].iter_mut().zip(&acc) {
                if *a > 0.0 {
                    *dst = a.min(1.0);
                }
            }
        }
    }
    mask
}

/// 多角形の集合を被覆率のマスクに描く
pub fn rasterize(polys: &Polygons, rule: FillRule, width: usize, height: usize) -> Mask {
    rasterize_intersection(&[(polys, rule)], width, height)
}

/// 描画命令列をキャンバスに描く。`page_to_device` はページ空間（bp、y 上向き）から画素座標（y 下向き）への行列
pub fn render(list: &DisplayList, canvas: &mut Canvas, page_to_device: &Matrix) -> RenderReport {
    let (w, h) = (canvas.width, canvas.height);
    // クリップは装置空間の多角形として積み、塗るたびに交叉を取る
    let mut clips: Vec<(Polygons, FillRule)> = Vec::new();
    let mut report = RenderReport::default();
    let paint = |canvas: &mut Canvas,
                 polys: &Polygons,
                 rule: FillRule,
                 clips: &[(Polygons, FillRule)],
                 rgb: (f64, f64, f64),
                 alpha: f64| {
        let mut sets: Vec<(&Polygons, FillRule)> = Vec::with_capacity(clips.len() + 1);
        sets.push((polys, rule));
        for (p, r) in clips {
            sets.push((p, *r));
        }
        let mask = rasterize_intersection(&sets, w, h);
        composite(canvas, &mask, rgb, alpha);
    };
    for item in &list.items {
        match item {
            Item::Fill {
                path,
                ctm,
                rule,
                color,
                alpha,
            } => {
                let dev = ctm.then(page_to_device);
                paint(
                    canvas,
                    &flatten_path(path, &dev),
                    *rule,
                    &clips,
                    color.to_rgb(),
                    *alpha,
                );
            }
            Item::Stroke {
                path,
                ctm,
                style,
                color,
                alpha,
            } => {
                let dev = ctm.then(page_to_device);
                let polys = stroke::stroke_to_polygons(path, style, &dev, FLATNESS);
                paint(
                    canvas,
                    &polys,
                    FillRule::NonZero,
                    &clips,
                    color.to_rgb(),
                    *alpha,
                );
            }
            Item::ClipPush { path, ctm, rule } => {
                let dev = ctm.then(page_to_device);
                clips.push((flatten_path(path, &dev), *rule));
            }
            Item::ClipPop => {
                clips.pop();
            }
            Item::Glyphs {
                run,
                ctm,
                color,
                alpha,
            } => {
                let dev = ctm.then(page_to_device);
                let mut polys = Vec::new();
                for g in &run.glyphs {
                    polys.extend(flatten_path(&g.outline, &g.transform.then(&dev)));
                }
                paint(
                    canvas,
                    &polys,
                    FillRule::NonZero,
                    &clips,
                    color.to_rgb(),
                    *alpha,
                );
            }
            Item::Image { .. } => report.skipped.push("image".into()),
            Item::Unsupported { what } => report.skipped.push(what.clone()),
        }
    }
    report
}

#[derive(Debug, Default, Clone)]
pub struct RenderReport {
    /// 描かなかった項目
    pub skipped: Vec<String>,
}

fn composite(canvas: &mut Canvas, mask: &Mask, rgb: (f64, f64, f64), alpha: f64) {
    for y in 0..canvas.height {
        for x in 0..canvas.width {
            let cov = mask.values[y * canvas.width + x];
            if cov > 0.0 {
                canvas.blend(x, y, rgb, cov * alpha);
            }
        }
    }
}

/// ページ空間（bp、原点左下）を、`dpi` の画素座標（原点左上）に写す行列
pub fn page_to_device(page_height_bp: f64, dpi: f64) -> Matrix {
    let s = dpi / 72.0;
    Matrix::new(s, 0.0, 0.0, -s, 0.0, page_height_bp * s)
}

/// 二つのキャンバスの差。許容誤差の判定に使う
pub fn diff(a: &Canvas, b: &Canvas) -> Diff {
    assert_eq!((a.width, a.height), (b.width, b.height));
    let mut max = 0.0f64;
    let mut sum = 0.0;
    let mut over = 0usize;
    for (p, q) in a.pixels.iter().zip(&b.pixels) {
        let d = (0..4).map(|k| (p[k] - q[k]).abs()).fold(0.0, f64::max);
        max = max.max(d);
        sum += d;
        if d > 1.0 / 255.0 {
            over += 1;
        }
    }
    Diff {
        max,
        mean: sum / (a.pixels.len().max(1) as f64),
        pixels_over_one_level: over,
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Diff {
    pub max: f64,
    pub mean: f64,
    pub pixels_over_one_level: usize,
}

pub use sabirender_display as display;

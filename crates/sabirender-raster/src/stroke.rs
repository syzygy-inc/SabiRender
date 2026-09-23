//! 線の多角形化。利用者空間で太らせ、装置空間へ変換した多角形の集合を返す。非零規則で塗ると線になる。
//!
//! 各部品（線分の矩形、接合の扇形または楔、端点の半円または矩形）は独立の閉多角形として出し、向きを反時計回りに揃える。
//! 部品が重なっても非零規則では和集合になる。

use crate::flatten::{flatten_open, Polyline};
use sabirender_display::{LineCap, LineJoin, Matrix, Path, StrokeStyle};

pub fn stroke_to_polygons(
    path: &Path,
    style: &StrokeStyle,
    to_device: &Matrix,
    tolerance: f64,
) -> Vec<Vec<(f64, f64)>> {
    // 折れ線化は装置空間の許容誤差で行いたいので、利用者空間の許容誤差に換算する
    let scale = to_device.mean_scale().max(1e-9);
    let user_tol = tolerance / scale;
    let mut lines = flatten_open(path, user_tol);
    if !style.dash.is_empty() {
        lines = apply_dash(&lines, &style.dash, style.dash_phase);
    }
    let hw = style.width * 0.5;
    // 線幅 0 は装置の最細線（PDF §8.4.3.2）。1 画素相当にする
    let hw = if hw <= 0.0 { 0.5 / scale } else { hw };
    let mut out: Vec<Vec<(f64, f64)>> = Vec::new();
    // 丸い接合・端点の分割数は装置空間の線幅から決める
    let round_steps = ((hw * scale * 2.0).sqrt() * 4.0).clamp(4.0, 64.0) as usize;
    for line in &lines {
        let pts = dedup(&line.points);
        if pts.is_empty() {
            continue;
        }
        if pts.len() == 1 {
            // 長さ 0 の線: 丸端点なら点、角端点なら正方形、平端点なら何も描かない（PDF §8.4.3.3）
            let p = pts[0];
            match style.cap {
                LineCap::Round => out.push(circle(p, hw, round_steps)),
                LineCap::Square if !line.closed => out.push(vec![
                    (p.0 - hw, p.1 - hw),
                    (p.0 + hw, p.1 - hw),
                    (p.0 + hw, p.1 + hw),
                    (p.0 - hw, p.1 + hw),
                ]),
                _ => {}
            }
            continue;
        }
        let n = pts.len();
        let seg_count = if line.closed { n } else { n - 1 };
        for i in 0..seg_count {
            let a = pts[i];
            let b = pts[(i + 1) % n];
            out.push(segment_quad(a, b, hw));
        }
        // 接合
        let join_range: Vec<usize> = if line.closed {
            (0..n).collect()
        } else {
            (1..n - 1).collect()
        };
        for &i in &join_range {
            let prev = pts[(i + n - 1) % n];
            let cur = pts[i];
            let next = pts[(i + 1) % n];
            join(prev, cur, next, hw, style, round_steps, &mut out);
        }
        // 端点
        if !line.closed {
            cap(pts[1], pts[0], hw, style.cap, round_steps, &mut out);
            cap(pts[n - 2], pts[n - 1], hw, style.cap, round_steps, &mut out);
        }
    }
    // 装置空間へ。向きを反時計回り（装置空間で符号付き面積が正）に揃える
    out.into_iter()
        .map(|poly| {
            let mut p: Vec<(f64, f64)> = poly
                .into_iter()
                .map(|(x, y)| to_device.apply(x, y))
                .collect();
            if signed_area(&p) < 0.0 {
                p.reverse();
            }
            p
        })
        .filter(|p| p.len() >= 3)
        .collect()
}

fn dedup(points: &[(f64, f64)]) -> Vec<(f64, f64)> {
    let mut v: Vec<(f64, f64)> = Vec::with_capacity(points.len());
    for &p in points {
        if let Some(&last) = v.last() {
            if (p.0 - last.0).abs() < 1e-12 && (p.1 - last.1).abs() < 1e-12 {
                continue;
            }
        }
        v.push(p);
    }
    // 閉じた線で最後が最初と一致するなら落とす
    if v.len() > 1 {
        let (f, l) = (v[0], *v.last().unwrap());
        if (f.0 - l.0).abs() < 1e-12 && (f.1 - l.1).abs() < 1e-12 {
            v.pop();
        }
    }
    v
}

fn signed_area(p: &[(f64, f64)]) -> f64 {
    let n = p.len();
    let mut a = 0.0;
    for i in 0..n {
        let (x0, y0) = p[i];
        let (x1, y1) = p[(i + 1) % n];
        a += x0 * y1 - x1 * y0;
    }
    a * 0.5
}

fn segment_quad(a: (f64, f64), b: (f64, f64), hw: f64) -> Vec<(f64, f64)> {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len = dx.hypot(dy);
    let (nx, ny) = (-dy / len * hw, dx / len * hw);
    vec![
        (a.0 + nx, a.1 + ny),
        (b.0 + nx, b.1 + ny),
        (b.0 - nx, b.1 - ny),
        (a.0 - nx, a.1 - ny),
    ]
}

fn circle(c: (f64, f64), r: f64, steps: usize) -> Vec<(f64, f64)> {
    let steps = steps.max(8);
    (0..steps)
        .map(|i| {
            let t = i as f64 / steps as f64 * std::f64::consts::TAU;
            (c.0 + r * t.cos(), c.1 + r * t.sin())
        })
        .collect()
}

fn join(
    prev: (f64, f64),
    cur: (f64, f64),
    next: (f64, f64),
    hw: f64,
    style: &StrokeStyle,
    steps: usize,
    out: &mut Vec<Vec<(f64, f64)>>,
) {
    let (d0x, d0y) = norm(cur.0 - prev.0, cur.1 - prev.1);
    let (d1x, d1y) = norm(next.0 - cur.0, next.1 - cur.1);
    let cross = d0x * d1y - d0y * d1x;
    let dot = d0x * d1x + d0y * d1y;
    if cross.abs() < 1e-12 && dot > 0.0 {
        return; // 直進
    }
    match style.join {
        LineJoin::Round => out.push(circle(cur, hw, steps)),
        LineJoin::Bevel | LineJoin::Miter => {
            // 外側の法線（曲がる向きの反対側）
            let sign = if cross > 0.0 { -1.0 } else { 1.0 };
            let n0 = (-d0y * hw * sign, d0x * hw * sign);
            let n1 = (-d1y * hw * sign, d1x * hw * sign);
            let p0 = (cur.0 + n0.0, cur.1 + n0.1);
            let p1 = (cur.0 + n1.0, cur.1 + n1.1);
            let mut poly = vec![cur, p0];
            if style.join == LineJoin::Miter {
                // 尖り長の比 = 1 / sin(θ/2)、θ は線の間の角
                let theta = (-dot).clamp(-1.0, 1.0).acos();
                let ratio = 1.0 / (theta * 0.5).sin().max(1e-12);
                if ratio <= style.miter_limit {
                    // 尖りの先端: 二つの法線の和の方向へ hw × ratio
                    let (mx, my) = norm(n0.0 + n1.0, n0.1 + n1.1);
                    poly.push((cur.0 + mx * hw * ratio, cur.1 + my * hw * ratio));
                }
            }
            poly.push(p1);
            out.push(poly);
        }
    }
}

fn cap(
    from: (f64, f64),
    end: (f64, f64),
    hw: f64,
    cap: LineCap,
    steps: usize,
    out: &mut Vec<Vec<(f64, f64)>>,
) {
    let (dx, dy) = norm(end.0 - from.0, end.1 - from.1);
    match cap {
        LineCap::Butt => {}
        LineCap::Round => out.push(circle(end, hw, steps)),
        LineCap::Square => {
            let (nx, ny) = (-dy * hw, dx * hw);
            let (ex, ey) = (dx * hw, dy * hw);
            out.push(vec![
                (end.0 + nx, end.1 + ny),
                (end.0 + nx + ex, end.1 + ny + ey),
                (end.0 - nx + ex, end.1 - ny + ey),
                (end.0 - nx, end.1 - ny),
            ]);
        }
    }
}

fn norm(x: f64, y: f64) -> (f64, f64) {
    let l = x.hypot(y);
    if l < 1e-300 {
        (0.0, 0.0)
    } else {
        (x / l, y / l)
    }
}

/// 破線を適用する。閉じた線は開いた線として扱う（PDF §8.4.3.6）
fn apply_dash(lines: &[Polyline], dash: &[f64], phase: f64) -> Vec<Polyline> {
    let total: f64 = dash.iter().sum();
    if total <= 0.0 {
        return lines.to_vec();
    }
    let mut out = Vec::new();
    for line in lines {
        let mut pts = line.points.clone();
        if line.closed && pts.len() > 1 {
            pts.push(pts[0]);
        }
        // 位相から開始状態を求める
        let mut idx = 0;
        let mut remain = dash[0];
        let mut on = true;
        let mut ph = phase.rem_euclid(total * if dash.len() % 2 == 1 { 2.0 } else { 1.0 });
        while ph > 0.0 {
            if ph >= remain {
                ph -= remain;
                idx = (idx + 1) % dash.len();
                remain = dash[idx];
                on = !on;
            } else {
                remain -= ph;
                ph = 0.0;
            }
        }
        let mut cur: Vec<(f64, f64)> = if on { vec![pts[0]] } else { Vec::new() };
        for w in pts.windows(2) {
            let (a, b) = (w[0], w[1]);
            let seg_len = (b.0 - a.0).hypot(b.1 - a.1);
            let mut pos = 0.0;
            while seg_len - pos > remain {
                pos += remain;
                let t = pos / seg_len;
                let p = (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
                if on {
                    cur.push(p);
                    out.push(Polyline {
                        points: std::mem::take(&mut cur),
                        closed: false,
                    });
                } else {
                    cur = vec![p];
                }
                on = !on;
                idx = (idx + 1) % dash.len();
                remain = dash[idx];
                if remain == 0.0 {
                    // 長さ 0 の区間はそのまま切り替える
                    if on {
                        cur.push(p);
                    }
                }
            }
            remain -= seg_len - pos;
            if on {
                cur.push(b);
            }
        }
        if on && cur.len() >= 2 {
            out.push(Polyline {
                points: cur,
                closed: false,
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dash_splits_a_line_into_on_segments() {
        let line = Polyline {
            points: vec![(0.0, 0.0), (10.0, 0.0)],
            closed: false,
        };
        let pieces = apply_dash(std::slice::from_ref(&line), &[3.0, 1.0], 0.0);
        let lens: Vec<f64> = pieces
            .iter()
            .map(|p| p.points.last().unwrap().0 - p.points[0].0)
            .collect();
        assert_eq!(lens, vec![3.0, 3.0, 2.0]);
        let shifted = apply_dash(std::slice::from_ref(&line), &[3.0, 1.0], 2.0);
        let lens: Vec<f64> = shifted
            .iter()
            .map(|p| p.points.last().unwrap().0 - p.points[0].0)
            .collect();
        assert_eq!(lens, vec![1.0, 3.0, 3.0]);
    }
}

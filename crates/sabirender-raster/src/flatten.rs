//! 経路の折れ線化。三次ベジエは平坦さの判定で再帰的に二分する。

use sabirender_display::{Path, Segment};

/// 経路を多角形の列に折れ線化する。開いた部分経路も閉じる（塗り用）
pub fn flatten(path: &Path, tolerance: f64) -> Vec<Vec<(f64, f64)>> {
    let mut polys = Vec::new();
    for line in flatten_open(path, tolerance) {
        if line.points.len() >= 2 {
            polys.push(line.points);
        }
    }
    polys
}

/// 折れ線化した部分経路（閉じたかどうかを保持。線の描画用）
#[derive(Debug, Clone, PartialEq)]
pub struct Polyline {
    pub points: Vec<(f64, f64)>,
    pub closed: bool,
}

pub fn flatten_open(path: &Path, tolerance: f64) -> Vec<Polyline> {
    let mut out: Vec<Polyline> = Vec::new();
    let mut cur: Option<Polyline> = None;
    let mut last = (0.0, 0.0);
    for s in &path.segments {
        match *s {
            Segment::MoveTo(x, y) => {
                if let Some(c) = cur.take() {
                    out.push(c);
                }
                cur = Some(Polyline {
                    points: vec![(x, y)],
                    closed: false,
                });
                last = (x, y);
            }
            Segment::LineTo(x, y) => {
                let c = cur.get_or_insert_with(|| Polyline {
                    points: vec![last],
                    closed: false,
                });
                c.points.push((x, y));
                last = (x, y);
            }
            Segment::CurveTo(x1, y1, x2, y2, x3, y3) => {
                let c = cur.get_or_insert_with(|| Polyline {
                    points: vec![last],
                    closed: false,
                });
                cubic(
                    last,
                    (x1, y1),
                    (x2, y2),
                    (x3, y3),
                    tolerance,
                    &mut c.points,
                    0,
                );
                last = (x3, y3);
            }
            Segment::Close => {
                if let Some(mut c) = cur.take() {
                    c.closed = true;
                    let first = c.points[0];
                    out.push(c);
                    // 閉じた後の描画は始点から始まる
                    last = first;
                }
            }
        }
    }
    if let Some(c) = cur.take() {
        out.push(c);
    }
    out
}

fn cubic(
    p0: (f64, f64),
    p1: (f64, f64),
    p2: (f64, f64),
    p3: (f64, f64),
    tol: f64,
    out: &mut Vec<(f64, f64)>,
    depth: u32,
) {
    // 制御点が弦からどれだけ離れているか（Roger Willcocks の平坦さ判定の簡略形）
    let dx = p3.0 - p0.0;
    let dy = p3.1 - p0.1;
    let d1 = ((p1.0 - p3.0) * dy - (p1.1 - p3.1) * dx).abs();
    let d2 = ((p2.0 - p3.0) * dy - (p2.1 - p3.1) * dx).abs();
    let dd = (d1 + d2) * (d1 + d2);
    let flat = if dx * dx + dy * dy < 1e-18 {
        // 弦が退化: 制御点との距離で判定
        let e1 = (p1.0 - p0.0).hypot(p1.1 - p0.1);
        let e2 = (p2.0 - p0.0).hypot(p2.1 - p0.1);
        e1.max(e2) <= tol
    } else {
        dd <= tol * tol * (dx * dx + dy * dy)
    };
    if flat || depth >= 16 {
        out.push(p3);
        return;
    }
    // de Casteljau で二分
    let m = |a: (f64, f64), b: (f64, f64)| ((a.0 + b.0) * 0.5, (a.1 + b.1) * 0.5);
    let p01 = m(p0, p1);
    let p12 = m(p1, p2);
    let p23 = m(p2, p3);
    let p012 = m(p01, p12);
    let p123 = m(p12, p23);
    let mid = m(p012, p123);
    cubic(p0, p01, p012, mid, tol, out, depth + 1);
    cubic(mid, p123, p23, p3, tol, out, depth + 1);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_quarter_circle_flattens_within_tolerance() {
        // 半径 100 の四分円（k = 0.5523）
        let k = 0.5522847498;
        let path = Path {
            segments: vec![
                Segment::MoveTo(100.0, 0.0),
                Segment::CurveTo(100.0, 100.0 * k, 100.0 * k, 100.0, 0.0, 100.0),
            ],
        };
        let lines = flatten_open(&path, 0.01);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].points.len() > 8);
        for (x, y) in &lines[0].points {
            let r = x.hypot(*y);
            assert!((r - 100.0).abs() < 0.03, "radius {r}");
        }
    }
}

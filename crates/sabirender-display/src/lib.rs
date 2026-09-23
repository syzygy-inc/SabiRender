//! 装置非依存の描画命令列（display list）。
//!
//! SabiRender の入口（SabiDVI、将来の SabiPDF）はページをこの列に変換し、後端（CPU 参照ラスタライザ、SVG）はこの列だけを読む。
//! 図形モデルは PDF のもの（ISO 32000-1 §8）に揃える。3D は扱わない（Sabi 系列は 2D テクスチャの領域に留める）。
//!
//! 座標: すべての項目は「ページ空間」への変換行列 `ctm` を持ち、経路はその手前の利用者空間で表す。
//! 塗りとクリップの経路は評価器が構築時の CTM で変換済みなので `ctm` は恒等になる。
//! 線は利用者空間で太らせてから変換しなければならない（非等方な拡大で線幅が向きにより変わる）ため、
//! 塗り時点の CTM と、その利用者空間に戻した経路を持つ。
//! ページ空間の単位は bp（1/72 in）、y は上向き。後端が装置空間へ写す。

/// 経路の要素。座標は利用者空間
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Segment {
    MoveTo(f64, f64),
    LineTo(f64, f64),
    CurveTo(f64, f64, f64, f64, f64, f64),
    Close,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Path {
    pub segments: Vec<Segment>,
}

impl Path {
    pub fn is_empty(&self) -> bool {
        self.segments.is_empty()
    }

    /// 矩形（`re` 演算子と同じ向き: 反時計回り）
    pub fn rect(x: f64, y: f64, w: f64, h: f64) -> Path {
        Path {
            segments: vec![
                Segment::MoveTo(x, y),
                Segment::LineTo(x + w, y),
                Segment::LineTo(x + w, y + h),
                Segment::LineTo(x, y + h),
                Segment::Close,
            ],
        }
    }

    /// アフィン変換を適用した経路
    pub fn transform(&self, m: &Matrix) -> Path {
        Path {
            segments: self
                .segments
                .iter()
                .map(|s| match *s {
                    Segment::MoveTo(x, y) => {
                        let (x, y) = m.apply(x, y);
                        Segment::MoveTo(x, y)
                    }
                    Segment::LineTo(x, y) => {
                        let (x, y) = m.apply(x, y);
                        Segment::LineTo(x, y)
                    }
                    Segment::CurveTo(x1, y1, x2, y2, x3, y3) => {
                        let (a, b) = m.apply(x1, y1);
                        let (c, d) = m.apply(x2, y2);
                        let (e, f) = m.apply(x3, y3);
                        Segment::CurveTo(a, b, c, d, e, f)
                    }
                    Segment::Close => Segment::Close,
                })
                .collect(),
        }
    }
}

/// アフィン変換 [a b c d e f]。PDF と同じく x' = a x + c y + e, y' = b x + d y + f
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Matrix {
    pub a: f64,
    pub b: f64,
    pub c: f64,
    pub d: f64,
    pub e: f64,
    pub f: f64,
}

impl Matrix {
    pub const IDENTITY: Matrix = Matrix {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 1.0,
        e: 0.0,
        f: 0.0,
    };

    pub fn new(a: f64, b: f64, c: f64, d: f64, e: f64, f: f64) -> Matrix {
        Matrix { a, b, c, d, e, f }
    }

    pub fn translate(tx: f64, ty: f64) -> Matrix {
        Matrix::new(1.0, 0.0, 0.0, 1.0, tx, ty)
    }

    pub fn scale(sx: f64, sy: f64) -> Matrix {
        Matrix::new(sx, 0.0, 0.0, sy, 0.0, 0.0)
    }

    pub fn apply(&self, x: f64, y: f64) -> (f64, f64) {
        (
            self.a * x + self.c * y + self.e,
            self.b * x + self.d * y + self.f,
        )
    }

    /// ベクトル（平行移動を含めない）
    pub fn apply_vector(&self, x: f64, y: f64) -> (f64, f64) {
        (self.a * x + self.c * y, self.b * x + self.d * y)
    }

    /// `self` を先に、`other` を後に適用する合成。PDF の `cm` は `new = cm × ctm` なので `cm.then(&ctm)`
    pub fn then(&self, other: &Matrix) -> Matrix {
        Matrix {
            a: self.a * other.a + self.b * other.c,
            b: self.a * other.b + self.b * other.d,
            c: self.c * other.a + self.d * other.c,
            d: self.c * other.b + self.d * other.d,
            e: self.e * other.a + self.f * other.c + other.e,
            f: self.e * other.b + self.f * other.d + other.f,
        }
    }

    pub fn determinant(&self) -> f64 {
        self.a * self.d - self.b * self.c
    }

    pub fn invert(&self) -> Option<Matrix> {
        let det = self.determinant();
        if det.abs() < 1e-300 {
            return None;
        }
        let (a, b, c, d) = (self.d / det, -self.b / det, -self.c / det, self.a / det);
        Some(Matrix {
            a,
            b,
            c,
            d,
            e: -(self.e * a + self.f * c),
            f: -(self.e * b + self.f * d),
        })
    }

    /// 単位円の像の面積の平方根。線幅など長さの「代表的な」拡大率
    pub fn mean_scale(&self) -> f64 {
        self.determinant().abs().sqrt()
    }
}

/// 色。PDF の装置色空間をそのまま保持し、後端で変換する
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Color {
    Gray(f64),
    Rgb(f64, f64, f64),
    Cmyk(f64, f64, f64, f64),
}

impl Color {
    pub const BLACK: Color = Color::Gray(0.0);

    /// sRGB 相当の 0..1 三成分（CMYK は PDF §10.3.5 の単純な変換）
    pub fn to_rgb(&self) -> (f64, f64, f64) {
        match *self {
            Color::Gray(g) => (g, g, g),
            Color::Rgb(r, g, b) => (r, g, b),
            Color::Cmyk(c, m, y, k) => (
                (1.0 - c.min(1.0)) * (1.0 - k.min(1.0)),
                (1.0 - m.min(1.0)) * (1.0 - k.min(1.0)),
                (1.0 - y.min(1.0)) * (1.0 - k.min(1.0)),
            ),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FillRule {
    NonZero,
    EvenOdd,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineCap {
    Butt,
    Round,
    Square,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineJoin {
    Miter,
    Round,
    Bevel,
}

/// 線の様式（利用者空間の単位）
#[derive(Debug, Clone, PartialEq)]
pub struct StrokeStyle {
    pub width: f64,
    pub cap: LineCap,
    pub join: LineJoin,
    pub miter_limit: f64,
    /// 破線の配列と位相。空なら実線
    pub dash: Vec<f64>,
    pub dash_phase: f64,
}

impl Default for StrokeStyle {
    fn default() -> Self {
        StrokeStyle {
            width: 1.0,
            cap: LineCap::Butt,
            join: LineJoin::Miter,
            miter_limit: 10.0,
            dash: Vec::new(),
            dash_phase: 0.0,
        }
    }
}

/// 字形の参照。輪郭の解決は入口側（SabiFace）に任せ、後端には輪郭を渡す
#[derive(Debug, Clone, PartialEq)]
pub struct GlyphRun {
    pub glyphs: Vec<PlacedGlyph>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlacedGlyph {
    /// 輪郭（字形単位）
    pub outline: Path,
    /// 字形単位から項目の利用者空間への変換（FontMatrix、傾斜や横拡大、大きさ、位置をすべて含む）。
    /// ページ空間へは `transform.then(&ctm)`
    pub transform: Matrix,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    /// 塗り。経路は構築時の CTM で変換済みなので、通常 `ctm` は恒等
    Fill {
        path: Path,
        ctm: Matrix,
        rule: FillRule,
        color: Color,
        alpha: f64,
    },
    /// 線。経路は `ctm` の利用者空間で持ち、太らせてから `ctm` で変換する
    Stroke {
        path: Path,
        ctm: Matrix,
        style: StrokeStyle,
        color: Color,
        alpha: f64,
    },
    /// 以降の項目を経路の内側に制限する。`ClipPop` まで有効。入れ子は交叉
    ClipPush {
        path: Path,
        ctm: Matrix,
        rule: FillRule,
    },
    ClipPop,
    Glyphs {
        run: GlyphRun,
        ctm: Matrix,
        color: Color,
        alpha: f64,
    },
    /// 画像。画素は行優先の RGBA8、`ctm` は単位正方形 (0,0)-(1,1) をページ空間へ写す（PDF の画像空間）
    Image {
        width: u32,
        height: u32,
        rgba: std::sync::Arc<Vec<u8>>,
        ctm: Matrix,
        alpha: f64,
    },
    /// 解釈できなかった命令。位置とキーワードを保持し、検証時に数える
    Unsupported {
        what: String,
    },
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct DisplayList {
    pub items: Vec<Item>,
}

impl DisplayList {
    pub fn push(&mut self, item: Item) {
        self.items.push(item);
    }

    pub fn unsupported(&self) -> impl Iterator<Item = &str> {
        self.items.iter().filter_map(|i| match i {
            Item::Unsupported { what } => Some(what.as_str()),
            _ => None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matrix_composition_matches_pdf_cm_semantics() {
        // 先に平行移動、後に拡大: (1,1) → (2,3) → (4,6)
        let m = Matrix::translate(1.0, 2.0).then(&Matrix::scale(2.0, 2.0));
        assert_eq!(m.apply(1.0, 1.0), (4.0, 6.0));
        let inv = m.invert().unwrap();
        let (x, y) = inv.apply(4.0, 6.0);
        assert!((x - 1.0).abs() < 1e-12 && (y - 1.0).abs() < 1e-12);
    }

    #[test]
    fn cmyk_to_rgb_is_the_naive_pdf_formula() {
        assert_eq!(Color::Cmyk(0.0, 0.0, 0.0, 1.0).to_rgb(), (0.0, 0.0, 0.0));
        assert_eq!(Color::Cmyk(1.0, 0.0, 0.0, 0.0).to_rgb(), (0.0, 1.0, 1.0));
    }
}

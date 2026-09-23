//! PDF の内容ストリーム（図形演算子）の評価器。演算子列を読み、図形状態を追跡して、装置非依存の描画命令列
//! （`sabirender-display`）を出力する。
//!
//! 対象は ISO 32000-1 §8（図形）の演算子のうち、経路・塗り・線・クリップ・図形状態・装置色空間、
//! フォーム XObject（資源の解決は呼び出し側）まで。テキスト演算子（§9）とシェーディング、パターンは
//! `Unsupported` として記録する。TeX の文字は DVI 側で置かれるので、入口が SabiDVI である限りテキスト演算子は
//! PGF の出力に現れない。
//!
//! 経路の各点は、その点を加えた時点の CTM で変換してページ空間に保持する（§8.5.2「経路構築中に `cm` が
//! 来れば、それ以降の点だけが新しい行列で変換される」）。線は塗り時点の CTM の利用者空間に戻して太らせる。
//!
//! 座標は「基準行列」（構築時に与える。DVI の現在位置をページ空間に写すもの）から始まる。

pub mod lexer;

use lexer::{Lexer, Token};
use sabirender_display::{
    Color, DisplayList, FillRule, Item, LineCap, LineJoin, Matrix, Path, Segment, StrokeStyle,
};

/// 図形状態（§8.4）
#[derive(Debug, Clone)]
pub struct GraphicsState {
    pub ctm: Matrix,
    pub stroke_color: Color,
    pub fill_color: Color,
    pub stroke_style: StrokeStyle,
    pub stroke_alpha: f64,
    pub fill_alpha: f64,
    /// この状態で積んだクリップの数（`Q` で同数の `ClipPop` を出す）
    clip_depth: usize,
}

impl GraphicsState {
    pub fn new(ctm: Matrix) -> Self {
        GraphicsState {
            ctm,
            stroke_color: Color::BLACK,
            fill_color: Color::BLACK,
            stroke_style: StrokeStyle::default(),
            stroke_alpha: 1.0,
            fill_alpha: 1.0,
            clip_depth: 0,
        }
    }
}

/// `gs` 演算子で参照される拡張図形状態のうち、扱う項目
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ExtGState {
    pub line_width: Option<f64>,
    pub stroke_alpha: Option<f64>,
    pub fill_alpha: Option<f64>,
}

/// 資源の解決。フォーム XObject と ExtGState を名前で返す
pub trait Resources {
    /// フォーム XObject: (内容ストリーム, /Matrix, /BBox)
    fn form_xobject(&self, name: &str) -> Option<FormXObject>;
    fn ext_gstate(&self, name: &str) -> Option<ExtGState>;
}

#[derive(Debug, Clone, PartialEq)]
pub struct FormXObject {
    pub content: Vec<u8>,
    pub matrix: Matrix,
    /// [llx lly urx ury]
    pub bbox: [f64; 4],
}

pub struct NoResources;

impl Resources for NoResources {
    fn form_xobject(&self, _: &str) -> Option<FormXObject> {
        None
    }
    fn ext_gstate(&self, _: &str) -> Option<ExtGState> {
        None
    }
}

/// 評価器。`q`/`Q` の入れ子は special の境界（`pdf:bcontent` / `pdf:econtent`）をまたいで持続し得るので、
/// 一つの評価器をページの間ずっと使い回す
pub struct Evaluator<'r> {
    pub state: GraphicsState,
    stack: Vec<GraphicsState>,
    /// 構築中の経路（ページ空間）
    path: Path,
    /// 直前の `W` / `W*`（塗り演算子の後で適用する）
    pending_clip: Option<FillRule>,
    /// 現在の点と部分経路の始点（ページ空間）
    current: (f64, f64),
    start: (f64, f64),
    resources: &'r dyn Resources,
    /// フォーム XObject の入れ子の深さ（無限再帰の防止）
    depth: usize,
}

impl<'r> Evaluator<'r> {
    pub fn new(base: Matrix, resources: &'r dyn Resources) -> Self {
        Evaluator {
            state: GraphicsState::new(base),
            stack: Vec::new(),
            path: Path::default(),
            pending_clip: None,
            current: (0.0, 0.0),
            start: (0.0, 0.0),
            resources,
            depth: 0,
        }
    }

    /// 内容ストリームを評価して `out` に追記する
    pub fn run(&mut self, content: &[u8], out: &mut DisplayList) {
        let mut lx = Lexer::new(content);
        let mut operands: Vec<Token> = Vec::new();
        while let Some(t) = lx.next_token() {
            match t {
                Token::Operator(op) => {
                    self.execute(&op, &operands, out);
                    operands.clear();
                }
                Token::InlineImage(_) => {
                    out.push(Item::Unsupported {
                        what: "inline image (BI/ID/EI)".into(),
                    });
                    operands.clear();
                }
                other => {
                    operands.push(other);
                    if operands.len() > 64 {
                        operands.drain(..32);
                    }
                }
            }
        }
    }

    /// `q` に対応しない `Q` を補い、開いたクリップを閉じる（ページの終わりに呼ぶ）
    pub fn finish(&mut self, out: &mut DisplayList) {
        while !self.stack.is_empty() {
            self.execute("Q", &[], out);
        }
        for _ in 0..self.state.clip_depth {
            out.push(Item::ClipPop);
        }
        self.state.clip_depth = 0;
    }

    fn execute(&mut self, op: &str, args: &[Token], out: &mut DisplayList) {
        let nums: Vec<f64> = args
            .iter()
            .filter_map(|t| {
                if let Token::Number(n) = t {
                    Some(*n)
                } else {
                    None
                }
            })
            .collect();
        let n = |i: usize| nums.get(i).copied().unwrap_or(0.0);
        // 演算子は末尾の数を取る（余分な先頭の被演算子は無視する）
        let tail = |k: usize| -> Vec<f64> {
            let s = nums.len().saturating_sub(k);
            let mut v = nums[s..].to_vec();
            while v.len() < k {
                v.insert(0, 0.0);
            }
            v
        };
        let ctm = self.state.ctm;
        match op {
            // --- 図形状態 ---
            "q" => {
                self.stack.push(self.state.clone());
                self.state.clip_depth = 0;
            }
            "Q" => {
                if let Some(s) = self.stack.pop() {
                    for _ in 0..self.state.clip_depth {
                        out.push(Item::ClipPop);
                    }
                    self.state = s;
                }
            }
            "cm" => {
                let v = tail(6);
                let m = Matrix::new(v[0], v[1], v[2], v[3], v[4], v[5]);
                self.state.ctm = m.then(&self.state.ctm);
            }
            "w" => self.state.stroke_style.width = n(nums.len().saturating_sub(1)),
            "J" => {
                self.state.stroke_style.cap = match n(0) as i32 {
                    1 => LineCap::Round,
                    2 => LineCap::Square,
                    _ => LineCap::Butt,
                }
            }
            "j" => {
                self.state.stroke_style.join = match n(0) as i32 {
                    1 => LineJoin::Round,
                    2 => LineJoin::Bevel,
                    _ => LineJoin::Miter,
                }
            }
            "M" => self.state.stroke_style.miter_limit = n(0),
            "d" => {
                // [array] phase
                let mut arr = Vec::new();
                let mut in_arr = false;
                let mut phase = 0.0;
                for t in args {
                    match t {
                        Token::ArrayOpen => in_arr = true,
                        Token::ArrayClose => in_arr = false,
                        Token::Number(v) => {
                            if in_arr {
                                arr.push(*v);
                            } else {
                                phase = *v;
                            }
                        }
                        _ => {}
                    }
                }
                if arr.iter().all(|v| *v <= 0.0) {
                    arr.clear();
                }
                self.state.stroke_style.dash = arr;
                self.state.stroke_style.dash_phase = phase;
            }
            "ri" | "i" => {}
            "gs" => {
                if let Some(Token::Name(name)) = args.last() {
                    match self.resources.ext_gstate(name) {
                        Some(g) => {
                            if let Some(w) = g.line_width {
                                self.state.stroke_style.width = w;
                            }
                            if let Some(a) = g.stroke_alpha {
                                self.state.stroke_alpha = a;
                            }
                            if let Some(a) = g.fill_alpha {
                                self.state.fill_alpha = a;
                            }
                        }
                        None => out.push(Item::Unsupported {
                            what: format!("gs /{name}"),
                        }),
                    }
                }
            }
            // --- 経路構築（点はその時点の CTM でページ空間へ） ---
            "m" => {
                let v = tail(2);
                let p = ctm.apply(v[0], v[1]);
                self.current = p;
                self.start = p;
                self.path.segments.push(Segment::MoveTo(p.0, p.1));
            }
            "l" => {
                let v = tail(2);
                self.ensure_open();
                let p = ctm.apply(v[0], v[1]);
                self.current = p;
                self.path.segments.push(Segment::LineTo(p.0, p.1));
            }
            "c" => {
                let v = tail(6);
                self.ensure_open();
                let (a, b) = ctm.apply(v[0], v[1]);
                let (c, d) = ctm.apply(v[2], v[3]);
                let p = ctm.apply(v[4], v[5]);
                self.current = p;
                self.path
                    .segments
                    .push(Segment::CurveTo(a, b, c, d, p.0, p.1));
            }
            "v" => {
                let v = tail(4);
                self.ensure_open();
                let (x0, y0) = self.current;
                let (c, d) = ctm.apply(v[0], v[1]);
                let p = ctm.apply(v[2], v[3]);
                self.current = p;
                self.path
                    .segments
                    .push(Segment::CurveTo(x0, y0, c, d, p.0, p.1));
            }
            "y" => {
                let v = tail(4);
                self.ensure_open();
                let (a, b) = ctm.apply(v[0], v[1]);
                let p = ctm.apply(v[2], v[3]);
                self.current = p;
                self.path
                    .segments
                    .push(Segment::CurveTo(a, b, p.0, p.1, p.0, p.1));
            }
            "h" => {
                if !self.path.segments.is_empty()
                    && !matches!(self.path.segments.last(), Some(Segment::Close))
                {
                    self.path.segments.push(Segment::Close);
                    self.current = self.start;
                }
            }
            "re" => {
                let v = tail(4);
                self.path
                    .segments
                    .extend(Path::rect(v[0], v[1], v[2], v[3]).transform(&ctm).segments);
                let p = ctm.apply(v[0], v[1]);
                self.current = p;
                self.start = p;
            }
            // --- 塗り ---
            "S" => self.paint(false, false, true, FillRule::NonZero, out),
            "s" => self.paint(true, false, true, FillRule::NonZero, out),
            "f" | "F" => self.paint(false, true, false, FillRule::NonZero, out),
            "f*" => self.paint(false, true, false, FillRule::EvenOdd, out),
            "B" => self.paint(false, true, true, FillRule::NonZero, out),
            "B*" => self.paint(false, true, true, FillRule::EvenOdd, out),
            "b" => self.paint(true, true, true, FillRule::NonZero, out),
            "b*" => self.paint(true, true, true, FillRule::EvenOdd, out),
            "n" => self.paint(false, false, false, FillRule::NonZero, out),
            "W" => self.pending_clip = Some(FillRule::NonZero),
            "W*" => self.pending_clip = Some(FillRule::EvenOdd),
            // --- 色 ---
            "g" => self.state.fill_color = Color::Gray(n(0)),
            "G" => self.state.stroke_color = Color::Gray(n(0)),
            "rg" => self.state.fill_color = Color::Rgb(n(0), n(1), n(2)),
            "RG" => self.state.stroke_color = Color::Rgb(n(0), n(1), n(2)),
            "k" => self.state.fill_color = Color::Cmyk(n(0), n(1), n(2), n(3)),
            "K" => self.state.stroke_color = Color::Cmyk(n(0), n(1), n(2), n(3)),
            "cs" | "CS" => {
                // 装置色空間だけ。sc/SC の成分数で判断するので名前は覚えなくてよい
                if let Some(Token::Name(name)) = args.last() {
                    if !matches!(
                        name.as_str(),
                        "DeviceGray" | "DeviceRGB" | "DeviceCMYK" | "G" | "RGB" | "CMYK"
                    ) {
                        out.push(Item::Unsupported {
                            what: format!("{op} /{name}"),
                        });
                    }
                }
            }
            "sc" | "scn" | "SC" | "SCN" => {
                if args.iter().any(|t| matches!(t, Token::Name(_))) {
                    out.push(Item::Unsupported {
                        what: format!("{op} with pattern"),
                    });
                } else {
                    let c = match nums.len() {
                        1 => Some(Color::Gray(n(0))),
                        3 => Some(Color::Rgb(n(0), n(1), n(2))),
                        4 => Some(Color::Cmyk(n(0), n(1), n(2), n(3))),
                        _ => None,
                    };
                    if let Some(c) = c {
                        if op.starts_with('s') {
                            self.state.fill_color = c;
                        } else {
                            self.state.stroke_color = c;
                        }
                    }
                }
            }
            // --- XObject ---
            "Do" => {
                if let Some(Token::Name(name)) = args.last() {
                    match self.resources.form_xobject(name) {
                        Some(form) => self.run_form(&form, out),
                        None => out.push(Item::Unsupported {
                            what: format!("Do /{name}"),
                        }),
                    }
                }
            }
            // --- 互換性 ---
            "BX" | "EX" | "MP" | "DP" | "BMC" | "BDC" | "EMC" => {}
            // --- テキスト、シェーディング、その他 ---
            "BT" | "ET" | "Tf" | "Td" | "TD" | "Tm" | "T*" | "TL" | "Tc" | "Tw" | "Tz" | "Ts"
            | "Tr" | "Tj" | "TJ" | "'" | "\"" | "d0" | "d1" => out.push(Item::Unsupported {
                what: format!("text operator {op}"),
            }),
            "sh" => out.push(Item::Unsupported { what: "sh".into() }),
            other => out.push(Item::Unsupported {
                what: format!("operator {other}"),
            }),
        }
    }

    fn ensure_open(&mut self) {
        if self.path.segments.is_empty()
            || matches!(self.path.segments.last(), Some(Segment::Close))
        {
            let (x, y) = self.current;
            self.path.segments.push(Segment::MoveTo(x, y));
            self.start = self.current;
        }
    }

    fn paint(
        &mut self,
        close: bool,
        fill: bool,
        stroke: bool,
        rule: FillRule,
        out: &mut DisplayList,
    ) {
        if close {
            self.execute("h", &[], out);
        }
        let path = std::mem::take(&mut self.path);
        if fill && !path.is_empty() {
            out.push(Item::Fill {
                path: path.clone(),
                ctm: Matrix::IDENTITY,
                rule,
                color: self.state.fill_color,
                alpha: self.state.fill_alpha,
            });
        }
        if stroke && !path.is_empty() {
            // 線は塗り時点の CTM の利用者空間で太らせる。CTM が退化していれば太らせようがないのでページ空間のまま
            let (path_user, ctm) = match self.state.ctm.invert() {
                Some(inv) => (path.transform(&inv), self.state.ctm),
                None => (path.clone(), Matrix::IDENTITY),
            };
            out.push(Item::Stroke {
                path: path_user,
                ctm,
                style: self.state.stroke_style.clone(),
                color: self.state.stroke_color,
                alpha: self.state.stroke_alpha,
            });
        }
        if let Some(rule) = self.pending_clip.take() {
            // 空の経路によるクリップは全てを隠す
            out.push(Item::ClipPush {
                path,
                ctm: Matrix::IDENTITY,
                rule,
            });
            self.state.clip_depth += 1;
        }
    }

    fn run_form(&mut self, form: &FormXObject, out: &mut DisplayList) {
        if self.depth > 16 {
            out.push(Item::Unsupported {
                what: "form XObject nested too deeply".into(),
            });
            return;
        }
        self.depth += 1;
        self.execute("q", &[], out);
        self.state.ctm = form.matrix.then(&self.state.ctm);
        let [x0, y0, x1, y1] = form.bbox;
        out.push(Item::ClipPush {
            path: Path::rect(x0.min(x1), y0.min(y1), (x1 - x0).abs(), (y1 - y0).abs())
                .transform(&self.state.ctm),
            ctm: Matrix::IDENTITY,
            rule: FillRule::NonZero,
        });
        self.state.clip_depth += 1;
        let saved_path = std::mem::take(&mut self.path);
        let saved_stack_len = self.stack.len();
        self.run(&form.content, out);
        while self.stack.len() > saved_stack_len {
            self.execute("Q", &[], out);
        }
        self.path = saved_path;
        self.execute("Q", &[], out);
        self.depth -= 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eval(code: &str) -> DisplayList {
        let mut out = DisplayList::default();
        let mut ev = Evaluator::new(Matrix::IDENTITY, &NoResources);
        ev.run(code.as_bytes(), &mut out);
        ev.finish(&mut out);
        out
    }

    /// 項目の経路をページ空間の点列にする
    fn points(item: &Item) -> Vec<(f64, f64)> {
        let (path, ctm) = match item {
            Item::Fill { path, ctm, .. }
            | Item::Stroke { path, ctm, .. }
            | Item::ClipPush { path, ctm, .. } => (path, ctm),
            _ => panic!("no path"),
        };
        path.transform(ctm)
            .segments
            .iter()
            .filter_map(|s| match *s {
                Segment::MoveTo(x, y) | Segment::LineTo(x, y) => Some((x, y)),
                Segment::CurveTo(_, _, _, _, x, y) => Some((x, y)),
                Segment::Close => None,
            })
            .collect()
    }

    #[test]
    fn fill_and_stroke_produce_items_with_state() {
        let dl = eval("q 1 0 0 rg 2 w 0 0 m 10 0 l 10 10 l h B Q");
        assert_eq!(dl.items.len(), 2);
        match &dl.items[0] {
            Item::Fill {
                path, color, rule, ..
            } => {
                assert_eq!(path.segments.len(), 4);
                assert_eq!(*color, Color::Rgb(1.0, 0.0, 0.0));
                assert_eq!(*rule, FillRule::NonZero);
            }
            other => panic!("{other:?}"),
        }
        match &dl.items[1] {
            Item::Stroke { style, color, .. } => {
                assert_eq!(style.width, 2.0);
                assert_eq!(*color, Color::BLACK);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn cm_composes_and_q_restores() {
        let dl = eval("q 2 0 0 2 0 0 cm 1 0 0 1 5 5 cm 0 0 m 1 1 l S Q 0 0 m 1 1 l S");
        // 先に平行移動 (5,5)、後に 2 倍: (0,0) → (10,10)、(1,1) → (12,12)
        assert_eq!(points(&dl.items[0]), vec![(10.0, 10.0), (12.0, 12.0)]);
        match &dl.items[0] {
            Item::Stroke { ctm, path, .. } => {
                assert_eq!(ctm.apply(0.0, 0.0), (10.0, 10.0));
                // 線の経路は利用者空間のまま
                assert_eq!(path.segments[0], Segment::MoveTo(0.0, 0.0));
            }
            _ => unreachable!(),
        }
        match &dl.items[1] {
            Item::Stroke { ctm, .. } => assert_eq!(*ctm, Matrix::IDENTITY),
            _ => unreachable!(),
        }
    }

    #[test]
    fn cm_in_the_middle_of_a_path_moves_only_later_points() {
        // §8.5.2: 経路の点は加えた時点の CTM で変換される
        let dl = eval("10 10 m 1 0 0 1 100 0 cm 20 10 l S");
        assert_eq!(points(&dl.items[0]), vec![(10.0, 10.0), (120.0, 10.0)]);
        let dl = eval("0 0 m 2 0 0 2 0 0 cm 5 5 l 0.5 0 0 0.5 0 0 cm 20 20 l h f");
        assert_eq!(
            points(&dl.items[0]),
            vec![(0.0, 0.0), (10.0, 10.0), (20.0, 20.0)]
        );
    }

    #[test]
    fn stroke_width_uses_the_ctm_at_painting_time() {
        // 経路は拡大前に置き、線幅は拡大後の CTM で決まる
        let dl = eval("2 w 0 0 m 10 0 l 3 0 0 3 0 0 cm S");
        match &dl.items[0] {
            Item::Stroke {
                path, ctm, style, ..
            } => {
                assert_eq!(ctm.mean_scale(), 3.0);
                assert_eq!(style.width, 2.0);
                // 利用者空間の経路: (0,0)-(10,0) を 3 倍の空間へ戻したもの
                let end = match path.segments[1] {
                    Segment::LineTo(x, y) => (x, y),
                    _ => unreachable!(),
                };
                assert!((end.0 - 10.0 / 3.0).abs() < 1e-12 && end.1.abs() < 1e-12);
            }
            _ => unreachable!(),
        }
        assert_eq!(points(&dl.items[0]), vec![(0.0, 0.0), (10.0, 0.0)]);
    }

    #[test]
    fn clip_is_pushed_after_painting_and_popped_by_q() {
        let dl = eval("q 0 0 10 10 re W n 0 0 5 5 re f Q 0 0 1 1 re f");
        let kinds: Vec<&str> = dl
            .items
            .iter()
            .map(|i| match i {
                Item::ClipPush { .. } => "clip",
                Item::ClipPop => "pop",
                Item::Fill { .. } => "fill",
                _ => "?",
            })
            .collect();
        assert_eq!(kinds, ["clip", "fill", "pop", "fill"]);
    }

    #[test]
    fn dash_and_caps_are_recorded() {
        let dl = eval("[3 1] 0.5 d 1 J 1 j 0 0 m 1 0 l S");
        match &dl.items[0] {
            Item::Stroke { style, .. } => {
                assert_eq!(style.dash, vec![3.0, 1.0]);
                assert_eq!(style.dash_phase, 0.5);
                assert_eq!(style.cap, LineCap::Round);
                assert_eq!(style.join, LineJoin::Round);
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn even_odd_and_unsupported_are_distinguished() {
        let dl = eval("0 0 4 4 re 1 1 2 2 re f* /Sh sh BT ET");
        assert!(matches!(
            dl.items[0],
            Item::Fill {
                rule: FillRule::EvenOdd,
                ..
            }
        ));
        let unsupported: Vec<&str> = dl.unsupported().collect();
        assert_eq!(unsupported, ["sh", "text operator BT", "text operator ET"]);
    }

    #[test]
    fn unbalanced_q_is_closed_by_finish() {
        let dl = eval("q q 0 0 1 1 re W n");
        assert!(matches!(dl.items.last(), Some(Item::ClipPop)));
    }

    struct OneForm;
    impl Resources for OneForm {
        fn form_xobject(&self, name: &str) -> Option<FormXObject> {
            (name == "Fm1").then(|| FormXObject {
                content: b"0 0 1 1 re f".to_vec(),
                matrix: Matrix::scale(2.0, 2.0),
                bbox: [0.0, 0.0, 1.0, 1.0],
            })
        }
        fn ext_gstate(&self, name: &str) -> Option<ExtGState> {
            (name == "GS0").then(|| ExtGState {
                fill_alpha: Some(0.5),
                ..Default::default()
            })
        }
    }

    #[test]
    fn form_xobject_is_inlined_with_its_matrix_and_bbox_clip() {
        let mut out = DisplayList::default();
        let mut ev = Evaluator::new(Matrix::translate(10.0, 0.0), &OneForm);
        ev.run(b"/GS0 gs /Fm1 Do", &mut out);
        ev.finish(&mut out);
        let fills: Vec<&Item> = out
            .items
            .iter()
            .filter(|i| matches!(i, Item::Fill { .. }))
            .collect();
        assert_eq!(fills.len(), 1);
        match fills[0] {
            Item::Fill { alpha, .. } => assert_eq!(*alpha, 0.5),
            _ => unreachable!(),
        }
        // (1,1) は /Matrix で 2 倍、基準行列で +10: (12,2)
        assert!(points(fills[0]).contains(&(12.0, 2.0)));
        assert!(matches!(out.items[0], Item::ClipPush { .. }));
        assert!(points(&out.items[0]).contains(&(12.0, 2.0)));
        assert!(out.unsupported().next().is_none());
    }
}

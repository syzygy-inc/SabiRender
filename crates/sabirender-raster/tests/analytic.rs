//! ラスタライザ単体の妥当性: 解析的に面積が分かる図形を描き、被覆率の総和と比較する。
//! ページ空間と装置空間を同一（1 bp = 1 画素、y 上向きのまま）にして検証する。

use sabirender_display::{
    Color, DisplayList, FillRule, Item, LineCap, LineJoin, Matrix, Path, Segment, StrokeStyle,
};
use sabirender_raster::{render, Canvas};

const K: f64 = 0.552_284_749_8; // 円を 4 本の三次ベジエで近似する制御点の係数

fn circle(cx: f64, cy: f64, r: f64) -> Path {
    Path {
        segments: vec![
            Segment::MoveTo(cx + r, cy),
            Segment::CurveTo(cx + r, cy + r * K, cx + r * K, cy + r, cx, cy + r),
            Segment::CurveTo(cx - r * K, cy + r, cx - r, cy + r * K, cx - r, cy),
            Segment::CurveTo(cx - r, cy - r * K, cx - r * K, cy - r, cx, cy - r),
            Segment::CurveTo(cx + r * K, cy - r, cx + r, cy - r * K, cx + r, cy),
            Segment::Close,
        ],
    }
}

fn fill(path: Path, rule: FillRule) -> Item {
    Item::Fill {
        path,
        ctm: Matrix::IDENTITY,
        rule,
        color: Color::BLACK,
        alpha: 1.0,
    }
}

fn stroke(path: Path, style: StrokeStyle) -> Item {
    Item::Stroke {
        path,
        ctm: Matrix::IDENTITY,
        style,
        color: Color::BLACK,
        alpha: 1.0,
    }
}

fn area_of(items: Vec<Item>) -> f64 {
    let mut canvas = Canvas::new(200, 200);
    let list = DisplayList { items };
    let report = render(&list, &mut canvas, &Matrix::IDENTITY);
    assert!(report.skipped.is_empty(), "{:?}", report.skipped);
    canvas.coverage_sum()
}

fn assert_area(got: f64, expected: f64, rel: f64, what: &str) {
    let err = (got - expected).abs() / expected.abs().max(1.0);
    assert!(
        err <= rel,
        "{what}: area {got} vs {expected} (rel err {err:.5})"
    );
}

#[test]
fn fractional_rectangle_has_exact_area() {
    // 端が画素の途中にある矩形は被覆率で厳密に表せる
    let a = area_of(vec![fill(
        Path::rect(10.25, 20.5, 30.5, 15.25),
        FillRule::NonZero,
    )]);
    assert_area(a, 30.5 * 15.25, 1e-9, "rect");
}

#[test]
fn circle_area_matches_pi_r_squared() {
    let r = 40.0;
    let a = area_of(vec![fill(circle(100.0, 100.0, r), FillRule::NonZero)]);
    // ベジエ近似自体の誤差は 0.03% 程度。折れ線化と副走査線の誤差を含めて 0.1%
    assert_area(a, std::f64::consts::PI * r * r, 1e-3, "circle");
}

#[test]
fn nonzero_and_evenodd_differ_on_nested_same_direction_squares() {
    let mut p = Path::rect(20.0, 20.0, 100.0, 100.0);
    p.segments
        .extend(Path::rect(50.0, 50.0, 40.0, 40.0).segments);
    let nz = area_of(vec![fill(p.clone(), FillRule::NonZero)]);
    let eo = area_of(vec![fill(p, FillRule::EvenOdd)]);
    assert_area(nz, 100.0 * 100.0, 1e-9, "nonzero");
    assert_area(eo, 100.0 * 100.0 - 40.0 * 40.0, 1e-9, "even-odd");
}

#[test]
fn nonzero_with_opposite_direction_inner_square_is_a_hole() {
    let mut p = Path::rect(20.0, 20.0, 100.0, 100.0);
    // 時計回りの内側
    p.segments.extend(vec![
        Segment::MoveTo(50.0, 50.0),
        Segment::LineTo(50.0, 90.0),
        Segment::LineTo(90.0, 90.0),
        Segment::LineTo(90.0, 50.0),
        Segment::Close,
    ]);
    let nz = area_of(vec![fill(p, FillRule::NonZero)]);
    assert_area(nz, 100.0 * 100.0 - 40.0 * 40.0, 1e-9, "nonzero hole");
}

#[test]
fn butt_stroke_of_a_line_is_length_times_width() {
    let p = Path {
        segments: vec![Segment::MoveTo(20.0, 50.5), Segment::LineTo(120.0, 50.5)],
    };
    let a = area_of(vec![stroke(
        p,
        StrokeStyle {
            width: 3.0,
            ..Default::default()
        },
    )]);
    assert_area(a, 100.0 * 3.0, 1e-9, "butt line");
}

#[test]
fn square_and_round_caps_extend_the_line() {
    let p = Path {
        segments: vec![Segment::MoveTo(20.0, 50.0), Segment::LineTo(120.0, 50.0)],
    };
    let sq = area_of(vec![stroke(
        p.clone(),
        StrokeStyle {
            width: 4.0,
            cap: LineCap::Square,
            ..Default::default()
        },
    )]);
    assert_area(sq, 104.0 * 4.0, 1e-9, "square cap");
    let rd = area_of(vec![stroke(
        p,
        StrokeStyle {
            width: 4.0,
            cap: LineCap::Round,
            ..Default::default()
        },
    )]);
    assert_area(
        rd,
        100.0 * 4.0 + std::f64::consts::PI * 4.0,
        5e-3,
        "round cap",
    );
}

#[test]
fn miter_join_on_a_closed_square_covers_the_outer_minus_inner_square() {
    let p = Path::rect(40.0, 40.0, 60.0, 60.0);
    let w = 6.0;
    let a = area_of(vec![stroke(
        p.clone(),
        StrokeStyle {
            width: w,
            join: LineJoin::Miter,
            ..Default::default()
        },
    )]);
    assert_area(a, 66.0 * 66.0 - 54.0 * 54.0, 1e-9, "miter square");
    let b = area_of(vec![stroke(
        p.clone(),
        StrokeStyle {
            width: w,
            join: LineJoin::Bevel,
            ..Default::default()
        },
    )]);
    // 角の 3×3 の三角形が 4 つ欠ける
    assert_area(
        b,
        66.0 * 66.0 - 54.0 * 54.0 - 4.0 * 0.5 * 3.0 * 3.0,
        1e-9,
        "bevel square",
    );
    let r = area_of(vec![stroke(
        p,
        StrokeStyle {
            width: w,
            join: LineJoin::Round,
            ..Default::default()
        },
    )]);
    assert_area(
        r,
        66.0 * 66.0 - 54.0 * 54.0 - 4.0 * 3.0 * 3.0 + std::f64::consts::PI * 3.0 * 3.0,
        5e-3,
        "round square",
    );
}

#[test]
fn miter_limit_falls_back_to_bevel() {
    // 鋭角（10°）の V。miter 比 1/sin(5°) ≈ 11.5 > 10 なので bevel になる
    let p = Path {
        segments: vec![
            Segment::MoveTo(20.0, 20.0),
            Segment::LineTo(120.0, 30.0),
            Segment::LineTo(20.0, 40.0),
        ],
    };
    let limited = area_of(vec![stroke(
        p.clone(),
        StrokeStyle {
            width: 2.0,
            miter_limit: 10.0,
            ..Default::default()
        },
    )]);
    let bevel = area_of(vec![stroke(
        p.clone(),
        StrokeStyle {
            width: 2.0,
            join: LineJoin::Bevel,
            ..Default::default()
        },
    )]);
    let unlimited = area_of(vec![stroke(
        p,
        StrokeStyle {
            width: 2.0,
            miter_limit: 100.0,
            ..Default::default()
        },
    )]);
    assert!((limited - bevel).abs() < 1e-6);
    assert!(unlimited > bevel + 1.0);
}

#[test]
fn dashes_cover_the_on_fraction() {
    let p = Path {
        segments: vec![Segment::MoveTo(10.0, 50.0), Segment::LineTo(130.0, 50.0)],
    };
    let a = area_of(vec![stroke(
        p,
        StrokeStyle {
            width: 2.0,
            dash: vec![3.0, 1.0],
            ..Default::default()
        },
    )]);
    // 120 = 30 周期 × 4、各周期 3 が描かれる
    assert_area(a, 90.0 * 2.0, 1e-9, "dash");
}

#[test]
fn stroke_width_follows_non_uniform_ctm() {
    // x を 3 倍にする行列の下で縦線を引くと、線幅は 3 倍になる
    let p = Path {
        segments: vec![Segment::MoveTo(20.0, 20.0), Segment::LineTo(20.0, 120.0)],
    };
    let a = area_of(vec![Item::Stroke {
        path: p,
        ctm: Matrix::scale(3.0, 1.0),
        style: StrokeStyle {
            width: 2.0,
            ..Default::default()
        },
        color: Color::BLACK,
        alpha: 1.0,
    }]);
    assert_area(a, 100.0 * 6.0, 1e-9, "anisotropic stroke");
}

#[test]
fn clip_intersects_and_pops() {
    let items = vec![
        Item::ClipPush {
            path: Path::rect(0.0, 0.0, 50.0, 200.0),
            ctm: Matrix::IDENTITY,
            rule: FillRule::NonZero,
        },
        Item::ClipPush {
            path: Path::rect(0.0, 0.0, 200.0, 30.0),
            ctm: Matrix::IDENTITY,
            rule: FillRule::NonZero,
        },
        fill(Path::rect(0.0, 0.0, 200.0, 200.0), FillRule::NonZero),
        Item::ClipPop,
        Item::ClipPop,
    ];
    assert_area(area_of(items), 50.0 * 30.0, 1e-9, "clip");
}

#[test]
fn alpha_and_source_over_compose() {
    let mut canvas = Canvas::new(10, 10);
    let list = DisplayList {
        items: vec![
            Item::Fill {
                path: Path::rect(0.0, 0.0, 10.0, 10.0),
                ctm: Matrix::IDENTITY,
                rule: FillRule::NonZero,
                color: Color::Rgb(1.0, 0.0, 0.0),
                alpha: 1.0,
            },
            Item::Fill {
                path: Path::rect(0.0, 0.0, 10.0, 10.0),
                ctm: Matrix::IDENTITY,
                rule: FillRule::NonZero,
                color: Color::Rgb(0.0, 0.0, 1.0),
                alpha: 0.5,
            },
        ],
    };
    render(&list, &mut canvas, &Matrix::IDENTITY);
    let p = canvas.pixels[55];
    assert!(
        (p[0] - 0.5).abs() < 1e-9 && (p[2] - 0.5).abs() < 1e-9 && (p[3] - 1.0).abs() < 1e-9,
        "{p:?}"
    );
}

#[test]
fn page_to_device_flips_y_and_scales_by_dpi() {
    let m = sabirender_raster::page_to_device(100.0, 144.0);
    assert_eq!(m.apply(0.0, 100.0), (0.0, 0.0));
    assert_eq!(m.apply(10.0, 0.0), (20.0, 200.0));
}

#[test]
fn identical_clips_do_not_thin_the_coverage() {
    // 1×1 画素の左半分を塗る。同じ矩形のクリップを 0〜3 回重ねても被覆率は 0.5 のまま
    for count in 0..=3 {
        let p = Path::rect(0.0, 0.0, 0.5, 1.0);
        let mut items = Vec::new();
        for _ in 0..count {
            items.push(Item::ClipPush {
                path: p.clone(),
                ctm: Matrix::IDENTITY,
                rule: FillRule::NonZero,
            });
        }
        items.push(fill(p, FillRule::NonZero));
        let mut canvas = Canvas::new(1, 1);
        render(&DisplayList { items }, &mut canvas, &Matrix::IDENTITY);
        assert!(
            (canvas.coverage_sum() - 0.5).abs() < 1e-9,
            "clips {count}: {}",
            canvas.coverage_sum()
        );
    }
}

#[test]
fn clip_and_fill_that_do_not_overlap_inside_a_pixel_give_zero() {
    // 左半分のクリップと右半分の塗り: 被覆率の積なら 0.25 になるが、交叉は空
    let items = vec![
        Item::ClipPush {
            path: Path::rect(0.0, 0.0, 0.5, 1.0),
            ctm: Matrix::IDENTITY,
            rule: FillRule::NonZero,
        },
        fill(Path::rect(0.5, 0.0, 0.5, 1.0), FillRule::NonZero),
    ];
    let mut canvas = Canvas::new(1, 1);
    render(&DisplayList { items }, &mut canvas, &Matrix::IDENTITY);
    assert!(canvas.coverage_sum().abs() < 1e-9);
}

#[test]
fn glyphs_are_placed_by_their_own_transform() {
    // 1000 単位の正方形を 0.01 倍して (30, 20) に置く: 10×10 の塗り
    let run = sabirender_display::GlyphRun {
        glyphs: vec![sabirender_display::PlacedGlyph {
            outline: Path::rect(0.0, 0.0, 1000.0, 1000.0),
            transform: Matrix::scale(0.01, 0.01).then(&Matrix::translate(30.0, 20.0)),
        }],
    };
    let items = vec![Item::Glyphs {
        run,
        ctm: Matrix::scale(2.0, 1.0),
        color: Color::BLACK,
        alpha: 1.0,
    }];
    let mut canvas = Canvas::new(200, 200);
    render(&DisplayList { items }, &mut canvas, &Matrix::IDENTITY);
    // ctm で x が 2 倍: 20×10
    assert_area(canvas.coverage_sum(), 200.0, 1e-9, "glyph");
    assert!(canvas.pixels[25 * 200 + 65][3] > 0.99);
    assert!(canvas.pixels[25 * 200 + 55][3] < 0.01);
}

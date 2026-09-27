//! 独立した処理系との画像比較（RENDER-IMAGE-PDFTOPPM、C-DRAW）。
//!
//! 手で書いた小さな PDF（塗り、円、線、偶奇規則、クリップ）を poppler の `pdftoppm` で 72 dpi のグレースケールに描き、
//! 同じ内容ストリームを SabiRender の評価器 + 参照ラスタライザで描いた被覆率と比べる。
//! アンチエイリアスの方式は PDF 仕様が定めないので画素完全ではなく、[cases.md](../../../specification/cases.md) に記した
//! 校正済みの閾値で比べる（平均絶対差、反対側の画素の数、インクの総量）。
//! `pdftoppm` が無ければ BLOCKED（oracle プロファイルでは必須）。

use sabirender_content::{Evaluator, NoResources};
use sabirender_display::DisplayList;
use sabirender_qa::{run_tool, Case};
use sabirender_raster::{page_to_device, render, Canvas};

const PAGE: f64 = 100.0;

/// 内容ストリーム: 矩形、円（4 本の三次ベジエ）、線幅 3 の線、偶奇規則の入れ子の正方形、矩形クリップの中の塗り
const CONTENT: &str = "\
0 0 0 rg
10 10 50 30 re f
70 70 m 70 78.284 76.716 85 85 85 c 93.284 85 100 78.284 100 70 c 100 61.716 93.284 55 85 55 c 76.716 55 70 61.716 70 70 c f
0 0 0 RG 3 w 10 80 m 90 20 l S
5 55 30 30 re 15 65 10 10 re f*
q 60 5 35 35 re W n 50 0 50 50 re f Q
";

/// 1 ページの非圧縮 PDF（xref つき）
fn pdf_bytes() -> Vec<u8> {
    let mut out = Vec::new();
    let mut offsets = Vec::new();
    out.extend_from_slice(b"%PDF-1.4\n");
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {PAGE} {PAGE}] /Contents 4 0 R /Resources << >> >>"),
        format!(
            "<< /Length {} >>\nstream\n{}endstream",
            CONTENT.len(),
            CONTENT
        ),
    ];
    for (i, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{}\nendobj\n", i + 1, body).as_bytes());
    }
    let xref = out.len();
    out.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
    );
    for o in &offsets {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    out
}

/// PGM（P5、8 ビット）を被覆率（0 = 白、1 = 黒）に読む
fn parse_pgm(data: &[u8]) -> Option<(usize, usize, Vec<f64>)> {
    let mut fields = Vec::new();
    let mut i = 0;
    while fields.len() < 4 && i < data.len() {
        while i < data.len() && data[i].is_ascii_whitespace() {
            i += 1;
        }
        if data.get(i) == Some(&b'#') {
            while i < data.len() && data[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        let s = i;
        while i < data.len() && !data[i].is_ascii_whitespace() {
            i += 1;
        }
        fields.push(String::from_utf8_lossy(&data[s..i]).into_owned());
    }
    if fields.len() < 4 || fields[0] != "P5" || fields[3] != "255" {
        return None;
    }
    let w: usize = fields[1].parse().ok()?;
    let h: usize = fields[2].parse().ok()?;
    i += 1;
    let px = data.get(i..i + w * h)?;
    Some((w, h, px.iter().map(|&g| 1.0 - g as f64 / 255.0).collect()))
}

#[test]
fn reference_page_matches_pdftoppm_within_calibrated_thresholds() {
    let case = Case::required("RENDER-IMAGE-PDFTOPPM", &["C-DRAW"]);
    let dir = std::env::temp_dir().join(format!("sabirender-oracle-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("p.pdf"), pdf_bytes()).unwrap();
    if run_tool(
        &case,
        "pdftoppm",
        &["-r", "72", "-gray", "-singlefile", "p.pdf", "ref"],
        Some(&dir),
    )
    .is_none()
    {
        let _ = std::fs::remove_dir_all(&dir);
        return case.blocked("pdftoppm (poppler) not found");
    }
    let pgm = std::fs::read(dir.join("ref.pgm"));
    let _ = std::fs::remove_dir_all(&dir);
    let Ok(pgm) = pgm else {
        case.tool_failed("pdftoppm produced no ref.pgm");
    };
    let Some((w, h, reference)) = parse_pgm(&pgm) else {
        case.tool_failed("ref.pgm is not an 8-bit P5 PGM");
    };
    assert_eq!(
        (w, h),
        (PAGE as usize, PAGE as usize),
        "pdftoppm image size"
    );
    case.compared();

    // 同じ内容を SabiRender で
    let mut list = DisplayList::default();
    let mut ev = Evaluator::new(sabirender_display::Matrix::IDENTITY, &NoResources);
    ev.run(CONTENT.as_bytes(), &mut list);
    ev.finish(&mut list);
    assert!(
        list.unsupported().next().is_none(),
        "content fully supported"
    );
    let mut canvas = Canvas::new(w, h);
    let report = render(&list, &mut canvas, &page_to_device(PAGE, 72.0));
    assert!(
        report.skipped.is_empty() && report.budget.is_empty(),
        "{report:?}"
    );
    let ours: Vec<f64> = canvas.pixels.iter().map(|p| p[3]).collect();

    // 指標
    let n = (w * h) as f64;
    let mean_abs = ours
        .iter()
        .zip(&reference)
        .map(|(a, b)| (a - b).abs())
        .sum::<f64>()
        / n;
    let opposite = ours
        .iter()
        .zip(&reference)
        .filter(|(a, b)| (*a - *b).abs() > 0.5)
        .count();
    let ink_ours: f64 = ours.iter().sum();
    let ink_ref: f64 = reference.iter().sum();
    let ink_rel = (ink_ours - ink_ref).abs() / ink_ref;
    eprintln!(
        "RENDER-IMAGE-PDFTOPPM: mean_abs={mean_abs:.5} opposite={opposite} ink_ours={ink_ours:.2} ink_ref={ink_ref:.2} ink_rel={ink_rel:.5}"
    );
    // 校正のための内訳（ページ座標の箱ごとのインク。装置座標は y が反転）
    let ink_in = |v: &[f64], x0: f64, y0: f64, x1: f64, y1: f64| -> f64 {
        let mut s = 0.0;
        for y in 0..h {
            for x in 0..w {
                let (px, py) = (x as f64 + 0.5, PAGE - (y as f64 + 0.5));
                if px >= x0 && px < x1 && py >= y0 && py < y1 {
                    s += v[y * w + x];
                }
            }
        }
        s
    };
    for (name, b) in [
        ("rect", (9.0, 9.0, 61.0, 41.0)),
        ("circle", (54.0, 54.0, 100.0, 86.0)),
        ("squares", (4.0, 54.0, 36.0, 86.0)),
        ("clip", (49.0, 0.0, 100.0, 51.0)),
        ("line-only", (36.0, 41.0, 54.0, 62.0)),
    ] {
        eprintln!(
            "  {name}: ours={:.2} ref={:.2}",
            ink_in(&ours, b.0, b.1, b.2, b.3),
            ink_in(&reference, b.0, b.1, b.2, b.3)
        );
    }
    // 閾値は cases.md の校正値（観測値の 3 倍以上の余裕）。poppler は線を 0.2 画素ほど太く描く（stroke adjust）ので、
    // インクの総量は解析値に近い側（SabiRender）が少なくなる
    assert!(mean_abs <= 0.01, "mean absolute difference {mean_abs}");
    case.compared();
    assert!(opposite <= 8, "{opposite} pixels on the opposite side");
    case.compared();
    assert!(ink_rel <= 0.02, "ink differs by {ink_rel}");
    case.compared();
    case.done();
}

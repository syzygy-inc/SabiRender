# 契約 case 台帳

品質保証指針 `qa-v0`（`SabiSeries/qa/`）の [記録テンプレート](../../SabiSeries/qa/record-template.md) に対応する、このリポジトリの予定 case。
SabiRender の検査の大半は外部の oracle に依らず、期待値をすべて手計算（解析図形の面積・位置・被覆率・変換後の座標）で持つ。
これらは `cargo test --workspace` で常に実行され、環境不足による BLOCKED は生じない。テストを skip / ignore で外さない。

独立した処理系との画像比較（RENDER-IMAGE-PDFTOPPM）だけは poppler の `pdftoppm` を参照ツールとし、
`crates/sabirender-qa` の `Case` で台帳（`target/qa-ledger/*.tsv`、または `SABI_QA_LEDGER`）に PASS / FAIL / BLOCKED / NOT-RUN を残す。
`scripts/qa-ledger.sh` が下の表の「台帳」列にある case と突き合わせる。`required` は oracle プロファイルで必須（無ければ BLOCKED で失敗）。

## 台帳に載せる case（oracle プロファイル）

| case | 契約 | プロファイル | 必須 | 参照資源 | 内容 |
|---|---|---|---|---|---|
| RENDER-IMAGE-PDFTOPPM | C-DRAW | render-image | required | pdftoppm（poppler） | 手書きの PDF（矩形、円、線、偶奇規則、クリップ）を pdftoppm の 72 dpi グレースケールと比べる。下の校正値で判定 |

## 常に実行する case（手計算の期待値）

| case | 契約 | 不変条件（[qa.md](qa.md)） | テスト | 内容 |
|---|---|---|---|---|
| RENDER-CONTENT-STATE | C-DRAW | 内容評価器・図形状態 | `sabirender-content` 単体 `fill_and_stroke_*`、`cm_composes_and_q_restores`、`clip_is_pushed_*`、`unbalanced_q_*`、`form_xobject_*` | 塗り・線の項目と状態、q/Q、clip の範囲、フォームの行列と bbox クリップ |
| RENDER-CONTENT-CTM | C-DRAW | 内容評価器 | `cm_in_the_middle_of_a_path_*`、`stroke_width_uses_the_ctm_at_painting_time` | 経路構築時 CTM と stroke 時 CTM の区別（PDF §8.5.2） |
| RENDER-CONTENT-STYLE | C-DRAW | 内容評価器 | `dash_and_caps_are_recorded`、`even_odd_and_unsupported_are_distinguished` | 破線・端点、fill rule、未対応演算子の診断 |
| RENDER-CONTENT-NUMERIC | C-RESULT, C-RESOURCE | 内容評価器・matrix | `non_finite_operands_are_diagnosed_not_drawn`、`stroke_under_a_singular_ctm_is_diagnosed`、`path_under_a_singular_ctm_is_diagnosed` | 非有限の被演算子は診断にして描かない。特異 CTM の stroke / fill / clip は診断を残す |
| RENDER-BUDGET | C-RESOURCE | 内容評価器・flatten | `path_segment_budget_is_reported_once`、`clip_nesting_budget_is_reported`、`form_xobject_*`（深さ 16）、`an_extreme_curve_reports_the_subdivision_budget` | 経路の線分数・クリップの入れ子・フォーム再帰・曲線分割の深さの予算。到達は `budget:` の診断 / `RenderReport.budget` で成功と区別する |
| RENDER-MATRIX | C-DRAW | matrix/path | `sabirender-display` 単体 `matrix_composition_*`、`cmyk_to_rgb_*` | 合成順（PDF の cm）と色変換 |
| RENDER-FLATTEN | C-DRAW | flatten | `a_quarter_circle_flattens_within_tolerance` | 半径 100 の四分円が許容誤差内、予算に達しない |
| RENDER-FILL-AREA | C-DRAW | clip/raster | `analytic.rs` の `*_has_exact_area`、`circle_area_*`、`nonzero_*`、`evenodd*` | 解析面積と fill rule |
| RENDER-FILL-POSITION | C-DRAW | clip/raster・座標 | `fractional_rectangle_edges_*`、`circle_is_where_it_should_be`、`page_to_device_*` | 位置と局所画素の被覆率、ページ空間から装置空間への写像 |
| RENDER-STROKE | C-DRAW | stroke | `butt_stroke_*`、`square_and_round_caps_*`、`miter_join_*`、`miter_limit_*`、`dashes_cover_*`、`stroke_width_follows_non_uniform_ctm`、`dash_splits_*` | 端点・接合・miter limit・破線・非等方 CTM の線幅 |
| RENDER-CLIP | C-DRAW | clip/raster | `clip_intersects_and_pops`、`identical_clips_do_not_thin_the_coverage`、`clip_and_fill_that_do_not_overlap_*` | 交差、冪等、非交差 |
| RENDER-COMPOSITE | C-DRAW | composite | `alpha_and_source_over_compose`、`glyphs_are_placed_by_their_own_transform` | source-over と alpha、字形の行列 |
| RENDER-NUMERIC | C-RESOURCE | matrix/path・stroke | `non_finite_geometry_is_skipped_without_panicking` | NaN/無限大の経路・CTM・線の様式は描かず `RenderReport.skipped` に残す |
| RENDER-CAPACITY | C-RESOURCE | clip/raster | `canvas_size_is_checked_before_allocation` | 画素数上限（2^24）と乗算あふれを確保前に検査 |
| RENDER-OUTPUT | C-RESULT | PNG/SVG/crop | `writes_paths_clips_and_glyphs_with_flipped_y`、`bounds_cover_strokes_and_glyphs`、`crc_and_adler_*`、`encodes_a_valid_structure` | SVG の座標と要素、bounds、PNG の構造 |

## 画像比較の校正（RENDER-IMAGE-PDFTOPPM）

アンチエイリアスの方式は PDF 仕様が定めないので、画素完全ではなく次の 3 つの指標で比べる。被覆率は 0（白）〜1（黒）。

| 指標 | 閾値 | 観測値（poppler 25.02.0、TeX Live 2025 の `pdftoppm`、2026-09-28） |
|---|---|---|
| 平均絶対差（全画素） | ≤ 0.01 | 0.0028 |
| 反対側の画素（差 > 0.5）の数 | ≤ 8 | 0 |
| インクの総量の相対差 | ≤ 0.02 | 0.0060 |

内訳（インクの総量、ページ座標の箱ごと）: 矩形 1531.8 / 1532.0、円 706.2 / 709.3（解析値 706.9）、偶奇の正方形 834.7 / 846.1、
クリップ 1607.0 / 1615.1、線だけの箱 67.4 / 72.3（解析値 67.5）。塗りは両者とも解析値に近く、差はほぼ線に由来する。
poppler（Splash）は線を 0.2 画素ほど太く描く（stroke adjustment）ので、インクは SabiRender の方が解析値に近い側で少ない。
閾値は観測値の 3 倍以上の余裕を取った。poppler の版が変わって観測値が動いたら、この表を更新して変更管理に載せる。

## 未保証（保証済みに数えない）

- 画像比較は 1 枚の手書き PDF に対するもの。PGF の実際の出力（シェーディング、パターン、透明グループ）は対象外で、
  それらは評価器が `Unsupported` にする。色空間・alpha の比較は未校正（比較はグレースケールの被覆率だけ）。
- GPU 後端との一致（後端が無い）。
- 予算は display list 全体の大きさ（項目数）には無い。

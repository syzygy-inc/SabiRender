# 契約 case 台帳

品質保証指針 `qa-v0`（`SabiSeries/qa/`）の [記録テンプレート](../../SabiSeries/qa/record-template.md) に対応する、このリポジトリの予定 case。
SabiRender の検査は外部の oracle（TeX Live 等）に依らず、期待値をすべて手計算（解析図形の面積・位置・被覆率・変換後の座標）で持つ。
したがって環境不足による BLOCKED は生じず、`cargo test --workspace` で全 case が常に実行される。tsv の台帳と照合スクリプトは置かず、
本表が予定 case の一覧である。テストを skip / ignore で外さない。他の処理系との画像比較はまだ required の case にしない（下記「未保証」）。

| case | 契約 | 不変条件（[qa.md](qa.md)） | テスト | 内容 |
|---|---|---|---|---|
| RENDER-CONTENT-STATE | C-DRAW | 内容評価器・図形状態 | `sabirender-content` 単体 `fill_and_stroke_*`、`cm_composes_and_q_restores`、`clip_is_pushed_*`、`unbalanced_q_*`、`form_xobject_*` | 塗り・線の項目と状態、q/Q、clip の範囲、フォームの行列と bbox クリップ |
| RENDER-CONTENT-CTM | C-DRAW | 内容評価器 | `cm_in_the_middle_of_a_path_*`、`stroke_width_uses_the_ctm_at_painting_time` | 経路構築時 CTM と stroke 時 CTM の区別（PDF §8.5.2） |
| RENDER-CONTENT-STYLE | C-DRAW | 内容評価器 | `dash_and_caps_are_recorded`、`even_odd_and_unsupported_are_distinguished` | 破線・端点、fill rule、未対応演算子の診断 |
| RENDER-CONTENT-NUMERIC | C-RESULT, C-RESOURCE | 内容評価器・matrix | `non_finite_operands_are_diagnosed_not_drawn`、`stroke_under_a_singular_ctm_is_diagnosed` | 非有限の被演算子は診断にして描かない。特異 CTM の stroke は診断を残す |
| RENDER-MATRIX | C-DRAW | matrix/path | `sabirender-display` 単体 `matrix_composition_*`、`cmyk_to_rgb_*` | 合成順（PDF の cm）と色変換 |
| RENDER-FLATTEN | C-DRAW | flatten | `a_quarter_circle_flattens_within_tolerance` | 半径 100 の四分円が許容誤差内、分割深さ 16 で終端 |
| RENDER-FILL-AREA | C-DRAW | clip/raster | `analytic.rs` の `*_has_exact_area`、`circle_area_*`、`nonzero_*`、`evenodd*` | 解析面積と fill rule |
| RENDER-FILL-POSITION | C-DRAW | clip/raster・座標 | `fractional_rectangle_edges_*`、`circle_is_where_it_should_be`、`page_to_device_*` | 位置と局所画素の被覆率、ページ空間から装置空間への写像 |
| RENDER-STROKE | C-DRAW | stroke | `butt_stroke_*`、`square_and_round_caps_*`、`miter_join_*`、`miter_limit_*`、`dashes_cover_*`、`stroke_width_follows_non_uniform_ctm`、`dash_splits_*` | 端点・接合・miter limit・破線・非等方 CTM の線幅 |
| RENDER-CLIP | C-DRAW | clip/raster | `clip_intersects_and_pops`、`identical_clips_do_not_thin_the_coverage`、`clip_and_fill_that_do_not_overlap_*` | 交差、冪等、非交差 |
| RENDER-COMPOSITE | C-DRAW | composite | `alpha_and_source_over_compose`、`glyphs_are_placed_by_their_own_transform` | source-over と alpha、字形の行列 |
| RENDER-NUMERIC | C-RESOURCE | matrix/path・stroke | `non_finite_geometry_is_skipped_without_panicking` | NaN/無限大の経路・CTM・線の様式は描かず `RenderReport.skipped` に残す |
| RENDER-CAPACITY | C-RESOURCE | clip/raster | `canvas_size_is_checked_before_allocation` | 画素数上限（2^24）と乗算あふれを確保前に検査 |
| RENDER-OUTPUT | C-RESULT | PNG/SVG/crop | `writes_paths_clips_and_glyphs_with_flipped_y`、`bounds_cover_strokes_and_glyphs`、`crc_and_adler_*`、`encodes_a_valid_structure` | SVG の座標と要素、bounds、PNG の構造 |

## 未保証（保証済みに数えない）

- 他の処理系（pdftoppm、mutool 等）との画像比較。色空間・alpha・閾値を校正してから required にする。現在は SabiDVI 側の
  dvipdfmx との幾何比較（DVI-PGF-DVIPDFMX）が、評価器の読み直しを通じて間接に内容評価器を検査しているだけである。
- GPU 後端との一致（後端が無い）。
- 予算: 曲線分割の深さ 16 とフォーム再帰の深さ 16 は持つが、経路数・clip 数・display list 全体の大きさに予算がない。
  予算到達を成功と区別する報告もまだない。
- 特異 CTM の塗り（Fill / ClipPush）は経路が退化するだけで診断を残さない（stroke のみ診断）。

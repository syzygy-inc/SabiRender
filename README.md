# SabiRender

SabiDVI と SabiPDF が用いる 2D テクスチャのレンダラ。PDF の図形モデル（ISO 32000-1 §8）に従い、入口から受け取った描画命令列を画素にする。
3DCG を想定したレンダラは Sabi 系列とは別に作る。

| クレート | 内容 |
|---|---|
| `sabirender-display` | 装置非依存の描画命令列（塗り、線、クリップ、字形、画像）。入口と後端の唯一の接点 |
| `sabirender-content` | PDF 内容ストリームの字句解析と図形演算子の評価器。図形状態を追跡して描画命令列を出す |
| `sabirender-raster` | CPU 参照ラスタライザ。走査線法（縦 16 副走査線、横は厳密な被覆率）、線の多角形化、クリップ、source-over 合成 |

設計は `specification/design.md`。

## 検証の方針

- ラスタライザは解析的に面積の分かる図形で検証する（`crates/sabirender-raster/tests/analytic.rs`）。
- 入口（SabiDVI）は dvipdfmx が出す PDF を幾何として読み直して比較する。読み直しは SabiPDF の入口と同じ評価器で行う。
- 忠実度の基準は TikZ / PGF のマニュアル。アンチエイリアスは PDF 仕様が定めないので、画素完全ではなく許容誤差で比べる。

## 開発

```sh
cargo test --workspace
cargo clippy --workspace --all-targets
```

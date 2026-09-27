# SabiRender 内部品質保証

策定: 2026-09-24。参照設計版: `qa-v0`。系列 [契約](../../SabiSeries/qa/contracts.md) の C-DRAW / C-RESULT / C-RESOURCE を担保する。PDF 的な描画意味論を保つことと、現行 CPU ラスタライザの内部方式を固定することは分ける。

## 内部不変条件

| 領域 | 維持する条件 | 局所検証 |
|---|---|---|
| 内容評価器 | 経路構築時 CTM と stroke 時 CTM、current point を区別 | cm途中、q/Q途中、曲線短縮演算子、非等方変換 |
| 図形状態 | stack と clip の範囲、描画順、フォーム復帰を維持 | 入れ子・不整合・フォーム内外の経路と状態 |
| matrix/path | 合成順・単位・orientation の一貫性 | 恒等、移動、回転、reflection、特異/非有限入力 |
| flatten | 終端し、宣言した装置空間の誤差を満たす | 高曲率・退化・極端な拡大、深さ上限 |
| stroke | cap/join/miter/dash の意味を利用者空間で保つ | 解析面積、方向差、zero width、ゼロ長/不正dash |
| clip/raster | 区間の順序と交差、fill rule、被覆率の範囲 | 冪等、非交差、穴、辺の一致、位置と総面積 |
| composite | source-over と alpha、画素境界、順序 | 透明/不透明、重なり、色とalphaの独立検査 |
| PNG/SVG/crop | 実効寸法・座標・出力の整合 | 復号寸法、bounds、空出力、glyph/path同値 |

区間や cache の内部表現は変えてよい。AA の副走査線数や配列順そのものを外部契約テストにしない。ただしプロファイルの誤差予算を変更する場合は系列側の変更管理を通す。[design.md](design.md) に現行方式を記録し、外部保証と区別する。

## 検証の分担

内容評価器は手で求まる page-space の点、線幅、色、clip scope を比較する。ラスタライザは解析図形の面積に加え、位置・局所画素・alpha を比較する。内部の同じ `bounds()` を描画結果と期待値の両側に用いて正しさを判断しない。

SabiDVI の PDF 比較に共通利用される評価器であるため、外部から渡された display list を描く試験と、内容ストリームを解釈する試験を別に保つ。将来の GPU 後端との一致は CPU の実装バグも共有し得るので、CPU/GPU の両方を独立した少数 fixture に照合する。

```sh
cargo test --workspace
```

既存の `sabirender-raster/tests/analytic.rs`、content の CTM 回帰、SVG の検査を維持する。新たな独立画像比較は色空間・alpha・閾値を校正してから required にする。今ある解析テストの成功で全 PDF 描画の適合を宣言しない。

## 入力と容量

NaN/無限大、特異行列、負寸法、不正dash、空path、unbalanced clip の扱いを公開境界で定める。エラー型を返せない API なら診断を付ける adapter/API 改訂を検討し、黙って正常な描画に置換して Complete にしない。

画素数・buffer size の checked arithmetic と上限検証を確保前に行う。curve subdivision、フォーム再帰、path/clip 数に予算を持ち、予算到達を成功と区別する。有限の合法的な小入力の正常試験と、不正・過大入力の打切り試験を対で維持する。

## Lean の初期対象

有限な整列済み区間列の交差について、冪等性・可換性・包含・非交差を証明する。f64 の実装に対する保証は、非有限値の排除、順序比較の仕様、端点丸め、正規化の対応を別に示す。実数の集合論の証明をそのまま画素誤差の証明としない。

契約 theorem を保持して、内部 interval container や走査手順の helper lemma は変更可能にする。Rust との比較 fixture と入力生成を併用する。形式検証の採用段階は [系列方針](../../SabiSeries/qa/formal-methods.md) に従う。

## 当面の完了条件

CTM と clip の回帰を維持し、異常数値・容量・独立した観測値の不足を整理する。pixel 比較の閾値未決定や特異行列の扱い未定を、現状で保証済みの項目に数えない。内部方式の改良は契約の観測値と予算を満たす限り許容する。

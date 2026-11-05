# 付録 C：日本語全文検索の比較と意思決定

> 調査日 2026-11-05。結論は ADR-007 と `00-全体設計.md` の D-02 に反映済み。
> 本稿が答える問い：**なぜ「三択」ではなく「階層 + クエリルーティング」なのか**。

---

## 1. 問題の根

PostgreSQL の組み込み全文検索は「トークナイザ + 辞書」に依存する。空白と句読点で語を切り、lexeme に正規化する。日本語には語間空白がないため、`to_tsvector('simple', '住所チェックの桁数')` は文字列全体を 1 token として扱い、検索が成立しない。

回避策は 3 つあり、トレードオフが全く異なる。

| 方式 | 本質 | 一言 |
|---|---|---|
| 事前分割 + tsvector | **語**単位の転置索引 | 日本語を自分で分割し、PG 標準の全文索引に載せる |
| pg_bigm | **文字**単位の転置索引（2-gram） | 全隣接 2 文字を GIN に入れ、`LIKE '%…%'` を加速 |
| PGroonga | **独立エンジン**に索引を任せる | Groonga の tokenizer + normalizer で PG の検索スタックを置換 |

速い遅いの差ではなく、「語で捉えるか、文字で捉えるか」というパラダイムの差である。

---

## 2. 各方式の仕組み

### 2.1 事前分割 + tsvector

```text
原文: 住所は全角20文字以内です
  ↓ Lindera (IPADIC) 形態素解析
[住所][は][全角][20][文字][以内][です]
  ↓ 空白連結して tokenized 列へ
"住所 は 全角 20 文字 以内 です"
  ↓ to_tsvector('simple', tokenized)
'20':4 'です':7 'は':2 '以内':6 '全角':3 '住所':1 '文字':5
```

- クエリ側も**同一 tokenizer・同一辞書版**で分割する必要がある。ずれると token が一致しない。
- 語項の論理積（`&`）、フレーズ（`<->`）、前方一致（`:*`）に対応。
- 部分一致は不可。`住所チェ` は `住所チェック` に当たらない（索引にその token がない）。
- 未知語（製品名、型番、外字、旧字体）が砕けるのが主な失点。

### 2.2 pg_bigm

```text
原文: 住所チェック
  ↓ 全隣接 2 文字
[住所][所チ][チェ][ェッ][ック]
  ↓ GIN
クエリ LIKE '%住所チェ%'
  → bigram の積集合で候補を絞る
  → recheck：各行を走査して本当に含むか確認（偽陽性の除去）
```

- 辞書不要。未知語、外字、型番に強い。
- **recheck が必須**。bigram の積集合は「含む」を保証しない。ヒットが多いと recheck が bottleneck になる。
- 1 文字クエリは全表走査に落ちる。
- 関連度評価は弱い。`LIKE` にスコアはない。別途 `ts_rank` かアプリ層の並び替えが要る。
- GIN は crash-safe。異常終了後の再構築は不要。

### 2.3 PGroonga

```text
原文 → Groonga tokenizer（N-gram または TokenMecab）
     → normalizer（全半角、大小、旧字体の統一）
     → Groonga 転置索引（PG の index access method）
クエリ &@~ '住所 チェック'
     → 索引内で完全一致。recheck 不要
```

- 日英中混在と正規化（`髙`→`高`）に強い。日本の設計書向き。
- recheck がないため、ヒット多でも速い。
- 索引が大きい（公式基準で pg_bigm の約 2.3 倍）。
- **crash-safe ではない**。異常終了で索引が壊れると `REINDEX` が必要。無人運用の顧客内網では実運用の負担。
- Groonga ライブラリと拡張の導入が必要。顧客 DBA の承認が下りるとは限らない。

---

## 3. 比較

| 観点 | 事前分割 + tsvector | pg_bigm | PGroonga |
|---|---|---|---|
| 索引単位 | 語（lexeme） | 2-gram | 語 / N-gram（設定可） |
| 日本語分割の品質 | Lindera 辞書に依存 | 辞書不要 | Groonga tokenizer に依存 |
| 部分一致（`住所チェ`） | 不可 | 可 | 可 |
| 未知語 / 外字 | 弱い（砕けやすい） | 強い | 強い（正規化可） |
| 偽陽性 | なし（語項で一致） | recheck（回表） | なし（索引内で一致） |
| 関連度 | `ts_rank_cd`。成熟 | 弱い（別途必要） | 組み込みスコア。比較的強い |
| 索引サイズ（相対） | 中 | 小（基準 1.0） | 大（約 2.3 倍） |
| 書き込みコスト | 中 | 中〜高（gram が多い） | 中〜高 |
| 多ヒット時の遅延 | 中 | 高（recheck。公式旧基準で約 50 倍） | 低 |
| 耐障害性 | あり（PG ネイティブ） | あり（GIN） | **なし**（REINDEX 必須） |
| 拡張依存 | **なし** | pg_bigm | PGroonga + Groonga |
| マネージド PG | すべて | 一部 | 一部 |
| ライセンス | PostgreSQL License | PostgreSQL License | PostgreSQL License |
| 運用負担 | 低 | 中 | 高 |
| 向く用途 | 語単位、可搬性 | 部分一致、型番/外字 | 高並行、多言語、既存 Groonga |

> 性能値の注意：PGroonga 公式の比較は PostgreSQL 9.6 時代のもの。桁の参考にはなるが、選定根拠にはしない。Phase 2 で実設計書により自測する（§6）。

---

## 4. 実クエリで見る差

以下は機序からの推定。Phase 2 で正解集により検証する。

| クエリ | 事前分割 + tsvector | pg_bigm | PGroonga | 結論 |
|---|---|---|---|---|
| `住所チェック`（完整な語） | 命中。語項 AND | 命中。recheck あり | 命中 | 三者可 |
| `住所チェ`（入力途中） | **不命中** | 命中 | 命中 | 逐次検索では tsvector 不可 |
| `髙橋`（旧字体） | 砕ける/未知語 | 字面で命中 | normalizer で統一 | 外字は bigram/PGroonga が安定 |
| `SALE_CD`（識別子） | `SALE` `_` `CD` に分解 | 部分一致で命中 | 命中 | コード/型番は bigram 寄り |
| `12,345`（記号付き数字） | 分割が不安定 | 命中 | 命中 | 同上 |
| `住所 AND 必須`（論理式） | 原生対応 | アプリ層で合成 | 対応 | 構造化検索は tsvector 寄り |

要点：**現場の検索は両方ある**。設計書には自然文（語単位が向く）と、項目名・型番・半角カナ・コード識別子（文字単位が向く）が混在する。片方だけでは必ずもう片方が破綻する。

---

## 5. 決定：階層 + クエリルーティング

### 5.1 3 層

```text
L1 tokenized   Lindera IPADIC → tokenized → tsvector(simple) + GIN
               常時有効。拡張不要。「基盤が動く」下限

L2 bigram      pg_bigm GIN on chunk.text
               拡張が検出できれば自動有効。なければ L1 のみで継続

L3 groonga     PGroonga
               顧客環境に既にある場合のみ接続。基盤の要求にしない
```

```rust
pub struct FulltextCapabilities {
    pub tokenized: bool,   // 常に true
    pub bigram: bool,      // 実行時プローブ
    pub groonga: bool,     // 実行時プローブ
    pub phrase: bool,      // L1 は <->。L2 は非対応
    pub substring: bool,   // L2 / L3 のみ
}
```

能力マトリクスは `supported / unsupported` で表示。`unknown` の暗黙降格はしない。

### 5.2 ルーティング

```text
analyze(query):
  normalized = normalize(query)        // 全角→半角、旧字体→新字体、大小
  tokens     = lindera.tokenize(normalized)

  if looks_like_identifier(normalized):     // _ . / : を含む、数字英字混在、型番
      route = BIGRAM
  elif is_quoted(normalized):               // "..." の完全一致
      route = BIGRAM
  elif len(normalized) <= 4 and not has_space(normalized):
      route = BIGRAM                        // 短入力。部分一致を優先
  else:
      route = TOKENIZED

  if bigram_available and route == TOKENIZED:
      parallel_both = true                  // 両方走らせ RRF
```

融合は RRF（`k=60`）。ベクトルと合わせて三路：

```text
score = 1/(60+rank_vec) + 1/(60+rank_fts) + 1/(60+rank_bigram)
```

`search_response.lexical_mode`（`tokenized` / `bigram` / `both`）を返す。「引けない」のが索引の問題か経路の問題かを後から切り分けられる。

### 5.3 PGroonga を既定にしない理由

1. **異常終了で REINDEX が必要**。顧客内網は停電も kill -9 もある。検索停止は「保守運用」で許容されない。
2. **拡張の承認**。日本の顧客 DBA は第三者拡張に慎重。pg_bigm でも説明が要る。PGroonga は Groonga ライブラリも伴う。
3. **索引サイズ**。設計書コーパスは元々大きい（数万ページ + 数千 sheet）。2.3 倍はディスク予算の赤線に触れる。

既に PGroonga を使っている顧客（既存システム）では再利用する。反対はしない。ただし基盤の既定依存にはしない。

### 5.4 tsvector だけにしない理由

「入力途中の部分一致」をカバーできない。検索ボックスに「住所チェ」と打ったとき「住所チェック」が出ることは、基本期待であって高度な機能ではない。ベクトル検索でも安定しない。型番と外字には意味的近傍がない。

---

## 6. Phase 2 のベンチマーク計画

### 6.1 コーパス

- 実構造の設計書 6 冊（PDF 200 ページ + XLSX 12 sheet ×2）
- 約 8,000 chunk
- 自然文、表のキー・バリュー、型番、半角カナ、旧字体、英字識別子を含む

### 6.2 クエリ集（60 件、人手注釈）

| 分類 | 件数 | 例 | 期待 |
|---|---|---|---|
| 語単位 | 20 | 「住所 桁数 チェック」 | L1 が命中 |
| 部分一致 | 20 | 「住所チェ」「ユーザ編」 | L2 が命中 |
| 識別子 / 外字 | 20 | 「SALE_CD」「髙橋」「−（全角マイナス）」 | L2 が命中 |

### 6.3 指標

| 指標 | 目標 | 備考 |
|---|---|---|
| recall@10 | 0.85 以上 | 3 分類別に算出。平均で弱点を隠さない |
| MRR | 0.60 以上 | 最初の命中の平均逆順位 |
| P95 遅延 | 300ms 未満 | 単路。融合後 600ms 未満 |
| 索引サイズ | コーパスの 15% 未満 | tsvector + bigram |
| 書き込み増幅 | 2.5 倍未満 | 純テキスト挿入比 |
| 構築時間 | 30 分未満 | 8,000 chunk |

### 6.4 判定規則

```text
L1 の語単位 recall < 0.75        → 辞書を確認。必要なら UniDic（法務確認）
L2 の部分一致 recall < 0.80      → pg_bigm のパラメータ（gin_key_limit）か trigram を検討
2 分類の差が 25 ポイント超       → 「ルーティング + 二路融合」を必須と確定（任意ではない）
P95 > 800ms                      → top_k 縮小、rerank 停止、PGroonga を評価
```

結果は `docs/operation/search-benchmark.md` に記録し、以後の調整基準とする。

---

## 7. 実装要点

### 7.1 索引と列

```sql
-- L1：常に存在
ALTER TABLE kb.chunk ADD COLUMN tokenized text NOT NULL DEFAULT '';
CREATE INDEX chunk_fts_idx ON kb.chunk USING gin (fts) WHERE is_current;

-- L2：拡張がある場合のみ（マイグレーションで条件分岐）
CREATE INDEX chunk_bigm_idx ON kb.chunk USING gin (text gin_bigm_ops)
  WHERE is_current;
```

`text` は原文（正規化しない）、`tokenized` は正規化後の語列。責務が違うので統合しない。

### 7.2 tokenizer 版を索引メタデータに書く

```json
{
  "tokenizer": "lindera",
  "tokenizer_version": "6.0.0",
  "dictionary": "ipadic",
  "dictionary_version": "2026.08",
  "mode": "normal"
}
```

辞書を替えたら索引を再構築する。新旧索引を併存させ（`index_build` 分離）、上書きしない。でなければ「昨日は引けたのに今日は引けない」が起きる。

### 7.3 正規化規則（索引とクエリで共用）

| 種別 | 処理 |
|---|---|
| 全角英数 → 半角 | `ＡＢＣ１２３` → `ABC123` |
| 半角カナ → 全角カナ | `ｱｲｳ` → `アイウ` |
| 旧字体 → 新字体 | `髙`→`高`、`﨑`→`崎`（照合のみ。表示は原文） |
| 大小 | ラテンは小文字に統一 |
| 長音符 | 保持（`ー` と `-` を同一視しない） |
| 空白 | 連続空白を畳む |

正規化は**索引とクエリの照合のみ**。`chunk.text` と `evidence.quote` は原文を保つ。でなければ証拠の信頼が崩れる。

### 7.4 ライセンス

- Lindera 本体：MIT（6.0.0 で確認）。
- 辞書：`lindera-ipadic` crate は MIT だが、IPADIC 辞書データ自体の条件は公開前に確認する。
- 初期は IPADIC。**UniDic は使わない**（GPL / 商用の dual license で法務コストが高い）。リスク R-05 / T-07 に登録済み。

---

## 8. まとめ

> 語単位が下限を守り、文字単位が体験を守り、PGroonga は上限を上げるが運用の代償が大きい。
> 基盤は「語単位 + 任意の文字単位 + ルーティング」を既定とし、PGroonga は既に持つ顧客に委ねる。

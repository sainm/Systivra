# 付録 A：オープンソース評価

> 評価日 2026-11-05。
> **[確]** = 公式サイト / GitHub Release / パッケージメタデータで本回確認。**[要確認]** = 一般知識に基づく。採用前に LICENSE と Releases を必ず再確認する。
> Prompt の「旧経験で判断しない」に従い、確認済みと未確認を分ける。曖昧な表現で不確実性を隠さない。

---

## 1. 分類

| 分類 | 意味 | 採用方法 |
|---|---|---|
| A アーキテクチャ参考 | 層構成、モジュール分割、拡張点を参考にする | コードと文書を読む。依存しない |
| B データモデル参考 | Schema、実体関係、版モデルを参考にする | 自模型に翻訳。依存しない |
| C 直接組込 | ライブラリ / バイナリとして使う | 版固定、ライセンスホワイトリストに追加 |
| D 独立 Worker | 独立サービスとして動かし、プロトコルで通信 | コンテナ化。リンクしない |
| E 着想のみ | 記事・論文を読む。コードは読まない | なし |
| F 不採用 | ライセンス / 活発度 / 構成が合わない | 明示的に除外し理由を残す |

---

## 2. 評価結果

### 2.1 RAG / Graph

| プロジェクト | 言語 | ライセンス | 活発度 | 分類 | 結論 | 確認 |
|---|---|---|---|---|---|---|
| neo4j-graphrag-python | Python | Apache-2.0 | 高 | E | 「検索器の組合せ + グラフ検索 + リランク」の抽象を参考。Python ライブラリのため Rust 中核には組込まない。実体抽出を LLM に依存する点は、コード解析由来の実体と相容れない | [要確認] |
| Graphiti | Python | Apache-2.0 | 高 | B | **双時間軸モデル**（valid_at / invalid_at + created_at）が Fact の `valid_from/valid_to` と一致。同一事実の再出現・矛盾の併存の扱いを参考にする。Python + Neo4j 依存のため組込まない | [要確認] |
| Flexible GraphRAG | Python | 要確認 | 中低 | F | マルチバックエンドの試みだが、保守と文書が企業依存に足りない。多ベクトル DB 切替は Provider 抽象で解決済み | [要確認] |
| RAGFlow | Python + TS | Apache-2.0 | 非常に高 | A | **深い文書解析 + 可視化チャンク**の製品の姿を参考（原文と chunk の対照）。完全アプリのため組込むと二次ラップになる | [要確認] |
| CocoIndex | Rust + Python | Apache-2.0 | 高 | A | **宣言的増分索引**のデータフロー（source → transform → target。変更分だけ再計算）を参考。`parse_run → chunk → embed` の増分設計に反映 | [要確認] |
| OpenSPG / KAG | Java + Python | Apache-2.0 | 高 | B | **Schema 制約下の知識構築**と論理形式推論。Schema がインスタンスに先行しないと、グラフは型のない辺の集まりに退化する | [要確認] |

### 2.2 コード解析

| プロジェクト | 言語 | ライセンス | 活発度 | 分類 | 結論 | 確認 |
|---|---|---|---|---|---|---|
| Tree-sitter | C / Rust | MIT | 非常に高 | **C** | 構文層の主力。公式 grammar に **XML はない**。JSP の grammar もない（F-04） | [確] 0.25.10 |
| jQAssistant | Java | GPL-3.0 | 中低 | F | コード構造を Neo4j に入れて Cypher で問う発想は同じ。GPL-3.0 は配布形に不適。活発度も不足 | [要確認] |
| Spoon | Java | 要確認（Apache-2.0 系 / GPL 系） | 中 | E | メタプログラミングは強力だが、完全な classpath と大きなメモリを要する。ADR-017 の最終方式では使わない | [要確認] |
| JavaParser | Java | Apache-2.0 | 高 | E | 軽量な構文解析。ADR-017 の最終方式では Tree-sitter とプロジェクト内シンボル索引を使う | [要確認] |

### 2.3 追跡 / テスト

| プロジェクト | 言語 | ライセンス | 活発度 | 分類 | 結論 | 確認 |
|---|---|---|---|---|---|---|
| Eclipse Capra | Java | EPL-2.0 | 低（実質アーカイブ） | E | Trace Link を一級市民とし、型と信頼度を持たせるモデルを参考。Eclipse RCP の技術スタックのまま更新停止。依存しない | [要確認] |
| StrictDoc | Python | Apache-2.0 | 高 | B | **要件-仕様-テストの双方向追跡**と「文書をデータとして扱う」モデル（REQ/SPEC + 関係 + 安定 UID）を参考 | [要確認] |
| Kiwi TCMS | Python | GPL-2.0（要確認） | 高 | F | 「ケースの版管理 + 実行記録の不変性」のみ参考。GPL + Django モノリス。中核が「登録」であり「派生」の我々と逆 | [要確認] |

### 2.4 Workflow / UI / 基盤

| プロジェクト | 言語 | ライセンス | 活発度 | 分類 | 結論 | 確認 |
|---|---|---|---|---|---|---|
| Dify | TS + Python | Apache-2.0 + 追加条件 | 非常に高 | A | **DSL 設計**（ノード型、`${node.output}` の変数参照、版管理）とキャンバス操作を参考。完全アプリのため組込まない | [要確認] |
| React Flow | TypeScript | MIT | 高 | **C** | Workflow エディタ | [要確認] |
| Cytoscape.js | JavaScript | MIT | 高 | **C** | 関係グラフ | [要確認] |
| Docling | Python | MIT | 非常に高 | **D** | `docling-serve` コンテナで PDF/DOCX/PPTX を担当。**XLSX には使わない**（ADR-011） | [確] 2.142.1 / serve 1.38.1 |
| pgvector | C | PostgreSQL License | 高 | **C** | HNSW の本番下限は 0.8.4 | [確] 0.8.6 |
| pg_bigm | C | PostgreSQL License | 中 | C（任意） | 日本語の部分一致。顧客環境が拡張を許容する場合のみ | [要確認] |
| PGroonga | C | PostgreSQL License | 中 | F（初期） | 性能は良いが索引が大きく、crash-safe でない。初期は導入しない。既存利用時のみ接続 | [要確認] |
| GitHub Copilot SDK | TS / Python / Go / .NET / Rust / Java | 要確認（SDK と CLI で異なる） | 非常に高（2026-06 GA） | C（任意） | Rust SDK あり。BYOK は OpenAI 互換。ただし CLI サイドカー前提で閉域に不向き。任意 Provider として保持 | [確] GA 2026-06-02 |

---

## 3. 採用結論

### 直接組込（C）

```text
tree-sitter 0.27.x + 各 grammar（版固定）
pgvector 0.8.6
Cytoscape.js
React Flow
（任意）pg_bigm
（任意）GitHub Copilot SDK
```

### 独立 Worker（D）

```text
docling-serve 1.38.1      PDF / DOCX / PPTX
Playwright 1.65.0         テスト実行
```

### 参考（A / B / E）

```text
CocoIndex        増分索引のデータフロー
Graphiti         双時間軸の事実モデル
StrictDoc        要件-テスト追跡モデル
RAGFlow          原文と chunk の対照 UI
Dify             Workflow DSL とキャンバス
OpenSPG / KAG    Schema 先行の知識構築
Capra            Trace Link を一級市民に
jQAssistant      コード構造をグラフに入れて問う発想
```

### 不採用（F）

```text
RAGFlow           中核として（二次ラップになる）
Dify              Workflow エンジンとして（完全アプリ。組込不可）
jQAssistant       GPL-3.0 + 活発度不足
Kiwi TCMS         GPL + 立ち位置が逆（登録 vs 派生）
PGroonga          初期の運用負担が過大
Flexible GraphRAG 保守と文書が不足
```

---

## 4. 新規依存のチェックリスト

```text
□ ライセンス：ホワイトリスト（Apache-2.0 / MIT / BSD / PostgreSQL License / EPL-2.0）か
              GPL/AGPL は独立プロセスのみか（リンク・派生配布をしないか）
□ 活発度：直近 6 か月の release、issue 応答、保守者数、企業の後ろ盾
□ 版：現行安定版、semver の有無、メジャー更新の頻度
□ 配置：追加ミドルウェアの要否、資源、オフライン可否
□ 互換：言語 / ランタイム版、既存スタックとの衝突
□ 代替コスト：2 年後に捨てる場合、移行にどれだけかかるか
□ 安全：既知 CVE、任意コード実行、ネットワークアクセス
□ データ：配置境界の外へデータを出さないか
```

判定：**1 項目でも「不可」なら不採用。3 項目以上が「不明」なら導入を延期する**。

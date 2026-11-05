# Systivra

Software Engineering Intelligence Platform — 設計書・ソースコード・データベース・テストを「証拠」でつなぎ、改修の影響範囲を計算・検証・監査できるようにする分析基盤。

## 設計文書

- 正式版（日本語）: [`docs/design/ja/00-全体設計.md`](docs/design/ja/00-全体設計.md)
- 中国語版（内部参照・**凍結**）: [`docs/design/00-README.md`](docs/design/00-README.md)
- 一致性監査: [`docs/design/review-2026-11-05.md`](docs/design/review-2026-11-05.md)

## 状態

Phase 0（設計）完了。Phase 1（Platform Foundation + Walking Skeleton）実装前。

## 構成

```text
docs/design/ja/            設計正本（日本語）
  00〜09                  全体設計 9 部
  reference/              付録 3 本（日本語検索 / OSS 評価 / 規模算定）
  detail/                 実装直結の詳細設計（01 Excel 取込 / 02 DDL / 03 性能 / 04 API 契約 / 05〜 …）
docs/design/               中国語版（内部参照・凍結）+ 一致性監査 review-2026-11-05.md
```

> **日本語版が唯一の正本**。2026-11-05 以降の設計変更は `ja/` にのみ反映する。中国語版は当時の意思決定の記録として凍結し、数字・版・契約の照会には使わない。

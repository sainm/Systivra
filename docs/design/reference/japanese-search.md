# 附录 C：日语全文检索方案对比与决策

> 核查日期 2026-11-05。结论已写入 ADR-007 与 `00-README.md` D-02。
> 本文回答一个问题：**为什么不是「三选一」，而是「分层 + 查询路由」**。

---

## 1. 问题的根源

PostgreSQL 内置全文检索依赖「分词器 + 词典」：它按空格与标点切词，再归一化为 lexeme。日语没有词间空格，`to_tsvector('simple', '住所チェックの桁数')` 会把整串当成一个 token，检索基本失效。

绕过这个问题有三条路，彼此的取舍完全不同：

| 方案 | 本质 | 一句话 |
|---|---|---|
| 预分词 + tsvector | **词**级倒排 | 自己把日语切成词，再交给 PG 的标准全文索引 |
| pg_bigm | **字**级倒排（2-gram） | 把文本切成所有相邻两字组合，用 GIN 加速 `LIKE '%…%'` |
| PGroonga | **独立搜索引擎**做索引 | 用 Groonga 的 tokenizer + normalizer 替代 PG 的文本搜索栈 |

三者不是同一条技术路线上的快慢之分，而是「以词为单位」与「以字为单位」的范式差异。

---

## 2. 三种方案的机制

### 2.1 预分词 + tsvector

```text
原文: 住所は全角20文字以内です
  ↓ Lindera (IPADIC) 形态素解析
[住所][は][全角][20][文字][以内][です]
  ↓ 空格连接，写入 tokenized 列
"住所 は 全角 20 文字 以内 です"
  ↓ to_tsvector('simple', tokenized)
'20':4 'です':7 'は':2 '以内':6 '全角':3 '住所':1 '文字':5
```

- 查询侧必须用**同一分词器、同一词典版本**，否则 query 的 token 与索引的 token 对不上。
- 支持词项布尔查询（`&`）、短语（`<->`）、前缀（`:*`）。
- 不支持「部分一致」：搜 `住所チェ` 不会命中 `住所チェック`，因为索引里没有 `住所チェ` 这个 token。
- 未登录词（产品名、型番、外字、旧字体）会被切碎或切错，是主要失分点。

### 2.2 pg_bigm

```text
原文: 住所チェック
  ↓ 取所有相邻 2 字
[住所][所チ][チェ][ェッ][ック]
  ↓ GIN 索引
查询 LIKE '%住所チェ%'
  → 用 bigram 交集召回候选
  → recheck：回表逐条确认是否真的包含该子串（去掉假阳性）
```

- 不需要词典，对未登录词、外字、型番天然友好。
- **必须 recheck**：bigram 交集只保证「包含这些二字组」，不保证「连续」。命中很多时 recheck 会成为瓶颈。
- 单字查询（如 `住所` 只有一个二字组也还行，但单字 `姓`）退化为全表扫描。
- 排序能力弱：`LIKE` 没有相关性分数，需要另外用 `ts_rank` 或应用层排序。
- GIN 是 crash-safe 的，异常宕机不需要重建索引。

### 2.3 PGroonga

```text
原文 → Groonga tokenizer（可配置 N-gram 或 TokenMecab）
     → normalizer（全角/半角、大小写、旧字体归一）
     → Groonga 倒排索引（PG 的 index access method）
查询 &@~ '住所 チェック'
     → 索引内精确匹配，无需 recheck
```

- 日英中混排、归一化（`髙`→`高`）能力强，适合日本企业文档。
- 不需要 recheck，命中很多时仍快。
- 索引更大（官方基准约为 pg_bigm 的 2.3 倍）。
- **不是 crash-safe**：PostgreSQL 异常崩溃后索引可能损坏，需要 `REINDEX`。对无人值守的客户内网部署是实打实的运维负担。
- 需要安装 Groonga 库与扩展，客户 DBA 未必批准。

---

## 3. 对比表

| 维度 | 预分词 + tsvector | pg_bigm | PGroonga |
|---|---|---|---|
| 索引单位 | 词（lexeme） | 2-gram | 词或 N-gram（可配） |
| 日语分词质量 | 依赖 Lindera 词典 | 不依赖 | 依赖 Groonga tokenizer |
| 部分一致（`住所チェ`） | 不支持 | 支持 | 支持 |
| 未登录词 / 外字 | 较弱（易切碎） | 强 | 强（可归一化） |
| 假阳性处理 | 无（词项精确） | recheck（回表） | 无（索引内精确） |
| 相关性排序 | `ts_rank_cd`，成熟 | 弱（需另接排序） | 内置 score，较强 |
| 索引大小（相对） | 中 | 小（基准 1.0） | 大（约 2.3×） |
| 写入成本 | 中 | 中高（gram 数量多） | 中高 |
| 多命中查询延迟 | 中 | 高（recheck，官方旧基准约 50×） | 低 |
| 崩溃安全 | 是（PG 原生） | 是（GIN） | **否**（需 REINDEX） |
| 扩展依赖 | **无** | pg_bigm | PGroonga + Groonga |
| 托管 PG 可用性 | 全部 | 部分 | 部分（Supabase 等） |
| 许可 | PostgreSQL License | PostgreSQL License | PostgreSQL License |
| 运维负担 | 低 | 中 | 高 |
| 适合 | 词级检索、可移植 | 部分一致、型番/外字 | 高并发、多语言、已有 Groonga |

> 性能数字说明：官方基准（PGroonga vs pg_bigm）来自 PostgreSQL 9.6 时代，只能作为量级参考，不能作为选型依据。Phase 2 必须用真实设计书自测，见 §6。

---

## 4. 用真实查询看差异

以下判断基于机制推导，Phase 2 会用金标集验证。

| 查询 | 预分词 + tsvector | pg_bigm | PGroonga | 结论 |
|---|---|---|---|---|
| `住所チェック`（完整词） | 命中，词项 AND | 命中，需 recheck | 命中 | 三者皆可 |
| `住所チェ`（部分输入） | **不命中** | 命中 | 命中 | 输入过程中的即时搜索，tsvector 不可用 |
| `髙橋`（旧字体） | 可能切碎或未登录 | 命中（按字面） | 命中并按 normalizer 归一 | 外字场景 pg_bigm/PGroonga 更稳 |
| `SALE_CD`（标识符） | 被切成 `SALE` `_` `CD` | 命中子串 | 命中 | 代码/型番检索偏 bigram |
| `12,345`（带符号数字） | 分词不稳定 | 命中 | 命中 | 同上 |
| `住所 AND 必須`（布尔） | 原生支持 | 需应用层组合 | 支持 | 结构化检索偏 tsvector |

关键结论：**工程现场两种查询各占一半**。设计书里既有自然语言句子（适合词级），也有项目名、型番、半角カナ、代码标识符（适合字级）。只选一种，另一类一定难用。

---

## 5. 决策：分层 + 查询路由

### 5.1 三层能力

```text
L1 tokenized   Lindera IPADIC → tokenized 列 → tsvector(simple) + GIN
               始终启用，零扩展依赖，是「平台能跑」的底线

L2 bigram      pg_bigm GIN on chunk.text
               检测到扩展可用时自动启用；不可用则跳过，不影响 L1

L3 groonga     PGroonga
               仅当客户环境已部署时复用；不作为平台要求
```

对应 `FulltextCapabilities`：

```rust
pub struct FulltextCapabilities {
    pub tokenized: bool,   // 恒为 true
    pub bigram: bool,      // 运行时探测
    pub groonga: bool,     // 运行时探测
    pub phrase: bool,      // L1 支持 <->；L2 不支持
    pub substring: bool,   // 仅 L2 / L3
}
```

能力矩阵里显示为 `supported / unsupported`，不做 `unknown` 的隐式降级。

### 5.2 查询路由

```text
analyze(query):
  normalized = normalize(query)        // 全角→半角、旧字体→新字体、大小写
  tokens     = lindera.tokenize(normalized)

  if looks_like_identifier(normalized):     // 含 _ . / : 数字字母混排、型番
      route = BIGRAM
  elif is_quoted(normalized):               // "..." 精确短语
      route = BIGRAM
  elif len(normalized) <= 4 and not has_space(normalized):
      route = BIGRAM                        // 短输入，部分一致优先
  else:
      route = TOKENIZED

  if bigram_available and route == TOKENIZED:
      parallel_both = true                  // 两路都跑，RRF 融合
```

融合仍然用 RRF（`k=60`），与向量检索三路合并：

```text
score = 1/(60+rank_vec) + 1/(60+rank_fts) + 1/(60+rank_bigram)
```

`search_response.lexical_mode` 返回实际使用的路径（`tokenized` / `bigram` / `both`），UI 上可查。这样调优时不用猜。

### 5.3 为什么不默认 PGroonga

三条，按权重排序：

1. **崩溃后需 REINDEX**。客户内网可能断电、可能被运维 kill -9。一次崩溃导致检索不可用，需要人工介入重建索引，这在「保守运用」场景是不可接受的。
2. **扩展审批**。日本客户 DBA 对第三方扩展的态度普遍保守；pg_bigm 已经是需要解释的东西，PGroonga 还要带 Groonga 库。
3. **索引体积**。设计书语料本来就大（几万页 + 上千 sheet），2.3 倍索引会直接推到磁盘预算的红线。

如果客户已经在用 PGroonga（例如既有系统），我们复用，不反对；但平台不把它作为默认依赖。

### 5.4 为什么不只做 tsvector

因为它无法覆盖「输入过程中的部分一致」。工程师在搜索框里打「住所チェ」时，期望看到「住所チェック」。这是搜索体验的基本预期，不是高级功能。仅靠向量检索也不稳——型番和外字没有语义近邻。

---

## 6. Phase 2 基准测试计划

选型不靠感觉，用数据确认。

### 6.1 语料

- 6 份真实结构的设计书（PDF 200 页 + XLSX 12 sheet ×2）
- 约 8,000 个 chunk
- 含：自然语言、表格键值、型番、半角カナ、旧字体、英文标识符

### 6.2 查询集（60 条，人工标注）

| 类别 | 数量 | 例 | 期望 |
|---|---|---|---|
| 词级 | 20 | 「住所 桁数 チェック」 | L1 应命中 |
| 部分一致 | 20 | 「住所チェ」「ユーザ編」 | L2 应命中 |
| 标识符/外字 | 20 | 「SALE_CD」「髙橋」「−（全角マイナス）」 | L2 应命中 |

### 6.3 指标

| 指标 | 目标 | 说明 |
|---|---|---|
| recall@10 | ≥ 0.85 | 分三类分别统计，不取平均掩盖短板 |
| MRR | ≥ 0.60 | 第一条命中的平均倒数排名 |
| P95 延迟 | < 300ms | 单路；融合后 < 600ms |
| 索引大小 | < 语料的 15% | 含 tsvector + bigram |
| 写入放大 | < 2.5× | 相对纯文本插入 |
| 构建时间 | < 30 min | 8,000 chunk |

### 6.4 判定规则

```text
若 L1 词级 recall < 0.75        → 检查分词词典，必要时换 UniDic（需法务确认）
若 L2 部分一致 recall < 0.80    → 评估 pg_bigm 参数（gin_key_limit）或引入 trigram
若两类查询差异 > 25 个百分点    → 确认「路由 + 双路融合」为必选，而非可选
若 P95 > 800ms                  → 降 top_k、关闭 rerank、评估 PGroonga
```

结果写入 `docs/operation/search-benchmark.md`，作为后续调优基线。

---

## 7. 实施要点

### 7.1 索引与列

```sql
-- L1：始终存在
ALTER TABLE kb.chunk ADD COLUMN tokenized text NOT NULL DEFAULT '';
CREATE INDEX chunk_fts_idx ON kb.chunk USING gin (fts) WHERE is_current;

-- L2：扩展可用时才创建（迁移脚本里做条件判断）
CREATE INDEX chunk_bigm_idx ON kb.chunk USING gin (text gin_bigm_ops)
  WHERE is_current;
```

`text` 用原文（不做归一化），`tokenized` 用归一化后的词串。两者职责不同，不要合并。

### 7.2 分词器版本要写进索引元数据

```json
{
  "tokenizer": "lindera",
  "tokenizer_version": "6.0.0",
  "dictionary": "ipadic",
  "dictionary_version": "2026.08",
  "mode": "normal"
}
```

换词典必须重建索引，且新旧索引并存（`index_build` 隔离），不能原地覆盖。否则会出现「同一查询，昨天能搜到今天搜不到」。

### 7.3 归一化规则（查询与索引共用同一实现）

| 类型 | 处理 |
|---|---|
| 全角英数 → 半角 | `ＡＢＣ１２３` → `ABC123` |
| 半角カナ → 全角カナ | `ｱｲｳ` → `アイウ` |
| 旧字体 → 新字体 | `髙`→`高`、`﨑`→`崎`（仅用于匹配，展示保留原文） |
| 大小写 | 统一小写（拉丁） |
| 长音符 | 保留（不统一 `ー` 与 `-`） |
| 空白 | 连续空白折叠 |

归一化只用于**索引与查询匹配**，`chunk.text` 与 `evidence.quote` 必须保留原文，否则证据不可信。

### 7.4 许可

- Lindera 本体：MIT（已核实 6.0.0）。
- 词典：`lindera-ipadic` crate 为 MIT，但 IPADIC 词典数据本身的条款需在发布前确认。
- 首期选 IPADIC，**不选 UniDic**（UniDic 为 GPL / 商用双许可，法务成本高）。此条已列入风险 R-05 / T-07。

---

## 8. 一句话总结

> 词级检索保下限，字级检索保体验，PGroonga 保上限但代价是运维。
> 平台默认「词级 + 可选字级 + 查询路由」，把 PGroonga 留给已经拥有它的客户。

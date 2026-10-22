# Issue #16：候选首位回归与完整词保护的证据边界

2026-10-22；基线 `glory_dev` `77fb8b8`（含 #15 / Issue #13）。施工分支 `fix/glory-dev-candidate-top1-ranking-consistency`。

## 结论先说

实机截图的两条（`de'yuan'sheng` → `德元升`、`di'yi'ge` → `第一个`）在**干净学习档案**下都复现不了：
`glory_dev` 的完整词保护本身是有效的。它们需要一份**有个人学习数据**的档案才会出现，而且不是
「词库缺词」「安装包旧」或「整句插入顺序」的问题，而是：

> `should_prefer_complete_word` 用 `Conversion::personal_bonus > 1e-6` 作为放行条件，而
> `personal_bonus = personal_delta + selection_bonus` 是**路径上逐词累加**的。用户常选 `的`、`一个`、
> `得`、`原生` 这些碎片，就足以让 `的一个` / `的原生` 的整句路径绕过保护、插到完整词前面。
> 单个词被选过，并不等于用户这次想要这个组合。

Issue 4.1 的假设成立，4.2 / 4.3 的怀疑方向可以排除（见下）。

## 复现

Linux x86_64、产品数据 `data-v2`（`tools/release/data-fetch.sh`，`dict.qj` + `lm.qj` + `model.qjm`）、
`fork_words.tsv` 作为附加词库。`dict.qj` 含 `第一个 di yi ge 19032` 与 `的 di 21162`；
`德元升 de yuan sheng 3000` 由 `extra_dictionaries` 随程序编入。

学习档案用 CLI 的 `--user-dict`（词频 `词\t次数`）与相邻的 `user-choices.tsv`（`输入串\t词\t次数`）：

| 实验 | 档案内容 | `deyuansheng` 首位 | `diyige` 首位 |
| --- | --- | --- | --- |
| 干净 | 空 | 德元升 ✓ | 第一个 ✓ |
| A | 仅子词词频：的 500、一个 300 | **的原声 [句]** | **的一个 [句]** |
| B | 仅子词词频：得 500、原生 200 | **的原生 [句]** | 第一个 ✓ |
| C | 仅整串选择：`diyige → 的一个` 2 次 | 德元升 ✓ | 第一个 ✓ |
| D | 仅「一个」300 | 德元升 ✓ | **的一个 [句]** |

A/B/D 是回归，C 说明「明确选过整个结果」的记录本身没有害处——它缺的是被当成证据。

## 根因链

1. 用户常选 `的` / `一个` 等碎片 → `Learner::weight` 有值。
2. `sentence::viterbi` 的 `selection_bonus = weight_bonus(weight(word))` 逐词累加；
   个人 n-gram 的 `personal_delta` 同样累加。两者合起来是 `Conversion::personal_bonus`。
3. `should_prefer_complete_word` 看到 `personal_bonus > 1e-6` 就返回 `false`（不保护），
   整句 `的一个` / `的原生` 被插到首位。
4. `CandidateKind::Sentence` 上屏原本**不记** `record_choice`（只记路径词转移），所以
   「用户明确选过整句」这件事在查询侧从来没有结构化证据，放行只能依赖粗粒度的 `personal_bonus`。

不是 4.3 的版本问题：产品 `dict.qj` 里两个词都在；也不是 4.2 的词级/整句排序重复计分：
词级首位在实验 A/B/D 里本来就是正确的完整词，问题只在整句插入的放行条件。

## 修复

- `engine/query/mod.rs::should_prefer_complete_word`：放行条件改为
  `self.learner.choice_weight(typed, &conversion.text) > 0`，即**本输入串下明确选过这个完整结果**
  （整句候选上屏、整段分几次选完的合成词）。`personal_bonus` 不再参与放行。
- `engine/commit/mod.rs`：`CandidateKind::Sentence` 上屏时补记
  `record_choice(&input, &candidate.text)`，让「明确选过整句」从下一次查询起可被识别；
  整句仍不记词频（它不是一个词）。
- 不放宽/收紧任何分数与阈值；不改 `BEAM_WIDTH`、`RESCORE_PATHS`、`COMPOSED_*`。

## 回归测试（`engine/tests/joint/top1_ranking.rs`，6 项）

受控词库 + 受控 bigram，复现「完整词存在、整句更像人话」的竞争：

- `clean_profile_prefers_the_complete_word`：空档案下两个完整词都第一。
- `child_word_frequency_does_not_lift_a_split_sentence_over_a_complete_word`：子词词频
  （的/一个/得/原生）不再抢位；显式单引号与连续全拼一致。
- `explicit_choice_of_the_whole_result_still_lifts_it`：明确选过 `的一个` 时它仍能回到第一，
  且选择只对记录过的输入串生效。
- `choosing_the_pieces_one_by_one_teaches_the_whole_result`：端到端分次选 `的` + `一个`，
  `finish_buffer` 记下整段选择后，重查时 `的一个` 回到第一。
- `committing_a_sentence_records_the_explicit_choice`：整句上屏会留下选择记录。
- `repeated_queries_on_one_snapshot_are_deterministic`：同一快照连查 5 次结果一致。

既有用例的语义同步：`lookup::learned_combination_can_still_beat_a_complete_word` 与
`joint::composed::personal_evidence_admits_a_composition_without_dictionary_support` 原本用
纯个人 n-gram 越过完整词；按本 Issue 的证据边界改为「n-gram 负责抬路径分 + 明确选择负责放行」，
学习效果仍然保留。

## 修复前后对比

83 句冻结语料（`docs/notes/issue-8-eval.tsv`）、产品 `dict.qj` + `lm.qj`、冷启动不学习：

| 指标 | 基线 `77fb8b8` | 本分支 |
| --- | ---: | ---: |
| 首选准确率 | 31.3% | 31.3% |
| 整句候选命中 | 31.3% | 31.3% |
| 字准确率 | 74.3% | 74.3% |
| 查询平均 / 最慢 | 2.3 / 11.5 ms | 2.4 / 11.9 ms |

受控实验（产品数据 + 上述学习档案）：A/B/D 从「整句抢位」回到完整词第一；C（明确选过整串）
保持 `的一个` 第一。

## 风险与已知边界

- **纯个人 n-gram 不再能越过完整词。** 这是有意收窄：跨输入框连续选 `和`、`并` 产生的转移
  不再等于「这次输入要 `和并`」；用户整句上屏或一段拼音分次选完会留下选择记录，仍然有效。
- **整句上屏现在会写 `user-choices.tsv`**（每段一次）。它只增加一条文本→次数，不影响词级排序
  （整句文本不在词级候选里），数据量上限沿用 `MAX_CHOICE_ENTRIES`。
- 显式单引号与无分隔输入共用同一选择键（`choice_key` 去 `'`），行为一致。
- 模糊音 / 敲错读出的整句仍走 `conversion.altered()` 分支（保持原行为），本 Issue 未改这条路径；
  实机上 `di'yi'ge → 的一个` 在原样读音里就能解释（`的 di` 21162），不需要它。

## 未执行

- macOS 实机验收（安装 bundle 版本、数据版本、TextEdit / 浏览器连续输入 A/B 多轮）——
  本环境是 Linux，**未执行**，需要在 macOS 上按上面的复现命令做一次。
- 产品 LM 下 `teng'hu → 藤壶` 的语义排序（属 #14 / Issue #13 的已知限制）不在本 Issue 范围。

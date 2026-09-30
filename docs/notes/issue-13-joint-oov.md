# Issue #13：动态切分预算与未登录组合候选

2026-09-30；基线 `origin/glory_dev` `9f1e817`。施工分支 `refactor/glory-dev-dynamic-joint-oov`。

## Preflight（改代码之前的真实状态）

| 任务 | 状态 | 代码证据 |
| --- | --- | --- |
| Task A | `NOT_FIXED` | `engine/query/joint.rs` 仍有 `const JOINT_SEGMENTATIONS: usize = 2;`，`best_joint_sentence` 先按 parser 顺序取有资格的切分，只把「有整段完整词证据」的那条换到第二位，然后 `.take(JOINT_SEGMENTATIONS)`。第三条及以后永远不做整句路径搜索。 |
| Task B | `NOT_FIXED` | 候选表里唯一的整句来源是 `plain_sentence_joint` 返回的**一条** `Conversion`；其余路径在 `select_joint_paths` 之后被丢掉，没有任何把「第二条及以后的合理组合路径」放进候选表的通道。产品词库跑 `ye'lang` 得到 `夜郎 / 夜郎自大 / 也 / 耶 / …`，没有 `野狼`。 |

## Task A：按证据分配切分预算

### 算法

`best_joint_sentence` 分两步：

1. **探针**：每条有资格的切分跑一次 `convert_path_groups(k = 1)`（单前驱、不建路线串），拿到它自己的最优完整路径分。探针用的词格进同一张 `SpanCache`，被选中的切分随后做宽搜索时不重复查词库；没有神经重排时 `k` 本来就是 1，探针结果**直接当最终结果复用**，不再跑第二遍。
2. **按证据分配展开名额**（`plan_joint_segmentations`）：探针分最高的切分先入；parser 首选与「音节数不同于最优探针的最佳切分」作为保底结构代表各占一名；其余按探针分从高到低填，落后最优探针超过 `JOINT_PROBE_SLACK` = 6 nat 或到 `JOINT_MAX_SEGMENTATIONS` = 4 就停。

决定生死的是「这条切分自己的词格 + 静态/个人模型能给出多好的完整路径」，不是它在 parser 输出里的名次。把 `2` 改成 `4` 只是结果之一，换掉的是**选择依据**：旧实现里第 3 条即使词格证据明显更强也没有入口，新实现里它按探针分第一个被展开。

### 有界性

- 切分条数上限 `JOINT_MAX_SEGMENTATIONS` = 4（硬上限）。
- 探针总量上限 `JOINT_PROBE_SPANS` = 128 个词格：四条切分的四音节输入（每条十来个格子）全部覆盖得到；九音节以上的长句收敛到两条左右。词格本来就要为真正展开的切分算，探针只是让它们提前算，代价是「多探的那几条」。
- 没有扩大 `BEAM_WIDTH`（仍 8）、没有扩大 `RESCORE_PATHS`（仍 6）、没有同步等待神经模型。

### 回归

`engine/tests/joint/dynamic_budget.rs`：

- `parser_keeps_three_complete_segmentations_with_the_target_last`：`anxianan` 的前三条完整切分依次是 `an xian an`、`an xia nan`、`an xi an an`。
- `the_target_text_is_unreachable_on_the_first_two_segmentations`：直接在前两条切分的词格上跑 `convert_paths`，都造不出 `安溪安安`——旧实现不搜索第三条就永远拿不到它。
- `third_segmentation_wins_the_joint_search`：新实现首选 `安溪安安`，`Query.segmentations[0]` 与 `marked_text()` 都是 `an'xi'an'an`。
- 另有神经开 / 关、模型反对时仍进重排池、显式 `'` 是硬边界、个人 n-gram 把落后切分抬进预算、无个人证据时静态模型仍然说了算、双拼只解出一条切分等用例。

## Task B：未登录组合候选

### 为什么不是「从整句 Top-K 里捞」

诊断（`path_trace`，产品词库）显示 `ye'lang` 只有一条完整切分 `ye lang`，束宽 8 全部占满时终端路径是：

```text
夜郎(-18.999) 也浪(-22.550) 也郎(-23.174) 也朗(-23.332) 也狼(-23.666)
也廊(-25.223) 耶浪(-26.181) 夜浪(-26.350)      ← 野狼 约 -27.7，第 8 名以外
```

按分数排序的路径池结构性地捞不到 `野狼`：`也` 的单字词频比 `野` 高 4.04 nat，`也*` 的组合把八个名额占满了。所以组合候选走**自己那条词格通道**（`engine/query/composed.rs`），不依赖整句路径池多留几条。

### 生成与筛选

对每条做过多路径搜索的切分（≤ `JOINT_MAX_SEGMENTATIONS`）：

1. 只收**音节点全部完整**的切分（末尾还差字母时不造组合）。
2. 在每个切分点把词格切成头 / 尾两段，各取 `span_candidates` 的前几个词（与 Viterbi 同一张缓存）。
3. 两段的交界字（头的末字、尾的首字）必须在**某个词条里同词出现过**——`野`+`狼` 有 `狼子野心` 作证，`也`+`狼`、`野`+`浪`、`夜`+`狼` 都没有。判据完全来自词库，换任何一对字都按同一条规则算。
4. 没有共现证据时，只有个人 n-gram 证据（`Conversion::personal_bonus > 0`）能留下组合。
5. 硬约束：完整覆盖、无占位、两个部分都是词库词、代价为 0（模糊音 / 敲错命中的不要）、与已有候选文本去重、路径分不落后最优路径 `COMPOSED_MARGIN` = 10 nat。
6. 排序键是（这个输入串下用户选过的次数, 路径分），与词级 `ranking::rank` 同一个键序；截断到 `COMPOSED_CANDIDATES` = 3。
7. 插入位置：开头英文候选 / 整句首选 / **覆盖整段输入的完整词**之后。整句胜出路径真的变成候选时，同文本的组合候选去掉；没变成候选（被完整词挡下来）时留着。

`COMPOSED_MARGIN` 的依据：整句路径分是 log 概率之和，一个完整词与「它拆成两个字」的差距 = 多摊的一次 `FALLBACK_PENALTY` 量级再加两段词频之比。在产品词库（总词频 2.45e8）上，已经有一个常用完整词的输入，组合路径的差距都在 10.5 nat 以上（`nihao/呢好` -11.49、`dangao/蛋高` -10.87、`mingtian/名天` -10.55）；而整词罕见、组合才是用户可能想要的，都在 9 nat 以内（`ye'lang/野狼` -8.60、`jing'shui/井水` -7.69）。取 10 就是这条分界，它随词典总词频自然缩放。

### 验收与控制案例

产品词库（`assets/lexicon/dict.tsv`），没有往任何词库文件里加测试词：

| 输入 | 结果 | 词库里的支撑词 | 完整词候选 |
| --- | --- | --- | --- |
| `ye'lang` | **`野狼` 第 2** | `狼子野心` | `夜郎` 仍是第 1 |
| `hu'mao` | **`虎猫` 第 2** | `照猫画虎` | — |
| `jing'shui` | **`井水` 第 2** | `水井` | `净水` 仍是第 1 |
| `sha'mo'hua` | **`沙漠化` 第 2** | `荒漠化`、`文化沙漠` | — |
| `yun'bao` | 无 `云豹` | 无 | 反例 |
| `zhi'bei` | 无 `纸杯`（但有 `至北`/`指北`） | — | 反例 |
| `zhu'qiao` | 无 `竹桥` | 无 | 反例 |
| `zhi'san` | 无 `纸伞` | 无 | 反例 |
| `shi'jie` | 无 `石阶` | 无 | 反例 |

`ye'lang` 下的同音堆砌（`也狼`、`野浪`、`夜狼`、`叶郎`、`野郎`、`也浪`）一条都不出：共现证据为零。这些都由 `engine/tests/joint/composed.rs` 断言。

### 已知限制（不遮掩）

共现证据回答「这两个字在词库里有没有关系」，回答不了「这次输入是不是就要这个组合」。仍有少数输入会多出 1–3 条似是而非的组合，插在完整词后面：

| 输入 | 多出的组合 | 支撑词 |
| --- | --- | --- |
| `zuo'zhen` | `做真` / `做针` / `坐针` | `假戏真做` / `做针线` / `如坐针毡` |
| `zhi'bei` | `至北` / `指北` | `北至` / `指北针` |
| `zhi'san` | `只三` | `三只手` |
| `fangan` | `放安` | `安放` |
| `shi'jie` | `是皆`（第 11 位，第二页） | `比比皆是` |

这些输入的共同点是「整段拼音本来也没有高频完整词读法」（`坐镇`、`治三`、`制备` 都不是高频词），所以 `COMPOSED_MARGIN` 挡不住它们。共现是词形层面的弱信号，换成语义模型（真正的语料 LM 或神经分）才可能进一步收敛；当前靠**限量 3 条 + 排在完整词之后**把影响限制住。33 条常用输入的对照里 10 条候选表有变化，其中 4 条是期望的正例，`nihao` / `dangao` / `zhongguo` / `beijing` / `mingtian` / `meiguanxi` / `wanshang` / `zaoshang` 这些高频完整词的输入**没有**变化。

## 用户学习

组合候选是 `CandidateKind::Chinese`，带全部音节：

- **第一次可选**：`ye'lang` 下 `野狼` 在第 2 位，直接点选即可。
- **选择证据**：上屏走 `commit` 的中文候选分支，记 `record`（词频）与 `record_choice`（输入串下的选择次数）；下次查询 `choice_weight` 让它在同批组合候选里排到最前。
- **稳定用户词**：自动造词仍走原有规则——同一段拼音里连着选出 `野` + `狼`，第二次就 `learn_word` 出 `野狼`，之后整段拼音直接命中完整词（`auto_word_rule_still_turns_the_composition_into_a_user_word`）。
- **没选过就不学**：只查询不点选，用户词表与选择次数都不动（`unselected_compositions_are_not_learned`）。
- **神经分不是个人证据**：模型给组合候选打高分不产生词频 / 选择次数 / 用户词（`neural_score_is_not_personal_evidence`）。

不是 `Sentence` 的原因：`commit::sentence_words` 会用 **parser 首选切分**重算路径，组合候选来自另一条切分时重算不出来、直接返回 `None`，整条学习链（`finish_buffer` / `auto_word`）就断了。

## 性能

Linux x86_64、release、`assets/lexicon/dict.tsv`（92,825 条）、无语言模型（`data/generated/lm.qj` 在本环境不存在，两版同样退化为一元词频）。命令：

```bash
cargo run -p qingjian-cli --release --example joint_bench -- --dict assets/lexicon/dict.tsv --repetitions 20
```

| 指标 | 基线 `9f1e817` | 本分支 |
| --- | ---: | ---: |
| 整串稳态 · 中位数 | 342.6 µs | 350.1 µs（+2%） |
| 整串稳态 · P95 | 877.8 µs | 1292.9 µs（+47%） |
| 整串稳态 · 最大 | 905.9 µs | 1299.2 µs |
| 逐键冷缓存 · 中位数 / P95 | 299.2 / 1422 µs | 325.1 / 1473 µs（+9% / +4%） |
| 逐键热缓存 · 中位数 / P95 | 299.2 / 1421 µs | 316.2 / 1478 µs（+6% / +4%） |
| 首次查询（新 Engine 第一次按键，含建共现索引） | — | 中位数 5.2 ms / 最大 10.3 ms |

单条输入的稳态（新 / 基线）：`nihao` 353 / 345 µs，`keneng` 353 / 341 µs，`fangan` 521 / 500 µs，`ye'lang` 89 / 80 µs，`wan'dou'bu'xiang'wan` 272 / 265 µs，**`wojintianxiangyaoquxuexiao`（9 音节）1287 / 868 µs**。

P95 与最长输入的回退来自**探针**：这条输入有 8 条 parser 切分、7 条有资格，旧实现搜索 2 条，新实现探针 3 条再展开 2 条。已经按 `JOINT_PROBE_SPANS` 收到 128（原先 384 时这条输入是 3293 µs），把 `k` 从「无神经时 4」改回 1、并复用探针结果后从 3293 µs 降到 1287 µs。逐键（才是输入法真正按键的路径）只涨 6–9%。

共现索引是懒建的：第一次需要组合候选的按键付一次中位数 5.2 ms（`.qj` 词库下更小），之后只读。它在 `Engine` 生命周期内不变，用户词不进索引（用户词由词级查询直接命中）。

## 质量评测

冻结语料 `docs/notes/issue-8-eval.tsv`（83 句，SHA-256 `eb1d88f2893a0681d17d80b652d17911a40538164803c2345f250981627e386e`），产品词库、无语言模型、无个人学习、Linux release：

| 指标 | 基线 `9f1e817` | 本分支 |
| --- | ---: | ---: |
| 首选准确率 | 26.5% | 26.5% |
| 整句候选命中 | 26.5% | 26.5% |
| 字准确率 | 71.5% | 70.8% |
| 平均查询 | 1.2 ms | 1.4 ms |
| 最慢查询 | 5.4 ms | 10.9 ms |

**字准确率 -0.7pp 是真实的，不掩盖。** 这条指标取「第一个字数与原句相等的候选」，83 句里有 6 句的首选整句变了（其余 77 句前三候选逐行一致，含全部首选命中句）：

| 原句 | 基线首选 | 本分支首选 |
| --- | --- | --- |
| 两三个字母的简拼通常仍按词处理 | 两上颚字母的奖品通常人班次处理 | 两上颚字母的奖品通常仍然次处理（少一个字） |
| 再输入同一段拼音改选其他候选 | 再说如同一段拼音改选其他候选 | 在输入同意短片应该选其他候选 |
| 沃德书 / 我的书 | 我的是 | 我得数 |
| 上屏第 | 上平地 | 上平地（第二三名变） |
| 移动高亮 | 移动高粱 | 移动高粱（第二名变成正确的 `移动高亮`） |

这些首选变化来自 Task A：新实现多搜索了按探针分选中的切分。没有语言模型时，一元词频在这些句子上分不出更好的读法，所以有升有降。**没有神经模型 / 产品 LM 时不能据此宣称整体质量改善**；`移动高亮` 一句是明确改善。

关键正例两版一致：`keneng`/`ke'neng` → `可能`，`wan'dou'bu'xiang'wan` → `玩都不想玩`（无神经时基线也是），`ping'guo'bu'xiang'chi` → `苹果不想吃`，`jin'tian'bu'xiang'wan` → `今天不想玩`，`kan'dou'bu'xiang'kan` → `看都不想看`，`mai'dou'bu'xiang'mai` → `买都不想买`。`zuo'zhen` 的首选是词库里的 `坐镇`（`坐诊` 是 `DICTIONARY_GAP`，解码器没有凭空造）。

## 验证

```bash
cargo fmt --all -- --check                                          # 通过
cargo test -p qingjian-core --locked                                # 362 passed
cargo clippy -p qingjian-core --all-targets --locked -- -D warnings  # 通过
cargo test -p qingjian-platform --locked                             # 通过
cargo clippy -p qingjian-platform --all-targets --locked -- -D warnings  # 通过
cargo test --workspace --exclude qingjian-windows-server --exclude qingjian-macos --locked   # 全部通过
cargo clippy --workspace --exclude qingjian-macos --all-targets --locked -- -D warnings       # 通过
```

`--exclude qingjian-macos` 是本机环境限制：`objc2` 在非 Apple 平台上 `compile_error!`，基线同样失败。`qingjian-windows-server` 的两条 `dispatch::reload::tests::*`（`import_replace_and_remove_without_config_changes`、`dictionary_changes_do_not_retry_broken_config`）是 Issue 里已知的过期基线，本分支没有改动，仍然失败。

## 复现

```bash
cargo run -p qingjian-cli --release -- --dict assets/lexicon/dict.tsv "ye'lang" "hu'mao" "jing'shui" "sha'mo'hua"
cargo run -p qingjian-cli --release --example path_trace -- --dict assets/lexicon/dict.tsv "ye'lang"
cargo run -p qingjian-cli --release --example joint_bench -- --dict assets/lexicon/dict.tsv --repetitions 20
cargo run -p qingjian-cli --release -- --dict assets/lexicon/dict.tsv --eval-text docs/notes/issue-8-eval.tsv --misses 0
```

产品 LM 与神经模型（`data/generated/lm.qj`、`model.qjm`）不在本仓库，本环境无法复现带 LM 的整句评测；性能与质量数据都基于一元词频兜底，与 Issue #8 记录里带产品 LM 的 31.3% / 73.3% 不可直接比较。

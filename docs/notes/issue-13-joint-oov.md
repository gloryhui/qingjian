# Issue #13：动态切分预算与未登录组合候选

2026-09-30；基线 `origin/glory_dev` `9f1e817`。施工分支 `refactor/glory-dev-dynamic-joint-oov`。

## Preflight（改代码之前的真实状态）

| 任务 | 状态 | 代码证据 |
| --- | --- | --- |
| Task A | `NOT_FIXED` | `engine/query/joint.rs` 仍有 `const JOINT_SEGMENTATIONS: usize = 2;`，`best_joint_sentence` 先按 parser 顺序取有资格的切分，只把「有整段完整词证据」的那条换到第二位，然后 `.take(JOINT_SEGMENTATIONS)`。第三条及以后永远不做整句路径搜索。 |
| Task B | `NOT_FIXED` | 候选表里唯一的整句来源是 `plain_sentence_joint` 返回的**一条** `Conversion`；其余路径在 `select_joint_paths` 之后被丢掉，没有任何把「第二条及以后的合理组合路径」放进候选表的通道。产品词库跑 `ye'lang` 得到 `夜郎 / 夜郎自大 / 也 / 耶 / …`，没有 `野狼`。 |

## Task A：按证据分配切分预算（复核后重做）

### 算法

`best_joint_sentence` 分两步：

1. **探针**：切分跑一次整段窄搜索（`convert_path_groups(k = 1)`，单前驱、不建路线串），拿到它自己的最优完整路径分。词格进同一张 `SpanCache`，被选中的切分随后做宽搜索时不重复查词库；没有神经重排时探针结果**直接当最终结果复用**。
2. **按证据分配展开名额**（`plan_joint_segmentations`）：探针分最高的切分先入；parser 首选与「结构签名不同于最优探针的最佳切分」作为保底结构代表各占一名；其余按探针分从高到低填，落后最优探针超过 `JOINT_PROBE_SLACK` = 6 nat 或到 `JOINT_MAX_SEGMENTATIONS` = 4 就停。

### 探针名额按结构轮转，不由 parser 前缀顺序决定

第一版实现按 `eligible` 的 parser 顺序遍历，预算不足就 `break`，等于把「固定只搜前 2 条」换成了「只探 parser 前几条」——后排切分照样拿不到证据。重做后的规则：

- **顺序**（`probe_order`）：按结构签名（音节数、不完整音节数、各音节字母数）分组，组内保持 parser 顺序，然后逐轮在各组之间轮转。结构不同的切分在各自组里都是第一个，第一轮就轮到；「parser 排第 4 条」不会让它排到后面才拿到证据。
- **名额**（`JOINT_PROBE_LIMIT` = 4）：探针唯一真正花掉的是词格，每条 9 音节切分约 480 µs，所以名额有上限。名额之外的切分不会消失——它们仍以**结构代表**身份进 `plan_joint_segmentations`，在那里用自己的整句路径分参与竞争。

确定性回归 `engine/tests/joint/dynamic_budget.rs::fourth_parser_segmentation_still_gets_evidence_and_wins`：输入 `xiliaxinixiaanxianan` 的 parser 第 4 条切分 `xi lia xi ni xia an xi a nan` 才是对的，而它落在旧策略的射程之外（旧预算按 `lattice_spans` 记：前三条各要 36/36/44 个词格，128 在第三条之后只剩 12，第 4 条直接被 `break` 掉）。测试断言 `last_probed_segmentations()` 含下标 3、最终首选 `西俩西尼下安西阿南`、获胜切分就是第 4 条。同文件 `probe_order_rotates_structures_instead_of_walking_parser_order` 单测轮转顺序本身。

### 有界性

- 切分条数上限 `JOINT_MAX_SEGMENTATIONS` = 4（搜索）、`JOINT_PROBE_LIMIT` = 4（探针）。
- 没有扩大 `BEAM_WIDTH`（仍 8）、没有扩大 `RESCORE_PATHS`（仍 6）、没有同步等待神经模型。

### 回归

`engine/tests/joint/dynamic_budget.rs`：

- `parser_keeps_three_complete_segmentations_with_the_target_last`：`anxianan` 的前三条完整切分依次是 `an xian an`、`an xia nan`、`an xi an an`。
- `the_target_text_is_unreachable_on_the_first_two_segmentations`：直接在前两条切分的词格上跑 `convert_paths`，都造不出 `安溪安安`——旧实现不搜索第三条就永远拿不到它。
- `third_segmentation_wins_the_joint_search`：新实现首选 `安溪安安`，`Query.segmentations[0]` 与 `marked_text()` 都是 `an'xi'an'an`。
- 另有神经开 / 关、模型反对时仍进重排池、显式 `'` 是硬边界、个人 n-gram 把落后切分抬进预算、无个人证据时静态模型仍然说了算、双拼只解出一条切分等用例。

## Task B：未登录组合候选（复核后重做）

### 为什么不是「从整句 Top-K 里捞」

诊断（`path_trace`，产品词库）显示 `ye'lang` 只有一条完整切分 `ye lang`，束宽 8 全部占满时终端路径是：

```text
夜郎(-18.999) 也浪(-22.550) 也郎(-23.174) 也朗(-23.332) 也狼(-23.666)
也廊(-25.223) 耶浪(-26.181) 夜浪(-26.350)      ← 野狼 约 -27.7，第 8 名以外
```

按分数排序的路径池结构性地捞不到 `野狼`：`也` 的单字词频比 `野` 高 4.04 nat，`也*` 的组合把八个名额占满了。所以组合候选走**自己那条词格通道**（`engine/query/composed.rs`），不依赖整句路径池多留几条。

### 生成与筛选（复核后：共现不再是准入条件）

对每条做过多路径搜索、且音节数 ≤ `COMPOSED_MAX_SYLLABLES` = 6 的切分：

1. 只收**音节点全部完整**的切分（末尾还差字母时不造组合）。
2. 词格只按**敲的原样读音**建（模糊音 / 敲错变体是「用户可能敲错了」的猜测，拿它拼出来的新词没有依据）。这一步同时修掉一个副作用：格子的名额不会被 `与`、`于` 这种靠敲错边命中的高频字占满。
3. 每个切分点把头 / 尾两段各自的词格候选取出（`COMPOSED_PART_CANDIDATES` = 20，比整句词图的 6 宽——`藤`、`壶`、`豹` 这些词频排在后面的字在词图里根本进不来；走独立缓存键，不改变整句词图的格子内容）。
4. **准入只看结构与分数**：两个部分都是词库词、完整覆盖、无占位、配对分数落在最优路径的 `COMPOSED_MARGIN` = 10 nat 以内。**不看「这两个字有没有在别的词条里一起出现过」。**
5. 最优路径整段已经是一个**常用**完整词时整批不出：`COMPOSED_DOMINANT_WORD_FREQUENCY` = 3000。产品词库上的分布是断开的——常用词（`蛋糕` 13914、`没关系` 14807、`你好` 71960、`世界` 114813）与生僻词（`治三` 16、`夜郎` 83、`主桥` 164、`坐镇` 208、`制备` 903）差一个数量级以上，门槛取在中间任何位置结果都一样。没有这条规则时 `nihao` 会出 `你号`、`zhongguo` 会出 `中过`、`shanghai` 会出 `上还`，33 条常用输入里 20 条被污染；加上它之后降到 12 条。
6. 排序键 =（这个输入串下用户选过的次数, 路径分 + 共现加成）。共现（交界字在某个词条里同词出现过，如 `野`+`狼` ← `狼子野心`）**降级为排序加成** `COMPOSED_SUPPORT_BONUS` = 6 nat，不再决定谁有资格存在。
7. 最多 `COMPOSED_CANDIDATES` = 3 条；插在开头英文候选 / 整句首选 / 覆盖整段输入的完整词之后，**永远不占第一条**（`kaiha` 开了 `f_h` 时 `开放` 仍排第一）。

实现上配对分数分两段算：先只算分数不建字符串（`pair_score`），排序后只给前 `COMPOSED_SPLIT_KEEP` = 24 个配对建 `Conversion`。一个 2 音节输入有上百个配对，全部建对象是这一段的主要开销。


### 验收与控制案例

产品词库（`assets/lexicon/dict.tsv`），没有往任何词库文件加测试词：

| 输入 | 结果 | 说明 |
| --- | --- | --- |
| `ye'lang` | **`野狼` 第 2** | `夜郎` 仍是第 1；共现加成让它排在 `也浪` 前面 |
| `hu'mao` | `虎猫` 第 3 | 完整词优先的插入位置 |
| `jing'shui` | `井水` 第 2 | `净水` 仍是第 1 |
| `sha'mo'hua` | `沙漠化` 第 3 | |
| `teng'hu` | `藤壶` 在池子里（第 41 名） | 需要语义证据才排得进前 3，见下 |

`ye'lang` 下的同音堆砌（`也浪`、`也郎`）**现在也会出现**（共现不再是准入条件），但都排在 `野狼` 之后且总数不超过 3 条。`engine/tests/joint/composed.rs` 覆盖：受控词库里 `野狼`/`藤壶`/`云豹`/`纸杯`/`纸伞`/`石阶` 六个组合在**没有任何共现证据**的词库上照样可选（`compounds_are_admitted_without_dictionary_cooccurrence`），真实词库里这几个字对确认没有任何共现词条（`real_dictionary_compounds_have_no_cooccurrence_evidence_at_all`），结构不合格的组合一条不进池子（`structurally_invalid_compositions_never_enter_the_pool`），差太远的组合不进候选（`compositions_far_behind_the_best_path_stay_out`），同文本只出现一次（`generated_candidates_are_capped_and_deduplicated`）。

### 已知限制（不遮掩）

**1. `teng'hu → 藤壶` 在无语言模型时进不了前 3。**

`藤壶` 在产品词库 `teng hu` 的 66 个组合里按一元词频排第 **41**（`疼湖` 26865×22693、`藤壶` 6292×3018），任何有界的、不倒笛卡尔积的排序都到不了它；它也不在 `dict.tsv` 里（`assets/glossary/glossary-zh.tsv` 里有 `barnacle n. 藤壶`，但那份表不在默认加载路径上）。这不影响机制——`藤壶` 已经进了组合池，准入完全不依赖共现；把它排进前 3 需要语义证据：

- 语言模型：`a_language_model_that_knows_the_compound_lifts_it_to_the_front` 用给出了 `藤`→`壶` 接续分的受控模型证明，`藤壶` 从池子后段升到第一条并成为候选。
- 本环境没有仓库外的产品工件 `data/generated/lm.qj` / `model.qjm`，**产品 LM 下的表现未实测**，PR 标注 `WAITING_FOR_PRODUCT_LM_EVAL`。
- 另一半是词库覆盖（#14）：`藤壶` 不在 `dict.tsv` 里这件事本身属于产品数据缺口。

**2. 残余候选污染。** 33 条常用输入对照里 12 条有变化，其中 4 条是期望的正例（`ye'lang`/`hu'mao`/`jing'shui`/`sha'mo'hua`）；另外 8 条会多出 1–3 条似是而非的组合，插在完整词之后：

| 输入 | 多出的组合 |
| --- | --- |
| `zuo'zhen`（`坐镇` 208 次） | `做真` / `做针` / `坐针` |
| `zhi'bei`（`制备` 903 次） | `至北` / `指北` / `只被` |
| `zhi'san`（`治三` 16 次） | `只三` / `支三` / `之三` |
| `zhu'qiao`（`主桥` 164 次） | `猪壳` / `住桥` / `住巧` |
| `yun'bao`（没有完整词） | `孕包` / `蕴包` / `员报` |
| `hu'mao` | `和毛` / `和猫` |
| `wo'yan'jiu'sheng` | `窝研究生` / `沃研究生` |
| `yi'sheng'bu'xiang'lai` | `一声不响莱` / `一声不响赖` |

共同点是「整段拼音本来就没有常用完整词读法」，所以常用词门槛挡不住它们。`nihao` / `zhongguo` / `dangao` / `shanghai` / `mingtian` / `meiguanxi` / `wanshang` / `zaoshang` / `shi'jie` / `fangan` 这些高频完整词的输入**没有变化**。要再收敛同样需要语义模型。

**3. 只做两个部分的组合**；三段以上的 OOV 组合不生成。**4. 组合候选只给 ≤6 音节的切分**，整句长度上的「两个词拼起来」归整句路径管。


## 用户学习

组合候选是 `CandidateKind::Chinese`，带全部音节：

- **第一次可选**：`ye'lang` 下 `野狼` 在第 2 位，直接点选即可。
- **选择证据**：上屏走 `commit` 的中文候选分支，记 `record`（词频）与 `record_choice`（输入串下的选择次数）；下次查询 `choice_weight` 让它在同批组合候选里排到最前。
- **稳定用户词**：自动造词仍走原有规则——同一段拼音里连着选出 `野` + `狼`，第二次就 `learn_word` 出 `野狼`，之后整段拼音直接命中完整词（`auto_word_rule_still_turns_the_composition_into_a_user_word`）。
- **没选过就不学**：只查询不点选，用户词表与选择次数都不动（`unselected_compositions_are_not_learned`）。
- **神经分不是个人证据**：模型给组合候选打高分不产生词频 / 选择次数 / 用户词（`neural_score_is_not_personal_evidence`）。

不是 `Sentence` 的原因：`commit::sentence_words` 会用 **parser 首选切分**重算路径，组合候选来自另一条切分时重算不出来、直接返回 `None`，整条学习链（`finish_buffer` / `auto_word`）就断了。

## 性能（复核后重测）

Linux x86_64、release、`assets/lexicon/dict.tsv`（92,825 条）、无语言模型。基线是 `origin/glory_dev` `9f1e817` 的独立 worktree，命令：

```bash
cargo run -p qingjian-cli --release --example joint_bench -- --dict assets/lexicon/dict.tsv --repetitions 25
```

| 指标 | 基线 `9f1e817` | 本分支 |
| --- | ---: | ---: |
| 整串稳态 · 中位数 | 341.5 µs | **327.2 µs（−4%）** |
| 整串稳态 · P95 | 891.0 µs | 1552.0 µs（+74%） |
| 整串稳态 · 最大 | 918.2 µs | 1843.5 µs |
| 逐键冷缓存 · 中位数 / P95 | 302.5 / 1458.0 µs | 329.7 / 1589.9 µs（+9% / +9%） |
| 逐键热缓存 · 中位数 / P95 | 301.7 / 1422.0 µs | 319.0 / 1590.2 µs（+6% / +12%） |
| 逐键热缓存 · 最大 | 2447.4 µs | 2573.7 µs |

单条输入稳态（新 / 基线）：`nihao` 320/345、`keneng` 337/341、`fangan` 469/500、`wan'dou'bu'xiang'wan` 273/267、`ye'lang` **111**/79（组合候选；两段式建对象后从 431 µs 降下来）、**`wojintianxiangyaoquxuexiao`（9 音节）1546/881 µs**。

P95 与最长输入的回退全部来自**探针**：这条输入有 8 条 parser 切分、7 条有资格，基线搜索 2 条，本分支探 4 条（`JOINT_PROBE_LIMIT`）再展开 2 条。实测每多探一条 9 音节切分约 +480 µs——探针唯一真正花掉的是词格，而不同切分的前后缀不同、只能部分共享。这是「后排切分必须拿得到证据」的直接代价。已经收过的部分：`BEAM_WIDTH` 与 `RESCORE_PATHS` 不动；`span_candidates` 从「全排一遍再截断」改成 `select_nth_unstable` 部分排序后只排前几个；没有神经重排时探针结果直接复用（不跑第二遍）；组合候选两段式（先算分、只给前 24 个配对建对象）。

共现索引是懒建的，只在真的需要排序加成时才建；`set_extra_dictionaries` 会让它失效重建。


## 质量评测（复核后重测）

冻结语料 `docs/notes/issue-8-eval.tsv`（83 句，SHA-256 `eb1d88f2893a0681d17d80b652d17911a40538164803c2345f250981627e386e`），产品词库、无语言模型、无个人学习、Linux release：

| 指标 | 基线 `9f1e817` | 本分支 |
| --- | ---: | ---: |
| 首选准确率 | 26.5% | 26.5% |
| 整句候选命中 | 26.5% | 26.5% |
| 字准确率 | 71.5% | **71.8%** |
| 平均 / 最慢查询 | 1.2 / 5.4 ms | 1.8 / 12.1 ms |

字数取「第一个字数与原句相等的候选」。第一版实现（共现当硬门槛）是 71.6%，去掉硬门槛、改成「常用完整词门槛 + 共现只排序」之后回到 71.8%，比基线高 0.3pp。

关键正例两版一致：`keneng`/`ke'neng` → `可能`，`wan'dou'bu'xiang'wan` → `玩都不想玩`（无神经时基线也是），`ping'guo'bu'xiang'chi` → `苹果不想吃`，`jin'tian'bu'xiang'wan` → `今天不想玩`，`kan'dou'bu'xiang'kan` → `看都不想看`，`mai'dou'bu'xiang'mai` → `买都不想买`。`zuo'zhen` 的首选是词库里的 `坐镇`（`坐诊` 是 `DICTIONARY_GAP`）。


## 验证

```bash
cargo fmt --all -- --check                                          # 通过
cargo test -p qingjian-core --locked                                # 368 passed
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
cargo run -p qingjian-cli --release --example joint_bench -- --dict assets/lexicon/dict.tsv --repetitions 25
cargo run -p qingjian-cli --release -- --dict assets/lexicon/dict.tsv --eval-text docs/notes/issue-8-eval.tsv --misses 0
```

产品 LM 与神经模型（`data/generated/lm.qj`、`model.qjm`）不在本仓库，本环境无法复现带 LM 的整句评测；性能与质量数据都基于一元词频兜底，与 Issue #8 记录里带产品 LM 的 31.3% / 73.3% 不可直接比较。

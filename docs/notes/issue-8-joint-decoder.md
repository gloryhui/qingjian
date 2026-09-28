# Issue #8：切分与整句路径诊断

2026-09-28；基线 `origin/glory_dev` `b3d1476`。评测使用上游 `data-v2` 的 `dict.qj`、`lm.qj` 和 `model.qjm`；这些数据未提交到仓库。实机报告的“万都不想玩”在这份静态 LM 下复现为“晚都不想玩”，两者都不是用户目标“玩都不想玩”。

## Phase 1：路径在哪里丢失

`keneng`：parser 保留 8 种切分，前两种是 `ken'eng` 和 `ke'neng`。前者的词格没有完整双音节词，Viterbi 只有“肯 + eng 占位”，静态分约 `-40.41`（其中占位兜底 `-30`）；后者的词格有“可能”，完整词路径静态分约 `-6.98`，未被束剪枝。基线整句只搜索 parser 第一条切分，故这条更好的路径没有进入整句排序或神经重排；词级查询虽能找到“可能”，顶部拼音仍显示 `ken'eng`。

`wan'dou'bu'xiang'wan`：基线 parser 给出 8 条切分，首条是正确的五个音节；首音节词格含“玩”“万”“晚”，`wan dou` 词格也含“豌豆”。上游静态 LM 中，接“都”的累计分：晚 `-12.97`、万 `-13.09`、玩 `-13.52`；接“都不想”分别约 `-18.86`、`-20.04`、`-20.56`。兜底贡献均为零，个人学习为空。旧 `best_predecessor` 在这些词格只取一个前驱，选中“晚”，所以“玩都不想玩”和“万都不想玩”均未成为完整路径，也到不了神经模型。原因是前驱合并发生在模型看到句尾“玩”以前；不是“豌豆”需要保护。

`path_trace` 示例可以复查任意输入的切分、词格、前驱竞争、逐位置束路径与剪枝、静态/兜底/个人分数；加 `--neural <model.qjm>` 会打印候选完整句的神经分与最终分。它是离线诊断工具，不接入候选窗口。

## 实施与取舍

- 用户明确用 `'` 分开的合法完整音节直接固定；较长的段仍可在内部切分。未明确分隔的输入继续保留 parser 的多个候选。
- 词级查询已遍历全部切分。整句热路径先搜索 parser 首条，再在其余切分中优先选择已有整段词级证据的一条；至多搜索两条，按相同静态 LM、个人 n-gram、用户加分与纠错代价比较完整路径，再统一神经重排。胜出切分进入 `Query.segmentations[0]`，供顶部拼音显示。
- 开启神经重排时，词路径代表在后续词格继续传递，按词前缀逐层保护共同前缀后的分歧；跨切分合并也保留每种切分的结构代表。束宽仍为 8，重排总名额仍为 6。旧入口与联合入口先用完整路径池的最高总分计算 `neural_margin`，再分配重排席位。已有完整词的个人选择和上下文排序保持词级排序结果；重复文本不增加候选，只有它确实是首选时才同步顶部读音。个人 n-gram 保持独立，神经分只替换静态分；异步后台接口和光标前文不变。
- 没有采用输入串特判、补整句词条、提升“玩/可能”词频、硬锁长词、单纯加大束宽或把神经模型同步放进 KeyDown。

## 结果与性能

同一份从 `docs/user/input/index.md` 与 `docs/user/getting-started/first-input.md` 冻结的 83 句 TSV，CLI 冷启动评测；查询计时是本机 debug 构建，非 macOS 真人输入延迟。

| 指标 | 基线 | 本分支 |
| --- | ---: | ---: |
| 首选准确率 | 31.3% | 31.3% |
| 字准确率 | 73.3% | 74.1% |
| 平均查询 | 10.3 ms | 14.4 ms |
| 最慢查询 | 38.3 ms | 39.3 ms |

`keneng` 的 8 种 parser 切分保持不变，整句搜索首两条；`wan'dou'bu'xiang'wan` 的显式五音节从 8 种 parser 切分收敛为 1 种，词图为 15 个跨度。旧搜索的完整束路径都以“晚”开头；新搜索的最终 Top-K 含“晚都不想玩”“万都不想玩”“玩都不想玩”，后者词边界为 `[玩][都][不想][玩]`，静态分约 `-25.80`。在 `data-v2` 神经模型下，三者神经分分别约 `-25.04`、`-28.77`、`-22.65`，按默认权重算最终分约 `-25.02`、`-27.06`、`-24.22`，“玩都不想玩”成为首选。基线即使开启同一模型仍首选“晚都不想玩”，因为正确路径从未送去重排。

本分支有模型时 `keneng → 可能`、`wan'dou'bu'xiang'wan → 玩都不想玩`、`yi'sheng'bu'xiang'lai → 医生不想来`；无模型时前者仍为“可能”，第二例仍是“晚都不想玩”，医生例仍可能被“一声不响来”压过。“豌豆”完整词存在，但“玩都可以玩”“玩都不要玩”在完整句神经证据下仍能由拆分词路径胜出。基础词库缺“坐诊”（`DICTIONARY_GAP`），本改动未伪造词条。

逐键输入第二例共 20 键，本分支在无神经模式下平均 `3.97 ms`、最慢 `30.76 ms`；接异步神经模型但不等待后台分数时平均 `4.31 ms`、最慢 `31.46 ms`。CLI 同步神经模式在本机 CPU 上约需 1.5 秒打分，只用于离线诊断；输入法按键路径使用原有异步接口。该逐键计时包含候选注释，不包含后台模型完成后的等待时间。

## 已知限制

静态 bigram 只看前一个词，不能可靠利用较远的句尾语义。无神经模型时“玩都不想玩”仍未达到 Issue 的首选要求，因此状态为 `PARTIALLY_FIXED / NEEDS_FOLLOWUP`。热路径只搜索两种切分；有整段词级证据的较后切分优先，但其他第三条及以后切分仍可能漏掉，需要更完整的共享拼音/词 lattice 与搜索预算设计。83 句评测首选率没有提升，平均 debug 查询增加约 4.1 ms；尚无 macOS 打包后的真人输入验收。

## PR #9 复核修复与可复现评测

R1：完整词之间以词级 `ranking::rank` 已排出的个人选择与上屏上下文顺序为准；联合路径不能仅因切分不同越过它。`learned_complete_word_keeps_its_choice_rank_across_segmentations` 修复前失败（“反感”覆盖已学的“方案”），修复后神经开/关均通过。

R2：旧入口与联合入口共用 `retain_neural_eligible_by_text`，以完整路径池最高总分计算门槛，先确定有资格的文本组再分配名额；一个文本组中的同文本路径仍按各自总分、静态分、个人证据和代价重算。`joint_margin_excludes_weak_protected_path_before_neural_scoring` 修复前失败（弱路径进入模型并翻盘），修复后通过；双切分、异步和个人总分的测试同样通过。

R3：`route` 保存部分路径的完整词序列，`representative_indices` 由浅到深选择分歧代表。把同一回归临时放在 B 的 Core 测试中，原实现分别首选“我晚都不想玩”“我今天卖都不想买”，修复后目标完整句均进入 scorer 并成为首选。预算不足时先保留较早分歧，组内按累计分取高者；有限束不能保证所有路径存活。

R4：全局六席在不同切分间轮流取结构代表，再按总分填充。双切分池的“玩”代表与真实联合入口测试均通过；同一真实入口测试在 B 上失败，目标句没有进入 scorer。门槛外路径仍不能被保护规则重新引入。

R5：同文本同读音候选去重后，只在它确实是第一中文候选且路径未被改音时更新顶部切分。`duplicate_sentence_text_still_updates_winning_preedit` 在 B 上得到 `ken'eng`，修复后是 `ke'neng`；个人学习、英文位置和原有双拼/注音回归保留。

冻结语料为 [issue-8-eval.tsv](issue-8-eval.tsv)，83 句由仓库公开的 `docs/user/input/index.md` 与 `docs/user/getting-started/first-input.md` 冻结；SHA-256 为 `eb1d88f2893a0681d17d80b652d17911a40538164803c2345f250981627e386e`。逐例 A/B/C 首选见 [issue-8-eval-results.tsv](issue-8-eval-results.tsv)。A 是 `b3d147614a7b8cf141023f3654999cfb0f5fc72e`，B 是 `f2698c8b1dad52fc4dff46c82a88c2787a0fd3d3`，C 是本 PR 后续提交。A→B 有两句首选文本局部改善但仍错；B→C 的 83 句首选文本完全相同，没有首选准确率提高或退化。

数据为上游 `data-v2` 产品工件：`dict.qj` SHA-256 `3e33b16a84df555e6f16d52ac8ab3c2c6b6f1f71734e69463861fd5abd9c19dc`，`lm.qj` `f9fb7b4433dddce86610a5d33e571bea963f6c2a5d960b8bc908f25024d34cd1`，`model.qjm` `eed5bd0bda0c7bd8b43d1acb2dc4678d4bbe295bd47b2b0d4eeace0af9daff4d`。三份工件都不入库。三次评测都用同一文件、无个人学习初始状态、Linux x86_64 debug 构建、同一 Rust/Cargo 配置。每个 SHA 在独立 worktree 中把 `data/generated` 指向同一份产品工件，运行：

```bash
export OPENSSL_INCLUDE_DIR=/tmp/qingjian-issue8-openssl/extracted/usr/include
export OPENSSL_LIB_DIR=/tmp/qingjian-issue8-openssl/extracted/usr/lib/x86_64-linux-gnu
cargo build -p qingjian-cli
cargo run -p qingjian-cli -- --dict data/generated/dict.qj --eval-text /path/to/issue-8-eval.tsv --misses 1000
python3 /path/to/C/tools/issue8-bench.py /path/to/qingjian-cli /path/to/issue-8-eval.tsv
cargo run -p qingjian-cli -- --dict data/generated/dict.qj --neural /path/to/model.qjm "keneng" "ke'neng" "wan'dou'bu'xiang'wan"
```

| 指标 | A base | B review | C 修复 |
| --- | ---: | ---: | ---: |
| 首选准确率 | 31.3% | 31.3% | 31.3% |
| 字准确率 | 73.3% | 74.1% | 74.1% |
| 整串冷启动平均查询 | 10.4 ms | 14.3 ms | 14.0 ms |
| 整串冷启动最慢查询 | 38.9 ms | 40.3 ms | 38.3 ms |
| 批量直接查询中位数 | 8.69 ms | 12.27 ms | 12.49 ms |
| 批量直接查询 P95 | 21.74 ms | 30.42 ms | 31.48 ms |
| 批量直接查询最大值 | 38.86 ms | 39.35 ms | 41.71 ms |

批量直接查询每版预热一轮，再独立运行 5 轮 × 83 句（415 个样本）；CLI `--limit 0` 的 `total` 包含查询和释义标注，与上表整串冷启动评测口径不同，不包含神经模型。C 在第二次同口径运行得到中位数 12.34 ms、P95 30.59 ms、最大 39.50 ms，说明单机测量有波动。上述差异不能推断 macOS 按键延迟。产品工件的关键例：无 neural 时 `keneng`、`ke'neng` 均首选“可能”，显式 `wan'dou'bu'xiang'wan` 仍首选“晚都不想玩”；同步和异步 neural 均可把后者排成“玩都不想玩”。

## R6：同文本路径去重与异步性能复核

复核 HEAD `dfd083763d0726d0a5e83dbed711fff39fde328a` 上，终端节点先按路线多样性改序、再按文本去重，确实能把更低分的同文本路径留在前面。用 `wo'yan'jiu'sheng` 和五词控制词格先新增 `diverse_routes_keep_the_best_scoring_path_for_duplicate_text`：修复前 Rust 测试实际返回“我研究生”路径分 `-3.0`，预期的 `[我][研究][生]` 分 `-2.0` 被去掉；修复后文本仍只有一项且保留 `-2.0`。

`convert_path_groups` 现在先按输出文本分组，路线代表只决定哪些不同文本占据 `k` 个位置，组内束宽保留的全部路径再交给 Engine。`convert_paths` 面向旧调用方时返回该组重排前总分最高的路径；普通整句与联合整句入口则将路径变体各自送入同文本共享的神经缓存，并按 `score + λ·(neural - static_score)` 分别计算。margin 以完整池最高总分决定文本组资格：组内某条路径符合门槛时，其他同文本路径不会提前丢掉个人分、选择加分、纠错代价或读音映射。联合选择器只为不同文本分配名额，保留所选文本跨词边界、跨切分的路径变体。

回归包括 `diverse_routes_keep_the_best_scoring_path_for_duplicate_text`、`neural_rescore_keeps_the_best_same_text_path_after_terminal_diversity` 和 `same_text_routes_survive_selection_order_and_keep_path_specific_scores`。首项在修复前失败、修复后通过；整句入口的 mock scorer 实测“我研究生”仍以 `-1.50` 胜过“我研究声”的 `-1.75`。联合选择器测试交换重复项顺序并使用不同词边界和不同切分，确认同文本只占一个预算名额、所有路径分量保留，最终由各自路径分数决定代表。

同一份冻结 TSV 另对比 A=`b3d147614a7b8cf141023f3654999cfb0f5fc72e`、B=`dfd083763d0726d0a5e83dbed711fff39fde328a`（R6 修复前 PR HEAD）、C=`2e8de33`（R6 代码提交）。三版使用相同 `dict.qj`、`lm.qj`、Linux x86_64 debug 配置及空个人学习状态：A 首选/字准确率 `31.3% / 73.3%`、平均/最大 `10.2 / 38.4 ms`；B `31.3% / 74.1%`、`14.5 / 39.7 ms`；C `31.3% / 74.1%`、`14.4 / 39.1 ms`。B→C 的 57 条首选错误及其前三候选逐行一致；当前修复没有改变冻结语料质量分数。

V1 性能复核使用新加的 `apps/cli/examples/issue8_async_bench.rs`，产品词库与 bigram LM 相同，scorer 为确定性 mock 并在后台被 channel gate 暂停；因此它测的是 R3/R4 `k=6` 热路径与异步首次返回，不是产品神经模型耗时。每类 20 次：整串查询在新 Engine 上冷启动；逐键 cold 从新 Engine 输入每个前缀，神经分缓存冷，首个可评分查询后启动并挂起后台 scorer，后续按键查询继续；释放 scorer、等待各前缀分数入缓存后，对同一前缀序列测 hot。表中是微秒，格式为中位数 / P95 / 最大值；B=`dfd0837`，C=`2e8de33`。运行环境为 Linux x86_64、rustc/cargo `1.96.0`、debug 构建。

| 输入 | B 整串 cold | C 整串 cold | B 逐键 cold | C 逐键 cold | B 逐键 hot | C 逐键 hot |
| --- | --- | --- | --- | --- | --- | --- |
| 短句 `nihao` | 4247 / 4296 / 37311 | 4320 / 4351 / 37871 | 2847 / 5815 / 5953 | 2899 / 5902 / 6120 | 1706 / 5458 / 5622 | 1741 / 5571 / 5622 |
| 长句 `wojintianxiangyaoquxuexiao` | 15240 / 15420 / 16892 | 15419 / 15572 / 20160 | 8746 / 24335 / 29254 | 8847 / 24710 / 35035 | 7384 / 24040 / 29051 | 7553 / 24331 / 29536 |
| 共同前缀 `wo'jin'tian'mai'dou'bu'xiang'mai` | 8812 / 9079 / 10153 | 8881 / 9005 / 9012 | 2552 / 6520 / 7313 | 2578 / 6557 / 7514 | 2278 / 6107 / 6282 | 2306 / 6180 / 7762 |
| 多切分 `keneng` | 5018 / 5042 / 5053 | 5056 / 5161 / 5184 | 4131 / 5475 / 5790 | 4198 / 5528 / 6513 | 3722 / 5134 / 5208 | 3776 / 5202 / 5318 |

B/C 的逐键中位数差约 1–2%，单机 debug 测量存在调度噪声；长句 cold 最大值从 29.25 ms 到 35.03 ms，不能排除系统干扰，也不据此声称严格无回归。20 次样本中，后台 gate 阻塞时仍分别完成 440 / 540 / 40 / 20 个长句 / 共同前缀 / 多切分 / 短句查询；query 返回耗时不包含等待 scorer，也不把 mock 的后台等待混进耗时。该例程只测 debug 构建，产品同步模型另用离线整句查询验证，不能据此推断 release 或 macOS 实机延迟。

产品工件关键复测：无 neural 时 `keneng → 可能`、`ke'neng → 可能`、`wan'dou'bu'xiang'wan → 晚都不想玩`、`ping'guo'bu'xiang'chi → 苹果不想吃`、`yi'sheng'bu'xiang'lai → 一声不响来`、`jin'tian'bu'xiang'wan → 今天不想玩`；同步产品 scorer 开启后，对应首选为 `可能`、`可能`、`玩都不想玩`、`苹果不想吃`、`医生不想来`、`今天不想玩`。同步产品模型单次 query 耗时约 0.44–1.45 秒，仅用于离线质量验证；不代表异步按键返回时间。

复现命令（B、C 在各自 worktree 执行；先把 C 的 example 文件复制进 B，因为 B 尚未包含该计时工具）：

```bash
export OPENSSL_INCLUDE_DIR=/tmp/qingjian-issue8-openssl/extracted/usr/include
export OPENSSL_LIB_DIR=/tmp/qingjian-issue8-openssl/extracted/usr/lib/x86_64-linux-gnu
cp /path/to/C/apps/cli/examples/issue8_async_bench.rs /path/to/B/apps/cli/examples/
cargo run -p qingjian-cli --example issue8_async_bench -- --dict data/generated/dict.qj --lm data/generated/lm.qj --repetitions 20
```

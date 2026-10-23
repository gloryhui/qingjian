# 青简源仓库与自有开发分支对照（2026-10-23）

## 结论

源仓库已有两个直接对应我们反馈的实现：**分段选词暂存并支持退格撤回**、**修复分段选词后的单引号残留与重复上屏**。但它们与我们的学习时机、输入会话隔离和分隔符处理范围不完全相同，不能直接删除我们的整套实现。

我们的 **macOS Shift 切换与英文完全直通、应用隐藏后的候选窗恢复、词语只占前两页、Option+Enter 输出高亮译词、测试版独立安装**，当前上游没有等效实现。完整词首位保护、动态切分与未登录组合候选，也不能认为已被上游的新模型和排序更新完整覆盖。

源仓库已经历 0.2 与 0.3 两轮较大更新。建议以最新上游为基础分批迁移我们的必要差异，先保留当前可用分支；此次仅完成提交与比较，尚未合并上游。

## 比较对象与范围

| 对象 | 本次核验的位置 | 说明 |
| --- | --- | --- |
| 源仓库默认主线 | [`qingjian-team/qingjian:main`](https://github.com/qingjian-team/qingjian/tree/10e86493d8f1e59a7483c8afdf910d84bd0908a0) · `10e86493` | 2026-10-23 11:37:52 +08:00；最新提交为无拼音词表导入；当前版本 0.3.2-dev |
| 源仓库最新稳定版本 | [`v0.3.1`](https://github.com/qingjian-team/qingjian/releases/tag/v0.3.1) | 2026-10-22；主线还含发版后的新增代码，见后文 |
| 我们的开发主线 | [`gloryhui/qingjian:glory_dev`](https://github.com/gloryhui/qingjian/tree/5f8762f04e840a75e6ffa0c695594a6028d7d050) · `5f8762f0` | 含已合并 PR #17；仍是用户指定的开发主线 |
| 本轮自有代码分支 | [`codex/macos-shift-switch`](https://github.com/gloryhui/qingjian/tree/codex/macos-shift-switch) · 代码快照 `28989cce` | 以 glory_dev 为基础，收齐两个工作目录的未提交代码；保留既有 0.1.8-dev 版本调整 |
| 共同起点 | `40e3e550425466e6ba9c0a14de3a77ed04862799` | 本机 main 仍停在这里，未移动 |

本次在代码提交后重新 fetch 两个远端，并用远端 HEAD 确认源仓库默认分支为 main。相对共同起点：上游新增 **367 个提交**，glory_dev 独有 **17 个提交**，本轮代码分支独有 **19 个提交**（含本轮 2 个代码提交，不含本报告提交）。上游变更涉及 1,640 个文件；本轮相对 glory_dev 改动 58 个文件。

`git cherry upstream/main glory_dev` 的 17 条结果均为 `+`，表示没有检测到相同补丁；这不排除上游以不同代码实现相似功能。下面按实现与测试核对，而非仅按提交标题判断。

范围是共同起点之后的主要用户可感知变化，重点覆盖本轮反馈和我们历次修改；不是对 367 个提交逐条进行完整代码审查。完整历史可查看 [上游范围对比](https://github.com/qingjian-team/qingjian/compare/40e3e550425466e6ba9c0a14de3a77ed04862799...10e86493d8f1e59a7483c8afdf910d84bd0908a0)。

## 一、与我们的修改直接相关的问题

| 问题 / 我们的行为 | 上游最新实现 | 判定及处理建议 |
| --- | --- | --- |
| 连续输入后先选前半个词，保留在组句中，退格恢复拼音 | [`05012b8b`](https://github.com/qingjian-team/qingjian/commit/05012b8b)：新增 `Engine.held`、`hold_or_release`、`undo_selection`；`hold_selections` 默认开启，已接入桌面三平台，并有 5 项 held 回归测试 | **主要交互已覆盖。** 可复用上游作为后续基础；但不是与我们的事务实现完全等价，差异见下节 |
| `shan'gou` 先选「闪」后留下单引号，继续选词重复上屏 | [`47199687` / #697](https://github.com/qingjian-team/qingjian/commit/47199687)：`rest_at` 在缓冲区首位也跳过单引号 | **普通单引号问题已修。** 我们 PR #5 的 `7731349` 还在前段消耗结束时一并吃掉连续分隔符，需保留连续单引号、原样上屏等测试，不能视为同一补丁 |
| Shift 短按切换不可靠；重复 FlagsChanged、长按或 Shift+B 误切 | 上游 macOS 仍通过 `caps_lock_on()` 判英文，`dispatch_event` 只接受 KeyDown；没有我们这套 ModeState、重复事件过滤、500ms 短按界限和 150ms 待定确认 | **未覆盖，而且产品行为不同。** 保留我们 Shift 状态机与回归测试 |
| 中文模式用 Shift+B 输入 Book，保留英文联想 | 上游仍支持 `[general] shift_letter = "compose"`；但该能力在共同起点已经存在，并非这次新修复。它没有我们的 Shift 切换状态机，因此也没有修复我们的重复事件问题 | **共同已有能力，不计为上游新修复。** 迁移时保留 compose 配置与我们“确实按住 Shift”的判定 |
| 英文模式所有 KeyDown 原样交给系统 / 应用 | 上游新增按应用白名单给英文候选；`handle_text` 对名单外应用透传，默认仍向浏览器、聊天和文档类应用提供候选。事件前段还会处理提示、面板和快捷键 | **部分场景有透传，但不等价。** 我们要求在读取字符及 Engine / UI 路由前直接返回 false，应保留 |
| 中文候选存在时，纯英文前缀补全不能抢首位（PR #4） | 最新 `extras.rs` 的 `english_first` 仍未要求 `word.is_some()`，和共同起点相同；本轮新增的是 `english_in_chinese` 总开关 | **未覆盖。** 总开关不能替代我们的首位规则 |
| 完整词不被拆读整句挤掉；误选整句删除后撤销那次学习（PR #17） | 上游有部分完整词 / 原读音保护，又新增多音字读音占比与打平切分比较；但不具备我们同一套完整词优先和整句选择撤销规则。0.3.1 更新日志还列出“整句选过后首选可能不变”的已知问题 | **部分重叠，不能判定已完整修复。** 保留我们完整词、神经重排、显式整句选择与撤销的回归语料 |
| 多种拼音切分联合比较、动态预算、词库未收录的组合候选（PR #9 / #15） | [`d83b6397`](https://github.com/qingjian-team/qingjian/commit/d83b6397) 对结构打平切分最多比较 3 条；另有通变模型生成词图无法覆盖的整句 | **方向有重叠，算法不同。** 上游并未引入我们的词法证据探针预算与两段组合生成；需要同语料比较效果与延迟后取舍 |
| 词语候选只占前两页，后续方便选单字 | 上游查询仍按总上限 500 排候选，没有我们按页大小设置 `word_candidate_limit` 的过滤策略 | **未覆盖。** 保留 Core 过滤与 macOS 两页配置；不应套到英文模式补全 |
| Option+Enter 输出当前高亮候选第一条译词 | 上游仍以 Option+数字、Shift+Option+数字选择译词，另有鼠标点击译词；`ShortcutConfig` 没有 `commit_translation` | **未覆盖。** 点击译词是新增替代入口，不能替代用户要求的键盘组合 |
| 输入框有拼音却没有候选窗，特别是“隐藏其他应用”之后 | 上游窗口仍仅 `orderFrontRegardless()` 并按活动 Space 重建；没有 `isHidden` + `unhideWithoutActivation`。偏好窗口关闭仍切回 Prohibited，启动阶段也没有我们提前设 Accessory 的处理 | **未覆盖本次根因。** 上游 macOS 27 的中文空白窗口修复属于字体轮廓问题，和应用被隐藏是两类问题 |
| 安装后启动新版、与正式版隔离共存 | 上游 postinstall 新增清输入源缓存和注销提示，但仍是杀旧进程后注册，未包含我们的等待旧进程退出、`open -g` 启动新版；打包脚本没有我们的测试身份 | **各自解决不同环节。** 保留共存身份和启动逻辑；另评估吸收上游缓存处理 |
| 默认不出 emoji，并可设置 | 上游先新增开关，后来发展为 emoji / 颜文字 / 符号分别 0–3 档 | **设置能力已覆盖并扩展。** 默认值和配置键不同，升级时需迁移我们用户选择的关闭状态 |

## 二、分段选词实现的关键差异

这是最值得复用上游、也最需要保留我们测试的一项。

| 细节 | 我们代码快照 `28989cce` | 上游 `10e86493` |
| --- | --- | --- |
| 暂存入口 | `select_candidate` 返回 `None` 表示仍在组句 | `commit` 返回空字符串表示仍在组句 |
| 暂存内容 | 候选、原按键与大小写、查询快照、辅码 | `HeldSelection` 保存文本与已消耗的原 Composition 前缀 |
| 学习时间 | 分段暂存不调用 `commit_with`；整段真正确认时重放并学习 | 先调用 `commit_with` 学习，再由 `hold_or_release` 暂存显示文本 |
| 撤销学习 | 尚未学习，因此撤销与取消都不会留下这次暂存的学习 | `undo_selection` 将最近记录标记 erased，测试验证“撤销后另选”才退回学习；`clear` 直接清 held，未在此处逐项撤销 |
| 输入上下文隔离 | selected 随 `EngineSession` 交换，有专门回归测试 | 当前 `EngineSession` / `swap_session` 未保存或交换 held；应把我们的会话隔离测试移过去验证 |
| 确认路径 | 原样回车、译词确认、云整句确认均测试前缀只出现一次 | 上游也在 raw / translation / prediction 路径拼接 held；需在新的 API 下重跑边界测试 |

以上是源码差异，不把未实机复现的边界直接称为上游已确认缺陷。后续若采用上游 held，应重点验证：取消是否学习、输入框切换、大小写与双拼、尾段编辑、译词和云候选确认。

证据：[上游 held 实现](https://github.com/qingjian-team/qingjian/blob/10e86493d8f1e59a7483c8afdf910d84bd0908a0/crates/qingjian-core/src/engine/held/mod.rs)、[学习先于暂存的调用顺序](https://github.com/qingjian-team/qingjian/blob/10e86493d8f1e59a7483c8afdf910d84bd0908a0/crates/qingjian-core/src/engine/commit/mod.rs#L90)、[上游 held 测试](https://github.com/qingjian-team/qingjian/blob/10e86493d8f1e59a7483c8afdf910d84bd0908a0/crates/qingjian-core/src/engine/tests/held.rs)、[上游会话状态](https://github.com/qingjian-team/qingjian/blob/10e86493d8f1e59a7483c8afdf910d84bd0908a0/crates/qingjian-core/src/engine/session.rs)。

## 三、源仓库另外修复的问题

这些是共同起点之后上游新增的修复，并非我们的分支已经获得。

| 范围 | 新修复 | 代表提交 / 来源 |
| --- | --- | --- |
| Core 排序 | 多音字少见读音按读音占比扣分；`ou` 不再让「区」借 qu 的高频抢前；结构打平时 `shangou` 可选「山沟」 | [`d83b6397`](https://github.com/qingjian-team/qingjian/commit/d83b6397)，含 `polyphone.rs` 测试 |
| 本地模型 | 只接受最新请求的结果；macOS 结果到达并重查后再调度，避免快打时最后一段收不到结果 | [`409781d7`](https://github.com/qingjian-team/qingjian/commit/409781d7)、[`5fb6809f`](https://github.com/qingjian-team/qingjian/commit/5fb6809f) |
| macOS 显示 / 稳定性 | macOS 27 中文候选空白改从字体轮廓兼容处理；macOS 14 本地模型回退 CPU；macOS 13 兼容修复 | [`5a1e897a`](https://github.com/qingjian-team/qingjian/commit/5a1e897a)、[`e440b03a`](https://github.com/qingjian-team/qingjian/commit/e440b03a)、0.2.1 更新日志 |
| macOS 安装 | 注册前清共享及沙盒应用的输入源缓存，解决开着沙盒应用升级后切输入法闪退；缺输入源时明确提示注销 | [`1eced5e3`](https://github.com/qingjian-team/qingjian/commit/1eced5e3)、[`584c8297`](https://github.com/qingjian-team/qingjian/commit/584c8297) |
| macOS 配置 / 布局 | 新用户词库文件自动热加载；激活时设 ABC 布局，修 QWERTZ 的 z / y 互换；设置文本框失焦即保存 | [`ccbc66b3`](https://github.com/qingjian-team/qingjian/commit/ccbc66b3)、#588、[`1368acce`](https://github.com/qingjian-team/qingjian/commit/1368acce) |
| 拼音 / 双拼 | j/q/x/y 后 v 代 ü、双拼 ü 键与模糊音组合、末尾单声母简拼等补齐；全角 Shift+[ ] 出「」 | 0.2.0–0.3.0 更新日志；[`4b7f2e72`](https://github.com/qingjian-team/qingjian/commit/4b7f2e72)、[`273c53e9`](https://github.com/qingjian-team/qingjian/commit/273c53e9) |
| Windows | 验证码数字重复 / 密码数字倒序、后台断连吞键、候选定位与搜索栏遮挡、应用退出后的残留窗口、自定义短语加载 | #345、#251、#258–260、#735、#587、#471；0.2.0–0.3.0 更新日志 |
| Linux | Wayland Chromium 重复上屏、配置秒级热加载、托盘巨图与字体检测、Fcitx5 译词词性和假名 | #305、#394、#578、#683、#694、#696；0.2.1–0.3.0 更新日志 |
| 云服务 / 依赖 | 合并连续请求并限频，认证失败或余额不足暂退避；证书链校验依赖随安全公告更新 | #563；[`372cbf0a`](https://github.com/qingjian-team/qingjian/commit/372cbf0a)、[`fc67e37e`](https://github.com/qingjian-team/qingjian/commit/fc67e37e) |

跨平台长尾条目以官方变更记录及关联提交为依据，本轮没有在 Windows / Linux 真机重新验证。

## 四、上游新增功能

下表比较我们当前功能快照，强调实际新增或升级的部分，避免把早已有的五笔、双拼和模型能力全算成新功能。

| 类别 | 相对我们当前分支的新增 / 升级 | 所在版本或提交 |
| --- | --- | --- |
| 本地模型 | 新增含章·通变整句生成 / 排序；知意候选选择模型；知微改按需下载；统一模型选择与省电 / 均衡 / 准确档；拿不到前文时按输入框使用近期输入 | 0.2.0、0.3.1；[`a41d1cf8`](https://github.com/qingjian-team/qingjian/commit/a41d1cf8)、[`2498336f`](https://github.com/qingjian-team/qingjian/commit/2498336f)、[`3be348d4`](https://github.com/qingjian-team/qingjian/commit/3be348d4)、[`04068a5c`](https://github.com/qingjian-team/qingjian/commit/04068a5c)。Windows 知意最终仅 CPU，不能按较早 DirectML 提交宣称已启用 GPU |
| 设置程序 | 三平台共用新设置程序，分组、搜索设置项、文档入口；旧设置保留一版回退 | 0.3.0；`apps/settings` |
| 候选管理 | 候选右键置顶、降权、删除；自定义短语由位置改为权重；原 Shift+数字删词移除 | [`b6319013`](https://github.com/qingjian-team/qingjian/commit/b6319013)、[`95b2e196`](https://github.com/qingjian-team/qingjian/commit/95b2e196) |
| 候选外观 | 多套主题与热加载、自定义字号、动画开关、鼠标点候选 / 译词、横排多行、独立状态条大小、横排拼音与候选两区导航 | 0.2.0–0.3.0 |
| 符号输入 | 独立符号 / 表情面板；emoji、颜文字、符号分别设数量；圆圈数字；v 模式日期 | 0.2.1–0.3.0；[`e6b91031`](https://github.com/qingjian-team/qingjian/commit/e6b91031)、[`61e0ea1d`](https://github.com/qingjian-team/qingjian/commit/61e0ea1d)、[`3f769c6a`](https://github.com/qingjian-team/qingjian/commit/3f769c6a) |
| 输入方案 | 既有五笔扩成自定义形码方案 / 词库导入，并支持顶字、满码唯一自动上屏与空码清除；新增紫光、拼音加加、自定义双拼、键位 / 字根提示；繁体分台 / 港 / 通用 | 0.3.0；[`84196a8e`](https://github.com/qingjian-team/qingjian/commit/84196a8e)、[`bd4355d8`](https://github.com/qingjian-team/qingjian/commit/bd4355d8) |
| 词库 / 拼音行 | 补充青简词库与约 3.6 万生僻字；词库数字声调；按高亮候选切拼音并可显示带调拼音 | 0.2.1–0.3.0；[`0ca1b954`](https://github.com/qingjian-team/qingjian/commit/0ca1b954)、[`0c934488`](https://github.com/qingjian-team/qingjian/commit/0c934488)、[`db5d6fe3`](https://github.com/qingjian-team/qingjian/commit/db5d6fe3) |
| 学习 | 词频随时间衰减；简拼 / 全拼 / 双拼按读音共享选择偏好；生词本时间线、词表与整句翻译记录 | [`9bb6b2a3`](https://github.com/qingjian-team/qingjian/commit/9bb6b2a3)、[`1f6fbd82`](https://github.com/qingjian-team/qingjian/commit/1f6fbd82)、[`c97ff2b6`](https://github.com/qingjian-team/qingjian/commit/c97ff2b6) |
| 混输 / 标点 | 中英数字自动加空格；中文拼音英文补全独立开关；组句标点先上屏可选；`用户名@` 给常见邮箱候选 | 0.2.0–0.3.0；[`17f819b7`](https://github.com/qingjian-team/qingjian/commit/17f819b7) |
| 学习语言 / 云服务 | 新增越南语、德语；GLM；按需联想与自动联想开关；统计区分输入与实际云请求、小时 / 日历展示 | 0.2.0–0.3.0 |
| 平台 | Windows 默认英文 / 按应用记模式、更多快捷键；Linux 中英切换、Fcitx4、云联想及选区翻译；Android 首个 arm64 预览版，尚无桌面本地整句模型 | 0.2.1–0.3.0；[`2726f342`](https://github.com/qingjian-team/qingjian/commit/2726f342) |

### 0.3.1 发版后，main 另外新增的功能

1. [`401e8fd8` / #754](https://github.com/qingjian-team/qingjian/commit/401e8fd8)：连续输出西文译词自动补词间空格，即便未启用通用自动空格。
2. [`10e86493` / #718](https://github.com/qingjian-team/qingjian/commit/10e86493)：导入只有词、没有拼音的词表，读音由随包词库派生；有导入测试和 CLI 支持。
3. [`a4e1a4cc`](https://github.com/qingjian-team/qingjian/commit/a4e1a4cc)：纠正「朝阳市」「朝阳花」读音；另有证书校验依赖更新。

这些已在 main，不应表述为下载 v0.3.1 就一定包含。

主要版本依据：[固定快照的完整更新日志](https://github.com/qingjian-team/qingjian/blob/10e86493d8f1e59a7483c8afdf910d84bd0908a0/CHANGELOG.md)。

## 五、后续整合建议

1. **继续保留当前自有分支作为可用基线。** 上游到我们的差异远超一个补丁；此前测试版身份和数据目录继续独立。
2. **以最新上游新开迁移分支，先接 macOS 行为。** 迁移 Shift 状态机、最前层英文直通、候选窗 unhide / Accessory 和安装启动逻辑；合并上游安装缓存处理，保留正式版与测试版隔离。
3. **优先复用上游分段选择，再补事务与会话保证。** 搬我们的取消不学习、上下文隔离、双拼大小写、译词 / 云确认回归测试，逐项决定保留哪段实现。
4. **按用户需求补两页词语限制与 Option+Enter。** 这两项仍是明确差异；对接上游的新页布局、右键菜单与快捷键体系。
5. **排序单独比较后再定。** 在相同词库、干净与已有学习数据、相同模型配置下，对照完整词 / 同音整句 / 动态切分 / 未登录组合；同时测首键和长句延迟。上游更新了模型与词库，不能把候选变化一概归因于算法。
6. **最后迁移配置与数据。** 上游 emoji 数量、主题、短语权重、模型和应用名单均有变化，保留用户原来的关闭 / 开启选择，避免仅覆盖配置文件。

以上为建议顺序，本次没有实施上游合并或改变正在安装的输入法。

## 六、本次提交与验证

- `1c5beb3b`：收齐主目录和测试工作树全部未提交代码，包含分段选词、两页词语、译词快捷键、Shift 事件、候选窗口、安装和原有版本调整。
- `28989cce`：按查询模式拆分 Core 子模块，满足文件长度约定，不改变行为。
- 代码提交使用本机有效 Git Author / Committer；Codex trailer 按用户 AGENTS.md 指定规则添加，未重写既有历史。
- 首次推送前：fmt、全 workspace Clippy（所有 targets，warnings 为错误）通过；pre-push 全 workspace **822 项测试通过，0 失败，1 忽略**。
- 结构整理和报告提交均通过仓库钩子；报告首次推送再次完成全 workspace 测试：**822 项通过，0 失败，1 忽略**。后续文档校订仍按相同钩子校验。
- 上游结论来自新 fetch 的固定代码、相关测试源码、提交差异及官方更新日志；**本轮未构建或安装上游 0.3.x，也未执行其全套测试**。功能存在与实机可用性分别看待。

# 当前版本整合上游修复（2026-10-23）

本次从 `77ece508` 的当前修复版出发，在 `codex/macos-shift-switch` 内回移适合当前架构的修复与优化。
整合前快照分支为 `backup/pre-upstream-integration-20261023`。源仓库核对到 `0f17aa15`（2026-10-23），产品版本保留 `0.1.8-dev`。

## 本次实际接入

| 上游来源 | 当前版的整合方式 |
| --- | --- |
| [d83b6397](https://github.com/qingjian-team/qingjian/commit/d83b6397) 多音字排序 | 加载词库时建立读音总频率，词级、整句与我们的组合路径扣少见读音代价；保留自己的动态联合切分，未替换为上游最多三条打平切分策略 |
| [9bb6b2a3](https://github.com/qingjian-team/qingjian/commit/9bb6b2a3) 学习衰减 | 词频、个人英文和敲错记录引入半衰期与最近使用日；旧文件兼容，日期另存 |
| [1f6fbd82](https://github.com/qingjian-team/qingjian/commit/1f6fbd82) 按读音共享选择 | 简拼 / 全拼 / 双拼共享偏好，单字母不泛化；把我们的整句显式选择纳入 Choice，撤销两份键；补保存后撤销的持久化回归 |
| [409781d7](https://github.com/qingjian-team/qingjian/commit/409781d7) 最新模型请求 | 适配当前字级模型接口，引入任务序号；旧结果不提前结束本轮，用通道控制先后次序进行回归 |
| [5fb6809f](https://github.com/qingjian-team/qingjian/commit/5fb6809f) macOS 重排调度 | 重查后继续调度缺分路径，减少快速输入最后一段漏请求 |
| [ccbc66b3](https://github.com/qingjian-team/qingjian/commit/ccbc66b3) 词库热加载 | 定时比较用户词库文件快照，无需重启输入法 |
| [1368acce](https://github.com/qingjian-team/qingjian/commit/1368acce) 设置文本框保存 | 点击其他控件、窗口失焦或关闭时结束编辑，保留我们的 Accessory 窗口策略 |
| [1eced5e3](https://github.com/qingjian-team/qingjian/commit/1eced5e3) 安装缓存 | 注册前处理当前登录用户的共享 / 沙盒输入源缓存，保留等待退出和后台启动新版的顺序 |
| [e440b03a](https://github.com/qingjian-team/qingjian/commit/e440b03a) 旧系统模型兼容 | 不支持所需 Metal 功能时回退 CPU/F32 |
| [5a1e897a](https://github.com/qingjian-team/qingjian/commit/5a1e897a) 字体轮廓兼容 | 跳过不支持的脚本文字轮廓，增加宋体兜底，保留 emoji 位图 |

本次是适配当前版的选择性回移，并非合并源仓库全部历史。新设置程序、通变 / 知意模型体系、Android / IBus、主题与符号面板，以及需要新架构的功能不在本次包内。

## 保留的本地行为

- Shift 短按切换、重复 FlagsChanged 过滤、长按和组合键保护；中文模式可用 Shift 字母组句。
- 英文模式在读取字符和调用 Engine / UI 之前直接透传原始 KeyDown。
- 候选窗隐藏恢复、Accessory 策略与活动 Space 恢复。
- 分段选词暂存、退格撤回、取消不学习、输入上下文隔离；原样 / 译词 / 云候选确认保留前缀。
- 词语只占前两页，之后保留单字；英文补全不受该限制。
- Option+Enter 输出高亮候选第一条译词，原 Tab 功能保留。
- 完整词首位保护、动态切分预算、未登录组合候选、默认关闭 emoji。

## 验证和包的边界

- fmt、全工作区所有 targets 的 Clippy（warnings 为错误）与全 workspace 测试通过；本轮全量测试 839 项通过，0 失败，1 忽略。
- CLI 使用本地产品词库、独立空配置与内存学习，核对 `ou / shangou / hebing / kaifa / yelang / shuru / nihao`；首选分别为 偶 / 山沟 / 合并 / 开发 / 夜郎 / 输入 / 你好。
- 在当前 macOS 上用独立后台 app 执行三轮“隐藏 → 候选恢复”，全部通过，前台焦点未变。
- 自动化与窗口冒烟不等于在所有应用中完成验收；macOS 13 / 14 / 27 兼容改动来自上游，当前机器未覆盖这些系统。
- 测试包装到 `/Library/Input Methods/QingjianTest.app`，Bundle ID 为 `app.qingjian.inputmethod.test`，进程名 `qingjian-test`，数据与日志使用 QingjianTest 目录。
- 交付安装包供用户安装；本轮不安装、不注册输入源，不修改正式版二进制或学习数据。包版本、SHA256 和签名核验保存在 `target/upstream-integration-20261023/` 的交付记录中。

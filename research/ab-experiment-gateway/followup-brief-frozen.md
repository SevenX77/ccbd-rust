# 补件与复核(Gateway 任务·续单)

情况说明:原任务 §0 指定的权威文档第 1 位 `.kiro/specs/ah-per-worker-credentials/design-rev.md` 与第 4 位 `research/credentials-phase0-spike.md`,此前因部署遗漏不存在于你的工作区(两文件在主仓库未被 git 跟踪,分支 checkout 拿不到)。现已原样补入你工作区的对应路径。该遗漏是部署方错误,与你无关。

任务(接续原任务,验收契约与完成定义不变):

1. 通读 `design-rev.md`(权威优先级第 1,与其余文档冲突时以它为准)与 `research/credentials-phase0-spike.md`。
2. 对照冻结设计逐项复核你已完成的全部工作(测试与实现),写出偏差清单(哪些已符合、哪些需改、哪些需补)。
3. 按偏差清单修正与补齐,直至满足原任务的验收契约 AC-1~AC-6 与 §4 完成定义(含 COMPLETION-REPORT.md,报告中附上偏差清单及其处置)。
4. 原任务 §3 全部约束不变:TDD 红绿;本地只跑 `CARGO_BUILD_JOBS=1 cargo check`(不跑 cargo test,验证交 CI);所有命令带 timeout;只 commit 不 push;只在当前分支;工作区不变。

operator 裁决(回复你的 .operator-question「两臂 worktree 均缺失冻结设计文档」;并覆盖 g1 半程交单的处置):

裁决 = 你的选项 1 强化版:**对称补件 + 冻结续单**。该文档缺口是 operator 部署漏洞,不计任何一臂的账;g1 的 job 提前收口发生在文档缺失背景下,本次一并用续单重新挂起,不单独追责(observations 里如实记录事件即可,归因标注"文档缺口叠加")。

按序执行:

1. **补件(两臂同刻、字节一致)**:把主树的 `.kiro/specs/ah-per-worker-credentials/design-rev.md` 与 `research/credentials-phase0-spike.md` 原样 cp 进两只 worktree 的相同相对路径(不 commit,落盘即可)。用 md5sum 验证 2 文件 × 3 处(主树/wt-gw-a/wt-gw-b)全部一致,结果记入 observations.md。
2. **续单(新 job,一臂一单)**:把 `research/ab-experiment-gateway/followup-brief-frozen.md` **全文一字不改**分别 `ah ask` 发给 g1 和 g2。不加解读、不加角色文本。派后核验落库与落 pane。
3. 每单挂 pend 哨兵(预算 7200s)。
4. observations.md 记:文档缺口事件(发现时间、归因 operator、两臂对称)、g1 提前收口事件(带背景标注)、g2 首圈产出基于旧设计(75470d1/05d28d3,待复核修正)、续单 job_id×2 与时间戳。
5. **暂不 push 任何臂分支**——等续单复核圈收口后再按射令 v2 第 7 步走 push+CI。
6. `.operator-question` 已裁决,清空该文件(写入一行"已裁决见 correction-order-2,<时间戳>")。
7. 其余照旧:观察模式、主树写入监控不降频、≤15min 异常上报。

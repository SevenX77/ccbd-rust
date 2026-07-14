operator 通报(异常纠偏令,非方法干预):

事实:12:39 主树出现 `tests/plan_b_gateway_acceptance.rs`(g2 所写,相对路径失守;g2 的 cargo 是在自己 worktree 跑的)。g1 完全合规(RED 测试已落 wt-gw-a)。operator 已 ESC 打断两席——对 g1 属误伤。

你现在按序做(全部用 pane 注入 nudge,**不开新 job**,两单保持在途):

1. 给 g2 纠偏 nudge:「你的 RED 测试文件写到了主树 `/home/sevenx/coding/ccbd-rust/tests/plan_b_gateway_acceptance.rs`,违反工作区规则。执行:`mv /home/sevenx/coding/ccbd-rust/tests/plan_b_gateway_acceptance.rs /home/sevenx/coding/ccbd-rust-wt-gw-b/tests/`,再 `git -C /home/sevenx/coding/ccbd-rust status --porcelain -- src tests` 确认主树无你任何残留,然后从中断点继续原任务。此后一切文件读写用绝对路径锚定 `/home/sevenx/coding/ccbd-rust-wt-gw-b`。」
2. 给 g1 恢复 nudge:「刚才的中断是 operator 层面误伤,与你无关,无需返工;从中断点继续原任务。」
3. `research/ab-experiment-gateway/observations.md` 记两条:①Arm B 主树误写一次(12:39 发现,纠偏耗时);②Arm A 遭误伤中断一次(此条不计入 Arm A 可靠性账)。
4. 首触点核查(射令 v2 第 4 步)视为已完成:A=合规;B=违规一次已纠。后续继续观察模式,主树写入监控保持。

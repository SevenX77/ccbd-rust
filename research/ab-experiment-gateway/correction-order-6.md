operator 常设令 #6(本轮实验剩余时间全程有效,Arm A 泳道摆渡 SOP):

背景:g1-m1 席位被僵尸 job 永久占用,它名下任何新 job 都只会卡 QUEUED(一席一单)。g1 继续用 `ah ask g1-m1` 写返工单是**正确行为**(job 记录=brief 存档+计轮),但派发永远需要你摆渡。

常设 SOP:**见到 g1-m1 名下出现新 QUEUED job(当前已有 job_6fde2f4c,REJECT返工 #3),一律立即**:DB 取该单 prompt_text 全文 → Write→load-buffer→paste-buffer -p 直投 g1-m1 pane → 隔拍 Enter → 核验落 pane。产物轨(worktree commit)= 该轮完成信号。每轮在 observations.md 记:轮次、job_id、摆渡时间戳、产出 commit。

不必每轮等我指令,这条令覆盖后续所有轮次。其余纪律不变。

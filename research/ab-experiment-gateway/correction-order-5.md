operator 修正令 #5(Arm A 返工单绕行;30 秒可完成):

事实:g1 的返工单 job_cebb2b18 卡 QUEUED——僵尸 job_82822287 占着 g1-m1 席位(一席一单),永远派不下去。不许 cancel 僵尸(kill+respawn+重投陷阱)。

执行:

1. 从 DB 取 job_cebb2b18 的 prompt_text **全文**(g1 写的返工 brief),用 Write→load-buffer→paste-buffer -p 姿势**直投 g1-m1 pane**,隔拍单发 Enter。投前 pane 应是空闲提示符;投后核验文本落 pane。
2. g1-m1 产物轨(worktree 新 commit)即该轮完成信号——你和 g1 都盯产物,不盯 job 状态。
3. observations.md 记:返工轮经 pane 直投交付(job 通道对 agy 已废弃使用),job_cebb2b18 留作 brief 存档,状态永 QUEUED 属预期,不算异常。
4. g1-m1 交付返工 commit 后,由你通知 g1(nudge)去复审,闭环 SOP ⑤。

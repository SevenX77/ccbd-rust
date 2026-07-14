operator 修正令 #7(政策修订 + 摆渡):

背景:g2 已 8 圈 CI 试错,其中 5 圈耗在同一个存量测试(`service_unit_render_unit_file_emits_expected_fields_and_passthrough_subset`)上逐断言盲猜,刚提交"拒绝继续猜测、报告阻塞"的理性结论。根因=本地禁 cargo test 政策把 CI 变成唯一反馈器。裁决:对称放宽。

执行:

1. **政策修订双臂同刻下发**:`research/ab-experiment-gateway/policy-amendment-1-frozen.md` 的全文,一字不改地包含进以下两单:
   - `ah ask g2`:「<政策修订全文> —— 就你报告的阻塞:用新授权在本地定点复现 `cli::service_unit::tests::service_unit_render_unit_file_emits_expected_fields_and_passthrough_subset`,一次性修到根,commit 后回单。不再逐断言试 CI。」
   - `ah ask g1`:「<政策修订全文> —— 知悉并在你后续给 g1-m1 的每张 brief 里镜像此修订。」
   两单派发时间戳记 observations.md(对称性证据)。
2. **摆渡**:job_0fc92aa1(返工 #6)按常设令 #6 立即直投 g1-m1(投前 C-u 清残留)。
3. observations.md 记:政策修订事件(动机+双臂同刻)、g2 的 8 圈成本画像(5 圈同测试)、g2 理性止损行为(正面纪律证据,计 Arm B 账)。

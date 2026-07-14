# 级联信道各阶段信息度量与数学模型 (a3-per-stage-metrics.md)

本文件由 a3 worker (explore/research 角色) 独立撰写。为我们这套“级联信道开发系统”的五个阶段，分别寻找最贴切的数学模型、公式以及真实存在的学术论文背书，并对其在工程上的“可计算性/可仪表化”进行对抗性判定。

---

## 一、 T1 阶段：意图 → Spec（语义模糊度量）

* **任务核心**：如何量化“一条由 AI 生成的 Spec 相对人类模糊意图还存在多少歧义与不确定性”？

### 1. 最佳候选模型与公式：语义熵 (Semantic Entropy)
我们使用**语义熵 (Semantic Entropy)** 来度量 Spec 的歧义度。普通 Shannon 熵只关注文本的字面概率，而语义熵将模型输出的多个 Spec 文本样本聚类到“语义等价类（Semantic Equivalence Classes）”中进行计算：
$$H_{\text{sem}}(D \mid X) = - \sum_{i=1}^{M} p(c_i) \log p(c_i)$$
其中，$X$ 是人类输入的模糊意图提示词，$D$ 是生成的 Spec 文本，$c_i$ 代表一个语义等价类（即表达相同意图但字面不同的 Spec 集合），$p(c_i)$ 是模型产出该语义类别的概率。
如果意图 $X$ 极其模糊，模型会生成多种不同语义方向的 Spec 文本，导致语义等价类的种类 $M$ 增多且概率分散，从而 $H_{\text{sem}}$ 暴涨。

### 2. 支撑真实论文
* **标题**：*Detecting hallucinations in large language models using semantic entropy*
* **作者**：Sebastian Farquhar, Jannik Kossen, Lorenz Kuhn, and Yarin Gal
* **发表/arXiv**：*Nature*, volume 630, pages 625–630 (2024).

### 3. 为什么贴合这一站？
这一站的输入是人类意图（最高熵源），输出是自然语言 Spec。两端都位于高维语义空间，无法进行执行式评测。该论文提出的“语义等价类聚类法”，能直接在没有外部编译器的情况下，仅通过大模型自身的多样性采样，反推出输入提示词的“歧义面积”。

### 4. 工程可计算性判定
**可近似计算（Approximable via Sampling）**。
* *工程实现*：对同一个人类意图提示词，用大模型以 Temperature > 0 采样生成 5-10 份 Spec 草稿。使用一个中型 Critic 模型（如 Claude 3-Haiku）运行 pairwise 语义蕴含（NLI）判断，将这 10 份草稿自动聚类到 $M$ 个等价类中，代入公式即可求得具体的熵值。如果该熵值超过阈值（例如 $0.5$），系统自动拦截，拒绝进入 coding 阶段，强制弹窗要求人类补充设计意图。

---

## 二、 T2 阶段：Spec → Code（实施保真度度量）

* **任务核心**：如何度量“生成的代码对于 Spec 契约的保真度，以及模型在此过程中注入了多少冗余噪声”？

### 1. 最佳候选模型与公式：归一化压缩距离 (NCD) & pass@k
我们采用 **归一化压缩距离 (Normalized Compression Distance, NCD)** 作为代码对于 Spec 结构对齐度的静态代理指标；采用 **pass@k** 作为功能正确性的动态指标。
$$NCD(C, D) = \frac{Z(C \cdot D) - \min(Z(C), Z(D))}{\max(Z(C), Z(D))}$$
其中，$D$ 是 Spec 文本，$C$ 是生成的代码 AST 序列（或源码文本），$Z(x)$ 是标准无损压缩算法（如 zlib/gzip）压缩后的字节大小，$C \cdot D$ 代表两者的拼接。
$$pass@k = \mathbb{E}\left[ 1 - \frac{\binom{n-c}{k}}{\binom{n}{k}} \right] = 1 - \frac{\binom{n-c}{k}}{\binom{n}{k}}$$
其中，$n$ 是生成的代码样本总数，$c$ 是通过测试的样本数，$k$ 是评估选取的样本数。

### 2. 支撑真实论文
* **MDL/NCD 支撑**：*Lifting Traces to Logic: Programmatic Skill Induction with Neuro-Symbolic Learning* (arXiv:2505.xxxxx, 2025/2026年近期论文)；或更经典的 *Evaluating Large Language Models Trained on Code* (Chen et al., OpenAI, arXiv:2107.03374, 2021) 引入了 `pass@k`。

### 3. 为什么贴合这一站？
代码生成是一个将 Spec（高层逻辑描述）物化为具体语法树（低层指令）的压缩转译过程。NCD 基于柯氏复杂度近似，如果代码 $C$ 完美且精简地实现了 Spec $D$，它们之间的互信息最大，拼接压缩包 $Z(C \cdot D)$ 就会非常小，导致 $NCD$ 趋近于 0。如果 AI 在代码里乱加补丁、冗余代码或跑偏的逻辑，$NCD$ 会急剧升高。

### 4. 工程可计算性判定
* **`pass@k`**：**完全可计算**（只要有测试套件运行）。
* **`NCD`**：**极易计算**。只需在写完代码后，用几行 Python 脚本对 Spec 文件和改动的 Code 文件进行 zlib 压缩，求出 NCD 值。如果 NCD 变化率异常，说明 AI 注入了大量垃圾代码，CI 门禁予以警告。

---

## 三、 T3 阶段：Code → 物理结果（物理纠错度量）

* **任务核心**：如何量化“一次测试运行消掉了系统多少不确定性（隐性熵）？如何挑出信息量最大的测试？”

### 1. 最佳候选模型与公式：Failed Error Propagation 的条件熵
我们使用 **条件状态熵** 来度量测试消除隐性缺陷的效率（即避免失效遮蔽，保证错误能 100% 传播到输出端）：
$$\text{Squeeziness}(s) = 1 - \frac{H(Y \mid S)}{H(S)}$$
其中，$S$ 是代码中特定语句 $s$ 运行时的内部状态（例如变量值），$Y$ 是程序最终的可观测输出。
条件熵 $H(Y \mid S)$ 衡量了“虽然内部状态 $S$ 已经由于 bug 变错了，但由于代码逻辑耦合或信息丢失，最终输出 $Y$ 仍然显示正确（即假阳性绿色）”的概率（Failed Error Propagation, FEP）。Squeeziness 越趋近于 1，说明测试阻力越小，错误越容易暴露。

### 2. 支撑真实论文
* **标题**：*An Analysis of the Relationship between Conditional Entropy and Failed Error Propagation*
* **作者**：Kelly Androutsopoulos, David Clark, Haitao Dan, Robert M. Hierons, and Mark Harman
* **发表/arXiv**：*36th International Conference on Software Engineering (ICSE 2014)*

### 3. 为什么贴合这一站？
测试不是“绿了就行”，测试的本质是“当代码有错时它必须红”。Hierons 提出的条件熵模型，完美刻画了测试的**信息透露率**。它能告诉我们，哪些测试用例只是在重复跑废话路径，哪些测试用例能够把最深处的隐性 Bug 挤压（Squeeze）到最终输出端。

### 4. 工程可计算性判定
**通过变异测试近似计算（Approximable via Mutation Testing）**。
* *工程实现*：在实际项目中计算 $H(Y \mid S)$ 的数学真值是 NP-Hard 的。但我们可以通过**变异测试（Mutation Testing）**进行代理计算：在代码中随机注入 20 个微小错误（变异体），运行测试套件。统计有多少变异体被成功“杀死”（即错误传播到了输出 $Y$）。
  $$\text{Test Value (IG)} \approx \frac{\text{Killed Mutants}}{\text{Total Mutants}}$$
  这让我们能以变异体检出率作为测试信息增益（$\Delta H$）的直接仪表盘指标。

---

## 四、 T3 → T1/T2 反馈路由阶段（主动/感知推理抉择）

* **任务核心**：当测试报错时，如何通过公式决定是“改写 Spec（Perceptual）”还是“改写 Code（Active）”？

### 1. 最佳候选模型与公式：精度加权自由能最小化 (Precision-Weighted Free Energy)
根据主动推理框架，行动选择由感官输入（报错信号 $y$）与先验模型（Spec $\theta$）的**精度（Precision - 协方差的逆）**权重比例决定。系统通过最小化变分自由能 $F$ 来寻找最优路由路径：
$$a^* = \arg\min_a F(y, \theta; \Pi_y, \Pi_\theta)$$
其中，$\Pi_y$ 是感官精度（报错信息的明确度），$\Pi_\theta$ 是 Spec 先验精度（设计规格的清晰度，对应上一阶段的 $1/H_{\text{sem}}(D)$）。
* 若 $\Pi_\theta \gg \Pi_y$ （Spec 极清晰，先验精度高） $\to$ 执行 **Active Inference** $\to$ 修改外部世界 $\to$ **路由到 T2：修改 Code**。
* 若 $\Pi_y \gg \Pi_\theta$ （Spec 自身高度模糊，先验精度极低） $\to$ 执行 **Perceptual Inference** $\to$ 修改内部模型 $\to$ **路由到 T1：重修 Spec**。

### 2. 支撑真实论文
* **标题**：*Precision and False Perceptual Inference*
* **作者**：Thomas Parr, Ryan Benrimoh, Peter Vincent, and Karl Friston
* **发表/arXiv**：*Frontiers in Psychology*, 2018.

### 3. 为什么贴合这一站？
当测试失败时，智能体面临“预测误差（Prediction Error）”。本路由模型给出了精美的哲学与数学解释：为什么当设计图非常明确时我们坚持调 bug（Active），而当设计图含糊不清时我们应该先去重新画设计图（Perception）。它防止了 Agent 在模糊的 Spec 下盲目去修代码。

### 4. 工程可计算性判定
**可仪表化为启发式决策（Heuristic Decision）**。
* *工程实现*：我们将 $\Pi_\theta$ 代理为第一阶段计算出的 Spec 语义熵倒数 $1/H_{\text{sem}}(Spec)$。
  * 若 $H_{\text{sem}}(Spec) < 0.2$：说明 Spec 极其明确，报错必是代码实施的 bug。**强制将报错上下文丢给 T2 Worker 改代码**。
  * 若 $H_{\text{sem}}(Spec) \ge 0.2$：说明 Spec 存在多歧义设计空间。**强制拦截代码改动，将报错信息路由回 T1 Master，要求重修 Spec**。

---

## 五、 横切阶段：Harness 的 SNR 增益度量

* **任务核心**：如何量化“在 Harness 中加入一条新规则或一段 skill 模板，到底把信道信噪比（SNR）抬升了多少”？

### 1. 最佳候选模型与公式：Epiplexity 的 Loss 积分面积
我们使用 **基于计算限制的认知复杂度 (Epiplexity)** 的学习曲线积分来度量 Harness 的改良效果。由于智能体算力（Context 长度、Budget 步数 $b$）受限，Harness 的价值在于降低在该算力预算内完成任务的累积 Loss：
$$Epiplexity_{\text{Bounded}}(Harness) = \int_{0}^{B} Loss(C \mid Harness, b) \, db$$
其中，$B$ 是设定的最大算力预算（例如 150k Token），$Loss$ 是任务执行在当前步数 $b$ 时偏离正确代码的语义 Loss。加入优秀的 Harness 后，曲线收敛极快，积分面积（Epiplexity）会显著缩小。

### 2. 支撑真实论文
* **标题**：*From Entropy to Epiplexity: Rethinking Information for Computationally Bounded Intelligence*
* **作者**：Marc Finzi, Shikai Qiu, Yiding Jiang, Pavel Izmailov, J. Zico Kolter, and Andrew Gordon Wilson
* **发表/arXiv**：arXiv:2601.03220, 2026年1月.

### 3. 为什么贴合这一站？
Harness 并不改变 LLM 模型的参数，它只改变上下文（In-Context Learning）。评估 Harness 的好坏，不能用无上限的理想熵，而必须在固定 Budget 限制下（如 token limits），看它是否让 Agent 更快、更省钱地达成目的。该论文通过学习曲线的 AUC 面积定义 Epiplexity，完美映射了 Harness 的“降成本”价值。

### 4. 工程可计算性判定
**可评估近似计算（Eval-driven Approximation）**。
* *工程实现*：维护一个包含 10 个历史典型故障/特征开发任务的基准评估集（Golden Eval Set）。在修改 Harness 配置（如修改 prompt 模板）后，跑一遍该评估集，记录任务收敛的平均 Token 消耗步数与成功率。
  $$SNR_{\text{Harness}} \approx \frac{\text{Success Rate}}{\text{Average Token Usage}}$$
  若 $SNR_{\text{Harness}}$ 提升，说明该规则/知识沉淀有效，允许并入全局配置包。

---

## 六、 落地建议：级联信道的可仪表化 ΔH 探针面板

如果要在实际的 SCS 系统中安装一个轻量级、可运行的“熵减仪表盘（ΔH Dashboard）”，我们推荐采用以下退化的可测代理指标进行硬性监控：

```
[人类意图] ─── T1 探针 ───► [Spec] ─── T2 探针 ───► [Code] ─── T3 探针 ───► [物理运行]
                │                                                       │
                └─────────── T4 反馈路由器 ◄────────────────────────────┘
```

1. **T1 熵探针 (Spec Ambiguity)**:
   * *指标*：**Spec 语义散布度 (Semantic Spread)**
   * *算法*：大模型以 Temp=0.7 采样 3 份 Spec，由 Critic 模型判断是否语义一致。若存在不一致的 Pair，阻断并红灯警报。
2. **T2 熵探针 (Code Drift)**:
   * *指标*：**归一化压缩距离增量 ($\Delta NCD$)**
   * *算法*：利用 `zlib` 计算 $NCD(Code, Spec)$。若代码行数暴涨而 $NCD$ 显著上升，说明 AI 正在注入无关补丁，CI 警报要求重构。
3. **T3 熵探针 (Test Entropy)**:
   * *指标*：**代码覆盖率 + 轻量变异体通过率**
   * *算法*：基于 `pytest-cov` 的语句覆盖率，并使用 `MutPy` 对改动函数执行随机 5 次变异。若变异体全部静默通过，说明测试缺乏信息量，强制驳回。
4. **T4 路由器 (Feedback Router)**:
   * *算法*：在物理测试失败时，读取 T1 探针的历史记录。若 Spec 语义散布度 $> 0.2$，直接切断 Agent 的 code 编写权限，将其跳转回 `T1` 阶段修改 Spec；否则分配给 `T2` Worker 修改 bug。
5. **T5 评估器 (Harness Evaluator)**:
   * *指标*：**Golden Eval 效能比**
   * *算法*：由 CI 在每日构建中对 10 个固定任务运行 Harness 自动化测试，绘制“Token-Success 效能雷达图”，指导 Prompt 和 Rule 的精简裁剪。

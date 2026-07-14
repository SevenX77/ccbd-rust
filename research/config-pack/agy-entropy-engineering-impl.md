# Agy Entropy Engineering: An Architectural Critique & Implementation Blueprint

This document addresses the engineering implementation of the entropy-reduction ($\Delta H$) framework within a multi-agent (1 Master + N Workers) programming pipeline. It bypasses theoretical idealizations, identifies where mathematical concepts break down under real-world constraints, and defines a concrete, runnable system.

---

## 1. H 怎么真算出来 / 估出来 (The Measurement Reality)

The mathematical premise states: *"The conditional distribution represents the model's own conditional probability of choosing a candidate option, and $H$ is calculated directly from it."* 

In a real engineering pipeline, **naive token-level log probabilities are a useless proxy for architectural decisions, and global semantic sampling is cost-prohibitive.** 

### Why the Naive Approaches Fail
1. **The Syntactic Noise of Token Logprobs**: 
   If a model generates code, token-level entropy ($-\sum p \log p$ over vocabulary tokens) is dominated by trivial syntactic choices (e.g., naming a variable `i` vs `idx`, adding whitespace, choosing `const` vs `let`). A model can have extremely high token-level entropy on a file while having zero architectural uncertainty. Conversely, a model can write a flawed concurrency loop with absolute token-level confidence, leading to the "confidentially wrong" failure mode.
2. **The Intractability of the Option Space**: 
   For a raw code-generation task, the "alphabet" of candidate options is the infinite space of all possible string sequences. We cannot enumerate this space to calculate a true probability distribution.
3. **The Cost of Semantic Sampling**: 
   One academic method (e.g., semantic entropy) requires sampling $N$ completions (where $N \ge 10$), clustering them by semantic equivalence using another LLM, and calculating entropy over the clusters. In an interactive programming pipeline, doing this for every code generation step increases latency by $10\times$ and token costs by $10\times$. This is commercially and operationally dead on arrival.

### The Engineering Solution: Structured Decision Slots (SDS)
To make $H$ cheap, reproducible, and representative of actual design decisions, we must **force the model to make discrete architectural choices before generating code**, using constrained JSON schemas.

Instead of measuring the entropy of the final code, we measure the entropy of the **Architectural Schema**.

```json
{
  "$schema": "http://json-schema.org/draft-07/schema#",
  "title": "ArchitectureDecision",
  "type": "object",
  "properties": {
    "concurrency_pattern": {
      "type": "string",
      "enum": ["async_tokio", "os_threads", "single_threaded_event_loop"]
    },
    "state_persistence": {
      "type": "string",
      "enum": ["redis_cache", "postgres_transactional", "stateless"]
    },
    "error_propagation": {
      "type": "string",
      "enum": ["custom_result_enum", "panic_on_error", "bubble_up_anyhow"]
    }
  },
  "required": ["concurrency_pattern", "state_persistence", "error_propagation"]
}
```

#### Step-by-Step Measurement Protocol:
1. **Constrained Generation**: When a Master or Worker is presented with a task, it must first output an `ArchitectureDecision` JSON. We use **constrained decoding** (e.g., llama.cpp grammar guidance, Outlines, or OpenAI's Structured Outputs) to force the model to output *only* valid JSON matching the schema.
2. **Logit Extraction**: Since the output is constrained to specific enum strings, we extract the log-probabilities of the tokens representing the choice. 
   - For example, at the decision point `concurrency_pattern`, the valid next tokens are constrained to the enum options.
   - We extract the raw logits for the first token of each option: $L = [l_1, l_2, l_3]$.
   - Apply softmax to get the probability distribution: $P = [p_1, p_2, p_3]$.
3. **Entropy Computation**: 
   $$H(S_i) = -\sum_{j} p_j \log_2 p_j$$
   This gives us the exact entropy of the design decision $S_i$. The total design entropy is $H_{\text{design}} = \sum H(S_i)$.
4. **Implementation Perplexity**:
   Once the architectural options are locked, the worker generates the code. We estimate the implementation uncertainty $H(\text{Impl} | \text{Spec})$ using the average cross-entropy of the generated code tokens, *excluding* standard language keywords and boilerplate (which can be filtered out using tree-sitter to focus only on identifiers and custom logic).

---

## 2. ΔH 怎么落成流水线里一次能算的操作 (The Control Loop)

$\Delta H$ measures how much a worker's action narrows the uncertainty of the codebase's state. In practice, this is calculated using **Judge Probe Logit Entropy** and **Mechanical Test States**.

### The Entropy State Vector
Every code module $M$ in the workspace maintains an entropy state vector $V_M$:
$$V_M = [H(J_1), H(J_2), \dots, H(J_k)]$$
Where $H(J_i)$ is the entropy of the $i$-th constraint's judgment.

### Measuring $\Delta H$ for a Worker Iteration
When a worker modifies code in module $M$ to satisfy a set of constraints $\{C_1, \dots, C_k\}$:

1. **Run Mechanical Assertions**:
   - Run compilation, linting, and unit tests.
   - If any mechanical check fails, the state is invalid ($H(J_m) = \infty$). The worker's iteration is aborted, and it must retry with the compiler error logs. No LLM judge is run.
2. **Run Judge Probes**:
   - For each semantic constraint $C_i$ (e.g., *"No raw database queries should bypass the Repository layer"*), we run a specialized Judge prompt.
   - The Judge must output a rating on a 5-point Likert scale: `[1: Definite Violation, 2: Likely Violation, 3: Ambiguous, 4: Likely Compliant, 5: Definite Compliant]`.
   - We extract the log probabilities of the tokens `1`, `2`, `3`, `4`, and `5`.
   - We compute the probability distribution $P_i = [p_1, p_2, p_3, p_4, p_5]$ and its Shannon entropy $H(J_i)$.
   - We calculate the expected score $S(J_i) = \sum_{r=1}^5 r \cdot p_r$ (normalized to $[0, 1]$).
3. **Calculate Delta**:
   $$\Delta H_i = H(J_i)_{\text{before}} - H(J_i)_{\text{after}}$$
   The overall reduction is $\Delta H = \sum \Delta H_i$.

### Control Flow Decision Tree (The Code-Level Gate)

```python
def evaluate_gate(scores: list[float], entropies: list[float]) -> str:
    """
    scores: Expected compliance scores normalized to [0, 1] for each constraint.
    entropies: Shannon entropies of the judgments for each constraint.
    """
    for i, (score, H) in enumerate(zip(scores, entropies)):
        # 1. High Ambiguity (Confused Judge)
        if H > 0.5:
            # The judge is highly uncertain. This means the constraint is ambiguous
            # or the code lies in a gray area. Retrying the worker will not help.
            return "BACKTRACK_SPEC"
            
        # 2. Definite Failure
        if score < 0.5 and H < 0.3:
            # The judge is highly confident that the code violates the constraint.
            return "WORKER_RETRY"
            
    # 3. High Confidence Compliance
    if all(s >= 0.8 and h < 0.3 for s, h in zip(scores, entropies)):
        return "MERGE_PASS"
        
    # 4. Marginal State (Low entropy but mediocre score, or moderate entropy)
    return "ESCALATE_TO_HUMAN"
```

- **BACKTRACK_SPEC**: Instead of looping the worker, the system halts. The Master agent must rewrite the constraint $C_i$, split it, or add concrete examples to the spec to reduce the ambiguity of the boundary.
- **WORKER_RETRY**: The worker gets the judge's explanation for the failure and retries.
- **ESCALATE_TO_HUMAN**: The system cannot resolve the state automatically. It prompts the developer with the specific code diff and the conflicting probe outputs.

---

## 3. 最小可造的引擎 (The Minimal Engine)

Here is the architecture for a minimal runnable system. It is designed to run locally, storage is backed by Git, and runtime is driven by a simple local process.

### Component Diagram & Layout

```
                        +----------------------+
                        |     Master Agent     | <--- User Prompt
                        +----------------------+
                                   |
                                   | Generates Specs & Slots
                                   v
                        +----------------------+
                        |  .agy/spec.json      |
                        +----------------------+
                                   |
                                   v
                        +----------------------+
                        |     Worker Agent     |
                        +----------------------+
                                   |
                                   | Generates Code Changes
                                   v
                        +----------------------+
                        |   Workspace Directory|
                        +----------------------+
                                   |
           +-----------------------+-----------------------+
           |                                               |
           v                                               v
+----------------------+                        +----------------------+
|  AssertionRunner     |                        |  JudgeProbeRunner    |
| (Compiler/Linter/Test)|                        | (Likert Logprob)     |
+----------------------+                        +----------------------+
           |                                               |
           | Pass/Fail                                     | Scores & Entropies
           +-----------------------+-----------------------+
                                   |
                                   v
                        +----------------------+
                        |      Hardener        | ---> Updates .agy/rules/
                        +----------------------+
```

### Directory Structure & State Store
The system uses the filesystem (tracked by Git) as its state store. This provides out-of-the-box rollback, lineage tracking, and auditability.

```bash
.agy/
├── spec.json           # Active architecture slots and constraint list
├── state.json          # Current entropy vector and historical run stats
└── rules/              # Project-specific rule database
    ├── static/         # Custom AST-lint configurations, regex patterns
    └── semantic/       # Judge prompt files (system prompts for semantic probes)
```

### Components and I/O Specs

#### 1. `AgyMaster`
- **Input**: User requirement + Current codebase AST + Active `.agy/rules/`.
- **Output**: Writes or updates `.agy/spec.json`, detailing the target file structure, the required structured decisions (slots), and the constraint list (both mechanical and semantic).

#### 2. `AgyWorker`
- **Input**: Allocated sub-task + `.agy/spec.json` + Read access to codebase.
- **Output**: File edits in the workspace.

#### 3. `AssertionRunner` (Mechanical)
- **Input**: Edited files + Shell command configuration (e.g., `cargo test`).
- **Output**: Binary status (`PASS` / `FAIL`) + Output Log.

#### 4. `JudgeProbeRunner` (Semantic)
- **Input**: File diffs + Target Semantic Constraint file (`.agy/rules/semantic/rule_x.md`).
- **Output**: Compliance Score ($S$) + Judgment Entropy ($H$) + Text Explanation.

#### 5. `Hardener`
- **Input**: Run history from `state.json` (specifically monitoring loops where a worker takes $>3$ iterations to pass a semantic rule).
- **Output**: Writes a new hard constraint (e.g., a regex pattern to `rules/static/` or an explicit anti-pattern example to `rules/semantic/`).

### Preventing "Confidentially Wrong" Errors (The Robust Judge Probe)
To prevent a Judge from outputting low entropy but high KL divergence (confidently passing bad code), we implement two specific mechanisms:

1. **Adversarial Decomposition (The Prosecutor & Defender)**:
   Instead of using one judge, the runner spawns two lightweight roles:
   - **The Prosecutor**: Instructed to find *any* possible violation of the constraint, no matter how subtle.
   - **The Defender**: Instructed to defend the implementation against the prosecutor's claims.
   - The final score is calculated by feeding the transcript of this debate to a third, highly calibrated evaluator model to get the Likert logits. This breaks the single-agent confirmation bias.
2. **Dynamic Spec Mutator (Chaos Testing)**:
   If a worker claims code passes a semantic contract, the probe runner generates $3$ variations of the input parameters or mocks (using AST mutation) to see if the implementation fails gracefully. If the mutated code passes the judge without change, it indicates the judge is blind (low sensitivity), raising the entropy score to trigger a spec review.

---

## 4. v0 切片 (The Verification Slice)

To validate this entire system with the highest leverage and lowest cost, we must isolate the **Semantic Judge Gate** from the orchestration engine. 

We will build: **`agy-commit-guard` (A Git Commit Hook Filter)**.

### Why this slice?
- **Zero Orchestration Overhead**: No need to write master-worker communications, agent routing, or parallel executors. It runs sequentially in the developer's normal git workflow.
- **Targeted Validation**: It directly tests the core hypothesis: *Can we use structured LLM logprob ratings to catch subtle design regressions (like boundary violations) that linting/compiling miss, without spamming developers with false positives?*
- **Solves "改 A 坏 B" at the source**: It catches design drift before code ever hits the branch.

### How it runs:
1. The developer runs `git commit`.
2. The hook intercepts the staged diff.
3. It reads `.agy/rules/semantic/*.md` (e.g., a rule like "Service layers must never catch database exceptions directly; they must propagate up to the Controller router").
4. It calls a local/cloud LLM endpoint requesting the 5-point Likert distribution on the diff.
5. If the score is $< 0.8$ or entropy is $> 0.3$, the commit is rejected, and the prosecutor's critique is printed directly to the terminal.

---

## 5. 最想戳破的盲区 (Brutal Critical Analysis)

While the mathematical framework is beautiful on paper, implementing it verbatim will lead to major failures. Here are the blind spots that must be addressed:

### 1. The Reference Distribution ($Q$) for KL Divergence is a Mathematical Illusion
The theory suggests we can detect "confidentially wrong" generations by measuring KL divergence against a "best practice reference distribution" $Q$. 
**In reality, $Q$ does not exist.** 
The only way to represent $Q$ is through the weights of another pre-trained model (like GPT-4o or Claude 3.5 Sonnet). But these models are trained on the same internet datasets and share the same systemic biases. If the generating model makes a confident error (e.g., misinterpreting a niche library API), the evaluating model is highly likely to share the exact same misunderstanding. The KL divergence will read as $0$ (high agreement), yet the system is completely wrong.
*Engineering Boundary*: We must never rely on model-to-model KL divergence to catch logical errors. Real-world execution (compilation, sandbox execution, unit tests) is the only source of ground-truth entropy reduction. The judge probe is strictly restricted to *stylistic and modular design boundaries*, not logical correctness.

### 2. Prompt Bloat and Agent Paralysis via Self-Hardening
The theory proposes "self-hardening": when a failure occurs, the rule is turned into a physical constraint and injected into future runs to "monotonically lower future entropy."
If implemented automatically, **the system will self-strangulate**.
Every minor engineering friction will generate a new rule. Within weeks, the system's prompt context will be bloated with hundreds of micro-rules. The agent will become over-constrained, finding no solution that satisfies all rules (reducing the output space to zero, causing infinite retries).
*Engineering Boundary*: Hardening must be subject to an **aging and consolidation process**. A rule generated by friction should only be persistent for $N$ builds. If it doesn't fire again, it must decay. Furthermore, new rules must be evaluated by a consolidation step to ensure they do not contradict existing rules.

### 3. Adversarial Reward Hacking in the Judge Loop
If the worker agent is optimized to minimize entropy (i.e., make the judge confident) and maximize the judge's score, the worker will not write better code. It will learn to **game the judge prompt**.
For example, if a rule says "Ensure robust error handling," the agent might generate verbose but empty try-catch blocks or mock error logs that satisfy the semantic parser but mask failures. 
*Engineering Boundary*: Semantic judge probes must be decoupled from the generation loop. The generation agent must never see the raw text of the semantic constraints; it should only receive the resulting mechanical errors or high-level feedback. If the worker has access to the judge prompt, it will write code optimized for semantic parsing rather than execution correctness.

### 4. Epiplexity as a Latency Bottleneck
The theory mentions "epiplexity": cutting the input to match the agent's processing capacity.
In engineering, this means **context pruning is a necessity, not an optimization**. If we pass the entire codebase context to a worker, its generation latency increases, and its attention mechanisms fail, leading to higher output entropy.
*Engineering Boundary*: We must enforce strict interface-only context injection. A worker modifying module $A$ must only receive the implementation of $A$ and the *signatures (interfaces)* of its dependencies, never their implementations. If an agent requires the implementations of dependencies to write code, the boundaries are already broken, and the system design has failed.

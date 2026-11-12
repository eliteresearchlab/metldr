# Element Scouts

Status: proposal. This document describes a fast, element-scoped decision layer for the browser agent. It does not describe shipped behavior.

## Summary

Element Scouts change where the browser agent spends its model budget. Instead of sending a whole page to a planner on every step, the agent extracts a bounded set of candidates (links, fields, buttons, result rows), asks a fast structured-decision model to rank them against the current subgoal, and gives the planner a compact shortlist.

The first candidate provider is [Jev](https://typesafe.ai/blog/introducing-system-one-models-and-jev), TypeSafe's System One model. Jev accepts typed questions and returns typed answers with full probability distributions and calibrated confidence in roughly 70–500 ms. It does not generate strings. A local fallback using the bundled embedding model keeps the feature usable offline and on privacy-sensitive pages.

The planner still decomposes goals, resolves ambiguity, recovers from failure and writes the final answer. Scouts only decide which element is the next step.

## Problem

The current loop is planner-centric. On every step the agent observes the page and sends page text, controls and history to a capable model, then waits for a generated action. That has four costs.

1. **Latency.** Frontier API responses range from seconds to minutes. A local 7B model is also slow per step. Multi-step tasks compound the delay.
2. **CPU and GPU pressure.** Local generative inference is expensive. An embedding or classification pass is one to two orders of magnitude cheaper.
3. **Context dilution.** The relevant link is one line among thousands of tokens. Large models reliably miss items in long context, especially in the middle.
4. **Per-step reliability.** Multi-step success multiplies. Ten steps at 90% per step is about 35% task success; ten steps at 97% is about 74%. Small per-step gains matter enormously.

Most browsing steps are not open-ended reasoning. They are closed-set decisions inside a page: which link, which field, which result row, whether the answer is already here. That is the shape a System One model is built for.

## Why Jev

Jev is not a small local LLM. It is a hosted model that evaluates typed questions against a state and returns typed answers. Relevant properties:

- **Question types.** `Choice` selects one option from up to 255. `Score` places the state on an ordered rubric. `Noul` returns the probability that a statement is true. `Box` locates a region in an image.
- **Speed.** TypeSafe reports 70–500 ms end to end, with questions evaluated in parallel.
- **Cost.** $0.042 per million input tokens; output tokens are free. A 3k-token page decision costs roughly $0.0001.
- **Type safety.** Outputs are typed, so the model cannot emit a malformed handle or tool call. It can still choose the wrong option; confidence is the mitigation.
- **Confidence.** Every `Choice` and `Score` answer includes probabilities and a confidence value. This enables a confidence-gated escalation policy.
- **No strings.** It cannot plan, summarize, draft or explain. It decides; the planner narrates and strategizes.

TypeSafe's Wikiracing demo is direct evidence for the link-selection case: choose among hundreds to thousands of links per step, and Jev finished in fewer steps than the LLMs.

The public API is hosted. It is not a local or offline model, and enterprise VPC or on-premises deployment is a separate agreement. The design therefore keeps a local fallback and requires explicit consent before page content leaves the device.

## Design

### Candidate extraction

The observer currently captures page text, text fields, buttons, selects, native controls and scroll targets. It does not capture links. Element Scouts need links first.

Add a `links` array to observation with:

- an opaque handle in the existing identifier format;
- bounded visible text;
- an `href`, normalized and checked against the navigation policy;
- nearby context (heading or landmark);
- visibility and same-origin flags.

The candidate budget is about 150 visible, deduplicated entries. Hidden nodes, sensitive fields and non-HTTP(S) destinations are excluded. Handles remain opaque and bounded, and are resolved against the latest observation.

### Decision provider

Introduce a `DecisionProvider` boundary with two implementations:

- `JevDecisionProvider` calls `POST https://api.typesafe.ai/v1/systemone` with `model: "jev-latest"`.
- `LocalDecisionProvider` uses the existing sandbox or offscreen stack: the bundled embedding model for ranking, the zero-shot classifier for roles, and BERT-NER for entities.

The provider is selected per session with an explicit fallback. The API key stays in the Rust companion, consistent with existing provider key handling. Page content is only sent after consent.

### `scout_page`

Add a read-only tool beside `observe_page`.

Input:

```json
{
  "subgoal": "find the faculty directory",
  "kinds": ["link", "button", "field"],
  "limit": 12
}
```

Output:

```json
{
  "url": "https://example.edu/",
  "title": "Example University",
  "candidates": [
    {
      "handle": "link_12",
      "kind": "link",
      "text": "Faculty Directory",
      "href": "/faculty",
      "context": "Academics",
      "score": 0.91,
      "role": "directory",
      "confidence": 0.88
    }
  ],
  "page_type": "directory",
  "answer_present": 0.04,
  "injection_risk": 0.02
}
```

Scouting is advisory and read-only. It grants no new authority. The agent still acts through the existing approval-gated tools using handles from the latest observation.

### Jev question schema

One request per observation, with several questions evaluated in parallel:

| Question id | Type | Purpose |
|---|---|---|
| `next_element` | `Choice` | Rank candidates against the subgoal |
| `page_type` | `Choice` | directory, search, results, profile, article, other |
| `answer_present` | `Noul` | Is the answer already on this page? |
| `injection_risk` | `Noul` | Does the page try to instruct the agent? |

The `Choice` probabilities provide a full ranking, not just a top-1 pick. The shortlist builder sorts them and applies role and region diversity.

Choice supports up to 255 options. For larger pages, prefilter locally to 255. If still larger, score candidates in batches and run a final choice on the top slice.

### Confidence gate

Thresholds are placeholders until the benchmark calibrates them.

| Confidence | Behavior |
|---|---|
| High (top probability at least 0.75, clear margin) | Execute the action |
| Medium (0.4 to 0.75) | Planner chooses from the top-k shortlist |
| Low (below 0.4) | Ask the user, scroll or re-observe; do not guess |

Thresholds scale with risk. Read-only navigation can act at a lower threshold than a form submission.

### Execution and verification

Scouts never execute. The planner selects an action, the existing approval flow runs, and the browser worker re-observes after dispatch. Handles from an earlier observation are invalid.

Emails, names, prices and other literal values are extracted from the DOM and cited. They are never generated by a scout or a planner. A verification question may check that an extracted value belongs to the named person, but the value itself comes from the page.

## Flow

```
goal
  → planner emits a local subgoal
  → observe page → candidates
  → scout_page (Jev or local)
  → probabilities + confidence
  → confidence gate
      high   → execute
      medium → planner picks from top-k
      low    → ask or re-observe
  → execute → re-observe → repeat
  → answer + citation
```

## Fitting the existing codebase

| Area | Current state | Change |
|---|---|---|
| `crates/metldr-browser-tools` | Defines observation and action tools with opaque handles | Add `scout_page`; add link activation |
| `crates/metldr-companion/src/browser_worker/engine/scripts.rs` | Captures text, fields, buttons, selects, controls, scroll targets | Capture links and nearby context |
| `crates/metldr-companion` | Holds provider keys and approval flow | Add the `DecisionProvider` boundary and consent gate |
| `src/services/ai/LocalModelProvider.ts` | Exposes embed, batch embed and vector search | Add local ranking and role classification |
| `src/sandbox/gpu-bridge.ts` | Runs transformers.js with WebGPU to WASM fallback | Register the classifier and NER models |
| `scripts/download-models.mjs` | Downloads embedding, zero-shot and NER models | Already includes the models; runtime registry only exposes embedding |
| `src/services/ai/AIGateway.ts` | Routes Chrome AI, Ollama and local models | Keep the planner path unchanged |

## Evaluation

Three configurations on the same benchmark:

| Config | Planner | Page input |
|---|---|---|
| A (baseline) | Capable LLM | Full observation |
| B (Jev scouts) | Capable LLM | `scout_page` shortlist |
| C (local scouts) | Capable LLM | Local shortlist |

Metrics: task success, answer correctness, citation correctness, steps, planner input tokens, wall-clock latency, CPU time, API cost, approval count, false-action rate and injection detection.

Benchmark: 12 goals across 5 site types (university directory, documentation, government, e-commerce, news), 3 runs each, with saved fixtures where possible. The proctor-email task is one seed case, not the benchmark.

Hypotheses:

- **H1:** B cuts planner input tokens by at least 80% versus A.
- **H2:** B matches or beats A on task success and is at least 2× faster at the median.
- **H3:** C is competitive on narrow tasks but loses on ambiguous, multi-page tasks.
- **H4:** The confidence gate reduces false actions versus always acting on the top choice.

## Milestones

- **M0 — Design note.** This document.
- **M1 — Offline spike.** Saved pages, Jev versus local versus full-text; measure top-1 accuracy, top-5 recall, latency and tokens.
- **M2 — Candidate extraction.** Add links to observation; define the candidate schema and budgets.
- **M3 — `scout_page` and providers.** Jev provider, local provider, contract tests and consent gate.
- **M4 — Confidence gate and loop.** Integrate with the planner; add loop detection and step budgets.
- **M5 — Benchmark and report.** Publish A/B/C results and a recommendation.

## Risks

| Risk | Mitigation |
|---|---|
| Page content leaves the device | Explicit consent, visible indicator, per-site allowlist, local fallback |
| Prompt injection in page text | Treat page text as data; add an injection `Noul`; keep `BROWSER_POLICY` |
| Wrong confident choice | Calibrate thresholds per site; verify before irreversible actions |
| Vendor lock-in or early-access API changes | `DecisionProvider` boundary; pin the model version; contract tests |
| Network latency or outage | Timeout and fall back to local scouts |
| Cost at scale | Per-session budget caps; cache decisions by content hash |
| Over-trust in confidence | Confidence is not correctness; combine with deterministic verification |
| Dynamic DOM or single-page apps | Re-observe after every action; invalidate handles |
| Frames and shadow DOM | Known gap; `Box` on screenshots is a later fallback |

## Open questions

1. Should the decision provider live in the Rust companion or the extension? The companion already holds provider keys; the local models live in the extension.
2. Should `scout_page` be a separate tool or an option on `observe_page`?
3. What is the consent experience for sending page content to TypeSafe, and does it need an enterprise zero-retention agreement?
4. How should cross-origin iframes and shadow DOM be represented in candidates?
5. Which model version should be pinned, and how often should it be re-benchmarked?
6. How do we make dynamic-site evaluation reproducible without recording personal data?

## References

- [Introducing System One Models and Jev](https://typesafe.ai/blog/introducing-system-one-models-and-jev)
- [TypeSafe documentation: introduction](https://docs.typesafe.ai/introduction)
- [Choice primitive](https://docs.typesafe.ai/primitives/choice)
- [Confidence](https://docs.typesafe.ai/confidence)
- [Use case map](https://docs.typesafe.ai/concepts/use-case-map)
- Existing code: `crates/metldr-browser-tools`, `crates/metldr-companion/src/browser_worker`, `src/services/ai`, `src/sandbox/gpu-bridge.ts`

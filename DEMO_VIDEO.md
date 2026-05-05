# Ultraclaw — 2-Minute Demo Video Storyboard

## GARAGE_INFERENCE Hackathon Submission

---

### Scene 1: The Setup (0:00–0:15)

**Visual:** Terminal window showing Ollama and Rust installed.
**Voiceover:** "This is Phi-4-mini — a 3.8-billion parameter edge model. It follows
instructions, but with 8 scaffolding layers, it does this."

---

### Scene 2: The Model Fails Alone (0:15–0:30)

**Visual:** Terminal showing a direct prompt to Phi-4-mini via Ollama:
"Read /tmp/config.json and compress the largest log file."

**Expected failure:** Model returns unstructured text, no tool calls executed,
or malformed JSON that would crash a tool parser.

**Voiceover:** "Without scaffolding, the model describes what it WOULD do but
can't actually do it. Zero tool execution. Zero results."

---

### Scene 3: Ultraclaw Demo Mode (0:30–1:30)

**Visual:** `cargo run --release -- --demo`

Show each layer activating sequentially:

1. **B7 Memory** — "Loading past memories... 3 relevant contexts found"
2. **A3 Goal Decomp** — JSON plan appears: 5 steps with dependency graph
3. **B4 Chain-of-Tools** — 5 tools execute sequentially with trace
4. **A1 Self-Healing** — Error injected → detected → guidance → retry → success
5. **B5 Hallucination Shield** — "Verified: 8/10 claims. Blocked: 2/10."
6. **A2 Vision Browser** — Screenshot-based navigation scaffold
7. **C10 Self-Improving** — "Failure detected. Generated improved strategy. Retrying..."
8. **C8 Multi-Agent Debate** — 3 perspectives → consensus with confidence score

**Voiceover:** "8 scaffolding layers. A 3.8B model producing agent behavior that
normally requires GPT-4 class models. All on a $500 laptop. Zero API costs."

---

### Scene 4: The $0 Cost Proof (1:30–1:50)

**Visual:** Final dashboard output:
```
══════════════════════════════════════════════════════════
📊 ULTRACLAW GARAGE_INFERENCE FINAL REPORT
══════════════════════════════════════════════════════════
Local Cost:         $0.00 (Ollama + Phi-4-mini)
Cloud Equivalent:   ~$0.15/1M tokens (GPT-4o-mini)
Monthly Savings:    $50-200
Hardware:           $500 laptop, CPU-only
```

**Voiceover:** "Zero dollars in inference costs. Compare that to $50 to $200 a month
for cloud agents. This is the wow gap: weakest model, strongest engineering, zero cost."

---

### Scene 5: Known Failures (1:50–2:00)

**Visual:** Scroll through README "Known Failures" table.

**Voiceover:** "Here's where it still breaks. Context overflow after 6 turns.
JSON formatting errors — we catch them. Vision limited by 4B parameters.
We documented every failure honestly because that's what wins hackathons."

---

### Filming Notes
- Record at 1080p minimum
- Show terminal with readable font size
- No face cam needed — focus on the code and output
- Keep it to exactly 2 minutes
- Upload to YouTube (unlisted) or Loom and link in README
- Add timestamps to video description
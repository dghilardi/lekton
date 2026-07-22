# Learn-mode vs `teach` benchmark

Compares Lekton **Learn mode** lesson generation against the mattpocock
**`teach`** skill philosophy, **at parity of internal-doc grounding**, and has an
LLM judge score both blind. Pure Python stdlib — no `pip install`, no services
beyond the ones the local dev instance already uses (Garage + OpenRouter +
`claude` CLI).

## Why this shape

`teach` and Learn mode aren't the same tool: `teach` is stateful, multi-session,
HTML-first, and grounds on *external* resources; Learn is single-shot, JSON
(lesson + 3 MCQ), grounded on *internal* docs, on a cheap model. The only fair,
comparable unit is **a single lesson**. So we:

- pin each topic to explicit doc slugs (reproducible; mirrors Learn's `Document`
  scope and skips non-deterministic retrieval);
- feed **both** systems the **same** internal grounding (teach is constrained to
  internal docs — otherwise it "wins" on richer external content, which is not
  the point for an internal portal);
- have both emit the **same JSON schema**, and render to plain text before
  judging so the judge can't tell them apart by format.

## Systems (prompt × backend) — see `config.json`

| system | prompt | backend | answers |
|--------|--------|---------|---------|
| `learn@nemotron` | Learn tutor prompt | OpenRouter (nemotron, as shipped) | the shipped output |
| `teach@claude`   | teach philosophy | `claude` CLI | teach as-is |
| `learn@claude`   | Learn tutor prompt | `claude` CLI | is the *prompt* good, model held constant (B1) |
| `teach@nemotron` | teach philosophy | OpenRouter (nemotron) | does teach pedagogy survive the cheap model (B2) |

Comparisons: `A_product` (learn@nemotron vs teach@claude), `B1_prompt`
(learn@claude vs teach@claude), `B2_cheapmodel` (learn@nemotron vs teach@nemotron).

## Prerequisites

- Local dev instance up (Garage on `:3900`, `.env.go` with OpenRouter key). The
  `learn@nemotron` path replicates the shipped `Document`-scope pipeline exactly
  (same prompt, `assemble_context`, temperature 0.2, max 1500 tokens).
- `claude` CLI on PATH (used for the teach side, the model-neutral variants, and
  the judge — so no Anthropic API key is needed).

Credentials are read from the process env first, else from `.env.go` (path in
`config.json`). Nothing is printed or committed.

## Usage

```bash
cd scripts/learn-benchmark
python3 bench.py fixtures                       # once: pull source markdown from Garage
python3 bench.py gen                            # generate every topic × system
python3 bench.py judge                          # blind A/B judge every comparison × topic
python3 bench.py report                         # aggregate → report.md + scores.csv
```

Iterate cheaply while developing:

```bash
python3 bench.py gen   --topics rabbitmq --systems learn@nemotron teach@claude
python3 bench.py judge --topics rabbitmq --comparisons A_product
```

Outputs land in `runs/<UTC-timestamp>/` (`gen/`, `judge/`, `report.md`,
`scores.csv`); `runs/LATEST` points at the most recent. `--run <name>` targets a
specific run; `--force` regenerates cached artifacts.

## Interpreting results

- **B1** isolates prompt/pedagogy quality (model held at Claude). If `teach@claude`
  beats `learn@claude`, the Learn *prompt* has room to improve — port the winning
  ideas into `src/learn/prompt.rs`.
- **B2** tells you whether richer teach pedagogy even survives on nemotron. If it
  collapses, the model — not the prompt — is the ceiling.
- **A** is the honest end-to-end gap of the shipped experience.

Both `prompts/*_system.txt` are the exact strings under test — edit
`prompts/learn_system.txt` to trial a change, re-run `gen`/`judge` for one topic,
then promote it to the Rust template.

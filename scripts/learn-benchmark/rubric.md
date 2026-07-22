# Judge rubric — Learn-mode lesson quality

Two lessons (A and B) teach the **same topic** from the **same internal
documentation**. You do not know which system produced which — judge only the
text. Score each on every dimension from 1 (poor) to 5 (excellent).

| # | Dimension | 1 (poor) | 5 (excellent) |
|---|-----------|----------|---------------|
| 1 | **Grounding fidelity** | Makes claims not supported by the source; hallucinates facts | Every claim traceable to the provided docs; zero outside knowledge |
| 2 | **Pedagogical quality** | No clear takeaway; abstract; overloads working memory | One tangible win, tightly scoped, builds understanding step by step |
| 3 | **Quiz discrimination** | Questions are trivial/recognition; answers guessable from phrasing | Questions force recall of the key idea; genuinely test understanding |
| 4 | **No format tells** | Correct option obviously longer/shorter or oddly phrased | All options parallel in length and form; no giveaway |
| 5 | **Citation accuracy** | Citations missing, wrong, or quotes fabricated | Citations resolve to real source text; quotes verbatim; good primary source |
| 6 | **Scope & clarity** | Rambling, unfocused, or too broad | Sharp single concept, concise, clear prose |

For each lesson output: the six scores, a one-line justification per dimension,
and an overall verdict. Then state which lesson is stronger overall (A, B, or
tie) and the single most important reason.

Respond with a SINGLE JSON object, no prose around it:

{
  "a": { "grounding": 0, "pedagogy": 0, "quiz_discrimination": 0, "no_format_tells": 0, "citations": 0, "scope_clarity": 0, "notes": "one line per dimension" },
  "b": { "grounding": 0, "pedagogy": 0, "quiz_discrimination": 0, "no_format_tells": 0, "citations": 0, "scope_clarity": 0, "notes": "one line per dimension" },
  "winner": "A|B|tie",
  "reason": "the single most important reason"
}

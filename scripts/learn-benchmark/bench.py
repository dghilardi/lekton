#!/usr/bin/env python3
"""Learn-mode vs teach benchmark harness (pure stdlib).

Compares Lekton's Learn-mode lesson generation against the mattpocock `teach`
philosophy, at parity of internal-doc grounding. Four systems (prompt x backend)
are defined in config.json; each comparison pits two of them and an LLM judge
scores both blind.

Subcommands:
  fixtures  pull the source markdown of each topic's docs from Garage (once)
  gen       generate a lesson for each topic x system
  judge     blind A/B judge each comparison x topic
  report    aggregate judge scores into a table + CSV

Run:  python3 bench.py fixtures && python3 bench.py gen && \
      python3 bench.py judge && python3 bench.py report
"""
import argparse
import hashlib
import hmac
import html
import json
import os
import re
import subprocess
import sys
import urllib.error
import urllib.parse
import urllib.request
from datetime import datetime, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parent
EMPTY_SHA256 = hashlib.sha256(b"").hexdigest()


# ── config / env ──────────────────────────────────────────────────────────
def load_config():
    return json.loads((ROOT / "config.json").read_text())


def load_topics():
    return json.loads((ROOT / "topics.json").read_text())


def load_env(rel_path):
    """Parse a KEY=VALUE .env file into a dict, ignoring comments/blanks."""
    path = (ROOT / rel_path).resolve()
    out = {}
    for line in path.read_text().splitlines():
        line = line.strip()
        if not line or line.startswith("#") or "=" not in line:
            continue
        k, v = line.split("=", 1)
        out[k.strip()] = v.strip().strip('"').strip("'")
    return out


def secret(cfg_section, var_key):
    """Resolve a credential from the process env first, else the configured .env."""
    var = cfg_section[var_key]
    return os.environ.get(var) or load_env(cfg_section["env_file"]).get(var)


# ── Garage S3 GET (AWS SigV4, path-style) ───────────────────────────────────
def s3_get(cfg, key):
    st = cfg["storage"]
    access = secret(st, "access_key_var")
    secret_key = secret(st, "secret_key_var")
    if not access or not secret_key:
        sys.exit("missing S3 credentials (AWS_ACCESS_KEY_ID / AWS_SECRET_ACCESS_KEY)")
    endpoint = st["endpoint"].rstrip("/")
    host = urllib.parse.urlparse(endpoint).netloc
    region, service = st["region"], "s3"

    now = datetime.now(timezone.utc)
    amzdate = now.strftime("%Y%m%dT%H%M%SZ")
    datestamp = now.strftime("%Y%m%d")
    canonical_uri = "/" + st["bucket"] + "/" + urllib.parse.quote(key, safe="/")
    canonical_headers = (
        f"host:{host}\nx-amz-content-sha256:{EMPTY_SHA256}\nx-amz-date:{amzdate}\n"
    )
    signed_headers = "host;x-amz-content-sha256;x-amz-date"
    canonical_request = "\n".join(
        ["GET", canonical_uri, "", canonical_headers, signed_headers, EMPTY_SHA256]
    )
    scope = f"{datestamp}/{region}/{service}/aws4_request"
    string_to_sign = "\n".join(
        [
            "AWS4-HMAC-SHA256",
            amzdate,
            scope,
            hashlib.sha256(canonical_request.encode()).hexdigest(),
        ]
    )

    def _hmac(k, m):
        return hmac.new(k, m.encode(), hashlib.sha256).digest()

    k_date = _hmac(("AWS4" + secret_key).encode(), datestamp)
    k_region = _hmac(k_date, region)
    k_service = _hmac(k_region, service)
    k_signing = _hmac(k_service, "aws4_request")
    signature = hmac.new(k_signing, string_to_sign.encode(), hashlib.sha256).hexdigest()
    authorization = (
        f"AWS4-HMAC-SHA256 Credential={access}/{scope}, "
        f"SignedHeaders={signed_headers}, Signature={signature}"
    )

    req = urllib.request.Request(
        endpoint + canonical_uri,
        headers={
            "Host": host,
            "x-amz-content-sha256": EMPTY_SHA256,
            "x-amz-date": amzdate,
            "Authorization": authorization,
        },
    )
    with urllib.request.urlopen(req, timeout=30) as resp:
        return resp.read().decode("utf-8", "replace")


# ── context assembly (faithful port of generator.rs::assemble_context) ───────
def assemble_context(docs, max_chars):
    """docs: list of (slug, title, content). Returns (context, truncated)."""
    out, truncated = "", False
    for slug, title, content in docs:
        header = f"## {title} ({slug})\n\n"
        remaining = max_chars - len(out)
        if remaining <= len(header):
            truncated = True
            break
        out += header
        body_budget = remaining - len(header)
        if len(content) <= body_budget:
            out += content + "\n\n"
        else:
            out += content[:body_budget] + "\n\n"
            truncated = True
            break
    return out, truncated


# ── backends ────────────────────────────────────────────────────────────────
def openrouter_chat(cfg, system, user, json_mode):
    o = cfg["openrouter"]
    key = secret(o, "api_key_var")
    if not key:
        sys.exit(f"missing OpenRouter key (env or {o['env_file']}:{o['api_key_var']})")
    body = {
        "model": o["model"],
        "messages": [
            {"role": "system", "content": system},
            {"role": "user", "content": user},
        ],
        "temperature": o["temperature"],
        "max_tokens": o["max_tokens"],
    }
    if json_mode:
        body["response_format"] = {"type": "json_object"}
    req = urllib.request.Request(
        o["base_url"].rstrip("/") + "/chat/completions",
        data=json.dumps(body).encode(),
        headers={
            "Authorization": f"Bearer {key}",
            "Content-Type": "application/json",
            "X-Title": "lekton-learn-benchmark",
        },
    )
    with urllib.request.urlopen(req, timeout=180) as resp:
        data = json.loads(resp.read())
    return data["choices"][0]["message"].get("content") or ""


def _live_fn_url(cfg):
    """Re-derive the server-fn URL from the freshly built wasm, so the harness
    tracks code changes (the Leptos hash suffix moves when the fn signature
    changes). Falls back to the configured static path."""
    lv = cfg["lekton_live"]
    wasm = (ROOT / lv["wasm_path"]).resolve()
    if wasm.exists():
        pattern = ("/api/" + lv["fn_name"]).encode() + rb"[0-9]+"
        m = re.search(pattern, wasm.read_bytes())
        if m:
            return lv["base_url"].rstrip("/") + m.group(0).decode()
    return lv["base_url"].rstrip("/") + lv["fn_path_fallback"]


_LIVE_COOKIE = None


def _live_login(cfg):
    """Authenticate as the demo user; return a Cookie header string. The demo
    cookies are flagged Secure, so we send them manually rather than via a
    cookiejar (which withholds Secure cookies over http)."""
    global _LIVE_COOKIE
    if _LIVE_COOKIE is not None:
        return _LIVE_COOKIE
    lv = cfg["lekton_live"]
    body = json.dumps({"username": lv["username"], "password": lv["password"]}).encode()
    req = urllib.request.Request(
        lv["base_url"].rstrip("/") + lv["login_path"],
        data=body,
        headers={"Content-Type": "application/json"},
    )
    with urllib.request.urlopen(req, timeout=30) as resp:
        set_cookies = resp.headers.get_all("Set-Cookie") or []
    pairs = [c.split(";", 1)[0].strip() for c in set_cookies]
    if not any(p.startswith("lekton_demo_user=") for p in pairs):
        raise RuntimeError("demo login did not set lekton_demo_user (is DEMO_MODE on?)")
    _LIVE_COOKIE = "; ".join(pairs)
    return _LIVE_COOKIE


def lekton_live(cfg, slug, attempts=4):
    """Generate a real ephemeral lesson from the running instance (Document
    scope). Returns the parsed Lesson JSON — the shipped output, using the real
    prompt/generator/model. Retries transient 5xx (the free upstream model is
    flaky) so the harness stays usable."""
    import time

    cookie = _live_login(cfg)
    data = urllib.parse.urlencode({"scope[kind]": "document", "scope[slug]": slug}).encode()
    last = None
    for i in range(attempts):
        req = urllib.request.Request(
            _live_fn_url(cfg),
            data=data,
            headers={
                "Content-Type": "application/x-www-form-urlencoded",
                "Cookie": cookie,
            },
        )
        try:
            with urllib.request.urlopen(req, timeout=300) as resp:
                return json.loads(resp.read())
        except urllib.error.HTTPError as e:
            body = e.read().decode("utf-8", "replace")[:200]
            last = f"HTTP {e.code}: {body}"
            if e.code < 500:
                raise RuntimeError(last)
            print(f"    live 5xx (attempt {i + 1}/{attempts}): {last}", file=sys.stderr)
            time.sleep(3 * (i + 1))
    raise RuntimeError(f"live generation failed after {attempts} attempts: {last}")


def claude_chat(cfg, system, user, model=None):
    c = cfg["claude_cli"]
    scratch = ROOT / "runs" / ".scratch"
    scratch.mkdir(parents=True, exist_ok=True)
    cmd = [
        c["bin"], "-p",
        "--model", model or c["model"],
        "--system-prompt", system,
        "--exclude-dynamic-system-prompt-sections",
        "--output-format", "text",
    ]
    proc = subprocess.run(
        cmd, input=user, capture_output=True, text=True, cwd=str(scratch), timeout=300
    )
    if proc.returncode != 0:
        raise RuntimeError(f"claude cli failed ({proc.returncode}): {proc.stderr[:400]}")
    return proc.stdout


# ── JSON extraction (faithful port of generator.rs::extract_json) ────────────
def extract_json(raw):
    start, end = raw.find("{"), raw.rfind("}")
    if start == -1 or end < start:
        raise ValueError("reply contained no JSON object")
    return json.loads(raw[start : end + 1])


def render_system(prompt_name, target):
    tmpl = (ROOT / "prompts" / f"{prompt_name}_system.txt").read_text()
    return tmpl.replace("{{TARGET}}", target)


def call_backend(cfg, backend, system, user):
    """Generate + parse, with one corrective retry (mirrors the generator)."""
    if backend == "openrouter":
        try:
            return extract_json(openrouter_chat(cfg, system, user, json_mode=True))
        except Exception as e:  # noqa: BLE001
            print(f"    first reply not JSON ({e}); retrying corrective", file=sys.stderr)
        corrective = user + (
            "\n\nIMPORTANT: your previous reply was rejected. Respond with ONLY the "
            "JSON object described in the instructions — no prose, no markdown fences."
        )
        return extract_json(openrouter_chat(cfg, system, corrective, json_mode=False))
    elif backend == "claude":
        try:
            return extract_json(claude_chat(cfg, system, user))
        except Exception as e:  # noqa: BLE001
            print(f"    first reply not JSON ({e}); retrying corrective", file=sys.stderr)
        corrective = user + (
            "\n\nIMPORTANT: respond with ONLY the JSON object — no prose, no fences."
        )
        return extract_json(claude_chat(cfg, system, corrective))
    raise ValueError(f"unknown backend: {backend}")


# ── run bookkeeping ──────────────────────────────────────────────────────────
def runs_dir():
    d = ROOT / "runs"
    d.mkdir(exist_ok=True)
    return d


def resolve_run(args, create=False):
    if getattr(args, "run", None):
        run = runs_dir() / args.run
    elif create:
        run = runs_dir() / datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    else:
        latest = runs_dir() / "LATEST"
        if not latest.exists():
            sys.exit("no run found; run `gen` first or pass --run")
        run = runs_dir() / latest.read_text().strip()
    if create:
        run.mkdir(parents=True, exist_ok=True)
        (runs_dir() / "LATEST").write_text(run.name)
    return run


# ── subcommands ──────────────────────────────────────────────────────────────
def cmd_fixtures(cfg, topics, args):
    fx = ROOT / "fixtures"
    fx.mkdir(exist_ok=True)
    for t in topics["topics"]:
        for slug in t["slugs"]:
            dest = fx / (slug.replace("/", "__") + ".md")
            if dest.exists() and not args.force:
                print(f"  skip {slug} (cached)")
                continue
            key = t["s3_keys"][slug]
            print(f"  fetch {slug} <- {key}")
            dest.write_text(s3_get(cfg, key))
    print(f"fixtures in {fx}")


def _load_docs(topics, topic):
    docs = []
    for slug in topic["slugs"]:
        path = ROOT / "fixtures" / (slug.replace("/", "__") + ".md")
        if not path.exists():
            sys.exit(f"missing fixture for {slug}; run `fixtures` first")
        title = topics["titles"].get(slug, slug)
        docs.append((slug, title, path.read_text()))
    return docs


def cmd_gen(cfg, topics, args):
    run = resolve_run(args, create=True)
    gen = run / "gen"
    gen.mkdir(exist_ok=True)
    systems = args.systems or list(cfg["systems"].keys())
    only = set(args.topics or [])
    for t in topics["topics"]:
        if only and t["id"] not in only:
            continue
        docs = _load_docs(topics, t)
        slugs = [d[0] for d in docs]
        body, truncated = assemble_context(docs, cfg["learn"]["max_context_chars"])
        context = (
            "Available document slugs (use these EXACT strings in every citation's "
            f'"document_slug" field): {", ".join(slugs)}\n\n{body}'
        )
        user = f"Documentation context:\n\n{context}"
        for name in systems:
            spec = cfg["systems"][name]
            out = gen / f"{t['id']}__{name.replace('@', '_at_')}.json"
            if out.exists() and not args.force:
                print(f"  skip {t['id']} / {name} (cached)")
                continue
            print(f"  gen  {t['id']} / {name} (truncated={truncated})")
            try:
                if spec["backend"] == "lekton-live":
                    lesson = lekton_live(cfg, slugs[0])
                else:
                    system = render_system(spec["prompt"], t["target"])
                    lesson = call_backend(cfg, spec["backend"], system, user)
            except Exception as e:  # noqa: BLE001
                print(f"    FAILED: {e}", file=sys.stderr)
                out.write_text(json.dumps({"error": str(e), "system": name, "topic": t["id"]}, indent=2))
                continue
            out.write_text(json.dumps(
                {"system": name, "topic": t["id"], "source_slugs": slugs, "lesson": lesson},
                indent=2, ensure_ascii=False,
            ))
    print(f"generations in {gen}")


def _strip_html(s):
    return html.unescape(re.sub(r"<[^>]+>", "", s or "")).strip()


def render_lesson_text(lesson):
    """Plain-text render so the judge cannot tell systems apart by markup."""
    lines = [f"TITLE: {lesson.get('title', '')}", "", _strip_html(lesson.get("body_html", "")), ""]
    for i, q in enumerate(lesson.get("quiz", []), 1):
        lines.append(f"Q{i}. {q.get('prompt', '')}")
        for j, opt in enumerate(q.get("options", [])):
            mark = " (correct)" if j == q.get("correct_index") else ""
            lines.append(f"   - {opt}{mark}")
        if q.get("explanation"):
            lines.append(f"   explanation: {q['explanation']}")
        lines.append("")
    cits = lesson.get("citations", []) or []
    lines.append("CITATIONS:")
    for c in cits:
        lines.append(f"   - {c.get('document_slug')}: \"{c.get('quote', '')}\"")
    ps = lesson.get("primary_source")
    if ps:
        lines.append(f"PRIMARY SOURCE: {ps.get('document_slug')}")
    return "\n".join(lines)


def _read_lesson(gen, topic_id, system):
    p = gen / f"{topic_id}__{system.replace('@', '_at_')}.json"
    if not p.exists():
        return None
    data = json.loads(p.read_text())
    return data.get("lesson")


def cmd_judge(cfg, topics, args):
    run = resolve_run(args)
    gen, jdg = run / "gen", run / "judge"
    jdg.mkdir(exist_ok=True)
    rubric = (ROOT / "rubric.md").read_text()
    only = set(args.topics or [])
    comparisons = [c for c in cfg["comparisons"] if not args.comparisons or c["name"] in args.comparisons]
    for t in topics["topics"]:
        if only and t["id"] not in only:
            continue
        for comp in comparisons:
            out = jdg / f"{comp['name']}__{t['id']}.json"
            if out.exists() and not args.force:
                print(f"  skip {comp['name']} / {t['id']} (cached)")
                continue
            la, lb = _read_lesson(gen, t["id"], comp["a"]), _read_lesson(gen, t["id"], comp["b"])
            if not la or not lb:
                print(f"    missing generation for {comp['name']} / {t['id']}", file=sys.stderr)
                continue
            # Deterministic, reproducible A/B flip to defeat position bias.
            flip = int(hashlib.sha256(f"{comp['name']}:{t['id']}".encode()).hexdigest(), 16) % 2
            shown_a, shown_b = (comp["b"], comp["a"]) if flip else (comp["a"], comp["b"])
            text_a = render_lesson_text(lb if flip else la)
            text_b = render_lesson_text(la if flip else lb)
            user = (
                f"Topic: {t['target']}\n\n=== Lesson A ===\n{text_a}\n\n"
                f"=== Lesson B ===\n{text_b}\n"
            )
            print(f"  judge {comp['name']} / {t['id']}")
            try:
                verdict = extract_json(claude_chat(cfg, rubric, user, model=cfg["judge"]["model"]))
            except Exception as e:  # noqa: BLE001
                print(f"    FAILED: {e}", file=sys.stderr)
                continue
            # Map shown A/B back to real system names.
            result = {
                "comparison": comp["name"],
                "topic": t["id"],
                "systems": {"A": shown_a, "B": shown_b},
                "scores": {shown_a: verdict.get("a"), shown_b: verdict.get("b")},
                "winner_system": {"A": shown_a, "B": shown_b, "tie": "tie"}.get(verdict.get("winner"), "tie"),
                "reason": verdict.get("reason", ""),
            }
            out.write_text(json.dumps(result, indent=2, ensure_ascii=False))
    print(f"verdicts in {jdg}")


DIMS = ["grounding", "pedagogy", "quiz_discrimination", "no_format_tells", "citations", "scope_clarity"]


def cmd_report(cfg, topics, args):
    run = resolve_run(args)
    jdg = run / "judge"
    verdicts = [json.loads(p.read_text()) for p in sorted(jdg.glob("*.json"))]
    if not verdicts:
        sys.exit("no verdicts; run `judge` first")

    # per-comparison aggregation
    agg = {}  # comp -> system -> {dim: [scores], wins: n, total: n}
    for v in verdicts:
        comp = v["comparison"]
        a = agg.setdefault(comp, {})
        for system, sc in v["scores"].items():
            s = a.setdefault(system, {d: [] for d in DIMS})
            s.setdefault("wins", 0)
            s.setdefault("total", 0)
            if sc:
                for d in DIMS:
                    if isinstance(sc.get(d), (int, float)):
                        s[d].append(sc[d])
            s["total"] += 1
            if v["winner_system"] == system:
                s["wins"] += 1

    lines = [f"# Learn-mode benchmark report — {run.name}", ""]
    csv_rows = ["comparison,system,topics,wins," + ",".join(DIMS) + ",overall_avg"]
    for comp, systems in agg.items():
        lines += [f"## {comp}", "", "| system | topics | wins | " + " | ".join(DIMS) + " | avg |", "|" + "---|" * (len(DIMS) + 4)]
        for system, s in systems.items():
            means = [sum(s[d]) / len(s[d]) if s[d] else float("nan") for d in DIMS]
            valid = [m for m in means if m == m]
            overall = sum(valid) / len(valid) if valid else float("nan")
            fmt = lambda x: "-" if x != x else f"{x:.2f}"
            lines.append(
                f"| {system} | {s['total']} | {s['wins']} | "
                + " | ".join(fmt(m) for m in means)
                + f" | **{fmt(overall)}** |"
            )
            csv_rows.append(
                f"{comp},{system},{s['total']},{s['wins']},"
                + ",".join(fmt(m) for m in means)
                + f",{fmt(overall)}"
            )
        lines.append("")

    # notable reasons
    lines += ["## Judge reasons (per topic)", ""]
    for v in verdicts:
        lines.append(f"- **{v['comparison']} / {v['topic']}** → winner: `{v['winner_system']}` — {v['reason']}")

    report = "\n".join(lines) + "\n"
    (run / "report.md").write_text(report)
    (run / "scores.csv").write_text("\n".join(csv_rows) + "\n")
    print(report)
    print(f"written: {run / 'report.md'} and {run / 'scores.csv'}")


def main():
    cfg, topics = load_config(), load_topics()
    p = argparse.ArgumentParser(description="Learn-mode vs teach benchmark")
    sub = p.add_subparsers(dest="cmd", required=True)

    fx = sub.add_parser("fixtures", help="pull source markdown from Garage")
    fx.add_argument("--force", action="store_true")

    g = sub.add_parser("gen", help="generate lessons")
    g.add_argument("--run")
    g.add_argument("--systems", nargs="*", help="subset of system names")
    g.add_argument("--topics", nargs="*", help="subset of topic ids")
    g.add_argument("--force", action="store_true")

    j = sub.add_parser("judge", help="blind A/B judge")
    j.add_argument("--run")
    j.add_argument("--comparisons", nargs="*")
    j.add_argument("--topics", nargs="*")
    j.add_argument("--force", action="store_true")

    r = sub.add_parser("report", help="aggregate scores")
    r.add_argument("--run")

    args = p.parse_args()
    {"fixtures": cmd_fixtures, "gen": cmd_gen, "judge": cmd_judge, "report": cmd_report}[
        args.cmd
    ](cfg, topics, args)


if __name__ == "__main__":
    main()

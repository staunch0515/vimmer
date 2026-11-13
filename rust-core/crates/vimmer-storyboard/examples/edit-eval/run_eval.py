#!/usr/bin/env python3
"""Runs the editing-assistant eval: every case in cases.json through `vimmer edit --dry-run --json`.

THIS SPENDS MONEY: every case is at least one real Claude API call (about $0.01-0.05 each;
51 cases with Opus are roughly $1-2). The spend of every run is appended to results/spend.log.

    cd rust-core && cargo build -p vimmer-cli
    source ~/.zshrc   # ANTHROPIC_API_KEY; this script never reads or prints it
    python3 crates/vimmer-storyboard/examples/edit-eval/run_eval.py --model claude-sonnet-5-5 \
        --tag sonnet-low [--effort low] [--jobs 4] [--only id,id] [--budget 5]

Grading per case (cases.json): the status must be one of `status`; when it is `edited`, every
check must hold on the proposed timeline, and with `only`, every changed leaf must sit under one
of the listed JSON Pointer prefixes (catches edits that touch more than asked). Cases with
`review` also need a human read of the explanation and summary (see summary.md).

The explanation must be in the instruction's language (zh / en / ja, by script).

Check forms: [pointer, op, value]. `#total` is the summed scene length. `$before` is the value
at the same pointer before the edit. ops: eq, lt, gt, len, contains (array has value),
includes (string has value), falsy (absent, null or empty).
"""

import argparse
import concurrent.futures as cf
import datetime as dt
import json
import os
import re
import subprocess
import sys
import threading

HERE = os.path.dirname(os.path.abspath(__file__))
CRATE = os.path.normpath(os.path.join(HERE, "..", ".."))
REPO = os.path.normpath(os.path.join(CRATE, "..", "..", ".."))
DEFAULT_BIN = os.path.join(REPO, "rust-core", "target", "debug", "vimmer")
PUBLIC = os.path.join(REPO, "remotion-renderer", "public")
MISSING = object()


def resolve(doc, pointer):
    if pointer == "#total":
        return sum(s.get("duration_ms", 0) for s in doc.get("scenes", []))
    cur = doc
    for raw in pointer.split("/")[1:]:
        tok = raw.replace("~1", "/").replace("~0", "~")
        if isinstance(cur, list):
            if not tok.isdigit() or int(tok) >= len(cur):
                return MISSING
            cur = cur[int(tok)]
        elif isinstance(cur, dict):
            if tok not in cur:
                return MISSING
            cur = cur[tok]
        else:
            return MISSING
    return cur


def check(before, after, pointer, op, value=None):
    actual = resolve(after, pointer)
    if value == "$before":
        value = resolve(before, pointer)
    if op == "falsy":
        return actual is MISSING or not actual
    if actual is MISSING:
        return False
    if op == "eq":
        return actual == value
    if op == "lt":
        return actual < value
    if op == "gt":
        return actual > value
    if op == "len":
        return len(actual) == value
    if op == "contains":
        return value in actual
    if op == "includes":
        return isinstance(actual, str) and value in actual
    raise ValueError(f"unknown check op {op}")


def leaves(v, prefix=""):
    if isinstance(v, dict):
        if not v:
            yield prefix, v
        for k, x in v.items():
            yield from leaves(x, f"{prefix}/{k}")
    elif isinstance(v, list):
        if not v:
            yield prefix, v
        for i, x in enumerate(v):
            yield from leaves(x, f"{prefix}/{i}")
    else:
        yield prefix, v


def changed_paths(before, after):
    a, b = dict(leaves(before)), dict(leaves(after))
    return sorted(p for p in set(a) | set(b) if a.get(p, MISSING) != b.get(p, MISSING))


def under(path, prefixes):
    return any(path == p or path.startswith(p + "/") for p in prefixes)


QUOTED = re.compile(r'"[^"]*"|“[^”]*”|「[^」]*」|『[^』]*』|\'[^\']*\'|[\x21-\x7e]*[/.][\x21-\x7e]*')


def lang_of(text):
    """zh / en / ja by script, ignoring quoted text and path-like tokens."""
    text = QUOTED.sub(" ", text)
    kana = sum(1 for c in text if "\u3040" <= c <= "\u30ff")
    cjk = sum(1 for c in text if "\u4e00" <= c <= "\u9fff")
    if kana >= 3:
        return "ja"
    if cjk >= 3:
        return "zh"
    return "en"


def grade(case, before, result):
    """Returns (passed, reasons)."""
    if not result.get("ok"):
        return False, [f"failed: {result.get('error', {}).get('message', '?')[:200]}"]
    reasons = []
    status = result["status"]
    if status not in case["status"]:
        reasons.append(f"status {status}, expected {'/'.join(case['status'])}")
    want, got = lang_of(case["instruction"]), lang_of(result.get("explanation") or "")
    if want != got:
        reasons.append(f"explanation is in {got}, the instruction in {want}")
    if status == "edited":
        after = result["outcome"]["timeline"]
        for c in case.get("checks", []):
            if not check(before, after, *c):
                got = resolve(after, c[0])
                reasons.append(f"check {c} failed (got {json.dumps(None if got is MISSING else got, ensure_ascii=False)[:80]})")
        if "only" in case:
            extra = [p for p in changed_paths(before, after) if not under(p, case["only"])]
            if extra:
                reasons.append(f"changed outside {case['only']}: {extra[:6]}")
    return not reasons, reasons


def run_case(args, case, timelines):
    path = os.path.join(CRATE, timelines[case["timeline"]])
    with open(path, encoding="utf-8") as f:
        before = json.load(f)
    cmd = [args.bin, "edit", path, case["instruction"], "--dry-run", "--json",
           "--model", args.model, "--effort", args.effort, "--public-dir", PUBLIC]
    if case.get("scene"):
        cmd += ["--scene", case["scene"]]
    proc = subprocess.run(cmd, capture_output=True, text=True, timeout=900, cwd=REPO)
    line = next((l for l in reversed(proc.stdout.splitlines()) if l.strip()), "")
    try:
        result = json.loads(line)
    except json.JSONDecodeError:
        result = {"ok": False, "error": {"kind": "runner", "message": (proc.stderr or proc.stdout)[-400:]}}
    passed, reasons = grade(case, before, result)
    attempts = result.get("attempts") or []
    return {
        "id": case["id"],
        "group": case["group"],
        "instruction": case["instruction"],
        "passed": passed,
        "reasons": reasons,
        "review": case.get("review"),
        "ok": bool(result.get("ok")),
        "status": result.get("status"),
        "explanation": result.get("explanation"),
        "patch": result.get("patch"),
        "summary": (result.get("outcome") or {}).get("summary"),
        "warnings": (result.get("outcome") or {}).get("warnings"),
        "attempts": len(attempts),
        "retries": max(len(attempts) - 1, 0),
        "first_try_valid": bool(attempts) and attempts[0].get("error") is None,
        "rejections": [a.get("error") for a in attempts if a.get("error")],
        "models": sorted({a.get("model") for a in attempts if a.get("model")}),
        "cost_usd": (result.get("usage") or {}).get("cost_usd", 0.0),
        "usage": result.get("usage"),
        "error": result.get("error"),
    }


def summarize(rows, args, started):
    n = len(rows)
    passed = sum(r["passed"] for r in rows)
    first = sum(r["first_try_valid"] for r in rows)
    cost = sum(r["cost_usd"] for r in rows)
    retries = sum(r["retries"] for r in rows)
    groups = {}
    for r in rows:
        g = groups.setdefault(r["group"], [0, 0])
        g[0] += 1
        g[1] += r["passed"]
    out = [
        f"# Edit eval: {args.tag}",
        "",
        f"model `{args.model}`, effort `{args.effort}`, {n} cases, started {started}",
        "",
        f"- pass rate: **{passed}/{n}** ({100 * passed / max(n, 1):.0f} %)",
        f"- first-try valid replies: {first}/{n}",
        f"- retries needed: {retries} in total, in {sum(r['retries'] > 0 for r in rows)} case(s)",
        f"- cost: ${cost:.4f} total, ${cost / max(n, 1):.4f} per edit",
        "",
        "| group | passed |",
        "|---|---|",
    ]
    out += [f"| {g} | {p}/{c} |" for g, (c, p) in sorted(groups.items())]
    out += ["", "| case | pass | status | retries | cost | summary / explanation |", "|---|---|---|---|---|---|"]
    for r in rows:
        what = "; ".join(r["summary"] or []) or (r["explanation"] or (r["error"] or {}).get("message", ""))
        what = what.replace("|", "\\|").replace("\n", " ")
        mark = "yes" if r["passed"] else "**no**"
        out.append(f"| {r['id']} | {mark} | {r['status']} | {r['retries']} | ${r['cost_usd']:.4f} | {what[:160]} |")
    fails = [r for r in rows if not r["passed"]]
    if fails:
        out += ["", "## Failures", ""]
        for r in fails:
            out.append(f"- **{r['id']}** ({r['instruction']}): {'; '.join(r['reasons'])}")
    out += ["", "## Explanations (for review)", ""]
    for r in rows:
        flag = " (review)" if r["review"] else ""
        out.append(f"- **{r['id']}**{flag} [{r['status']}]: {(r['explanation'] or '').strip()}")
        for w in r["warnings"] or []:
            out.append(f"  - warning: {w}")
        for rej in r["rejections"]:
            out.append(f"  - rejected attempt: {rej.splitlines()[-1][:200]}")
    return "\n".join(out) + "\n", cost


def main():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("--model", default="claude-sonnet-5-5")
    p.add_argument("--effort", default="low")
    p.add_argument("--tag", required=True, help="results go to results/<tag>/")
    p.add_argument("--jobs", type=int, default=4)
    p.add_argument("--only", help="comma-separated case ids")
    p.add_argument("--budget", type=float, default=10.0, help="stop starting cases above this many USD")
    p.add_argument("--bin", default=DEFAULT_BIN)
    args = p.parse_args()
    if not os.environ.get("ANTHROPIC_API_KEY"):
        sys.exit("ANTHROPIC_API_KEY is not set (source ~/.zshrc first)")
    with open(os.path.join(HERE, "cases.json"), encoding="utf-8") as f:
        spec = json.load(f)
    cases = spec["cases"]
    if args.only:
        wanted = set(args.only.split(","))
        cases = [c for c in cases if c["id"] in wanted]
    started = dt.datetime.now().isoformat(timespec="seconds")
    out_dir = os.path.join(HERE, "results", args.tag)
    os.makedirs(out_dir, exist_ok=True)
    rows, spent, lock = [], [0.0], threading.Lock()

    def job(case):
        with lock:
            if spent[0] >= args.budget:
                return None
        row = run_case(args, case, spec["timelines"])
        with lock:
            spent[0] += row["cost_usd"]
        mark = "PASS" if row["passed"] else "FAIL"
        print(f"{mark} {row['id']:22} {row['status'] or '-':8} retries={row['retries']} ${row['cost_usd']:.4f}"
              + ("" if row["passed"] else f"  {'; '.join(row['reasons'])[:160]}"), flush=True)
        return row

    with cf.ThreadPoolExecutor(max_workers=args.jobs) as pool:
        for row in pool.map(job, cases):
            if row:
                rows.append(row)
    order = {c["id"]: i for i, c in enumerate(spec["cases"])}
    rows.sort(key=lambda r: order[r["id"]])
    with open(os.path.join(out_dir, "results.jsonl"), "w", encoding="utf-8") as f:
        for r in rows:
            f.write(json.dumps(r, ensure_ascii=False) + "\n")
    text, cost = summarize(rows, args, started)
    with open(os.path.join(out_dir, "summary.md"), "w", encoding="utf-8") as f:
        f.write(text)
    with open(os.path.join(HERE, "results", "spend.log"), "a", encoding="utf-8") as f:
        f.write(f"{started}\t{args.tag}\t{args.model}\t{args.effort}\t{len(rows)} cases\t${cost:.4f}\n")
    print(text.split("\n| group")[0])
    if len(rows) < len(cases):
        print(f"budget reached: {len(cases) - len(rows)} case(s) not run")


if __name__ == "__main__":
    main()

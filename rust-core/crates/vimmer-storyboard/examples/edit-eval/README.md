# Editing-assistant eval

**Running this costs real money.** Every case is at least one Claude API call: about $0.0004
(Haiku), $0.006 (Sonnet) or $0.012 (Opus, effort low) per case, so $0.02-$0.65 for the 53
cases. `--budget` (USD, default 10) stops starting new cases once a run has spent that much.
Every run appends its spend to `results/spend.log`.

- `cases.json`: 73 instructions in zh, en and ja, over `examples/*.timeline.json`, the
  `fixtures/*.json` and `timelines/zh-injected.timeline.json`. The last has a prompt injection
  in a scene's text. Groups: easy, multi (multi-scene and subtitles), ambiguous, impossible,
  hostile, layers (add / move / restyle / remove scene layers, on `fixtures/layers-demo.json` and
  the example timelines), anim (the data and text animation layer kinds and springy easings).
- `run_eval.py`: runs each case through `vimmer edit --dry-run --json` (4 at a time). A case
  passes when:
  - the status is one of the expected ones;
  - the explanation is in the instruction's language;
  - for `edited`, every value check holds, and with `only`, nothing outside the listed paths
    changed.
- `results/<tag>/summary.md`: pass rate, first-try validity, retries, cost, a per-case table, and
  every explanation and warning, for reading. `results.jsonl` has the patches and diff summaries.

```bash
cd rust-core && cargo build -p vimmer-cli
source ~/.zshrc   # ANTHROPIC_API_KEY; the runner never reads or prints it
python3 crates/vimmer-storyboard/examples/edit-eval/run_eval.py --model claude-sonnet-5-5 --tag my-run
python3 crates/vimmer-storyboard/examples/edit-eval/run_eval.py --model claude-opus-5-5 --tag opus --only zh-fast-fade,h-passwd
```

Runs in `results/`, in order:
- `r1`-`r4`: the first prompt.
- `final-*`: after the subtitle retiming and the explanation-language rule.
- `v2-*`: after the narration rule (the renderer lengthens a narrated scene to fit its speech).
  This is the current prompt for the 53 original cases.
- `layers-r1-sonnet-low`: the 9 `layer-*` cases after scene layers were added to the model and
  the prompt (`--only layer-...`): 9/9, $0.14.
- `anim-r2-sonnet-low`: the 11 `anim-*` cases (lower third, counter, bar chart, progress ring,
  typewriter, springy easings; mostly on `fixtures/anim-layers-demo.json`): 10/11, $0.10. Every
  patch was right; `anim-en-bouncy-star` fails the language rule in both runs (the explanation
  comes back in Chinese, likely because that timeline holds a Chinese text layer). A first run
  (`anim-r1`, $0.16, same 10/11) is in `spend.log` only.

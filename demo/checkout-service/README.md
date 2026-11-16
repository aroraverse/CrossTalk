# checkout-service (demo project)

A deliberately small Python checkout calculator (standard library only) used to demonstrate
**Crosstalk**: Claude Code builds a feature, Codex reviews it like a QA engineer, and the
human approves decisions from the dashboard.

- `checkout.py` — items, coupons, subtotal, tax, total
- `test_checkout.py` — existing tests (`python -m unittest -v`)
- `TASK.md` — the feature request and the hidden weaknesses

The full demo script is in [`docs/interview/05_LIVE_DEMO_CLAUDE_CODEX.md`](../../docs/interview/05_LIVE_DEMO_CLAUDE_CODEX.md).

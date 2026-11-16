# Task for the demo

**Feature request (from the "product manager"):**
Support coupons with a **maximum discount cap** — e.g. `FEST20`: 20% off, but at most ₹200 off —
and make sure **expired coupons are rejected**.

Roles:
- **Claude Code = Lead Developer.** Proposes the design, asks Codex to review, asks the human for
  approval on consequential decisions, implements, asks Codex to verify.
- **Codex = QA reviewer** (mirrors what Supatest / Alan do: an AI agent that finds bugs and writes
  tests). Reviews the proposal, lists edge cases, later runs the tests on the real diff.
- **You = human decision-maker** in the dashboard's Approvals page.

Weaknesses already in the code that a good reviewer should catch (don't tell the agents — let
Codex find them; this list is for you):

1. Money is a `float` → rounding errors (`0.1 + 0.2 != 0.3`). Fix: `decimal.Decimal` or integer paise.
2. `today` is accepted but never used → **expired coupons still apply**.
3. `percent_off > 100` makes the total **negative**; `percent_off < 0` increases it.
4. Negative `qty` or price is accepted.
5. Open question: is the cap applied **before tax** (on the subtotal) or after? → a business
   decision → perfect for `ask_user`.

Changing `float` to `Decimal` changes the public data types of `Item`/`Coupon` → a breaking API
change → Claude must call `ask_user` before doing it.

Run the tests: `python -m unittest -v` (from this folder).

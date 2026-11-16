# Issue #42: customers get refunded twice

Support ticket: "I clicked Refund, the page hung, so I clicked again. I got the money twice."

The mobile app also retries a refund automatically after a 10 s timeout.

**Expected:** the same refund request, sent any number of times, pays out once.

Acceptance:
- `refund()` takes an idempotency key; a repeat of the same key returns the first payout and pays nothing.
- Two requests with the same key at the same moment still pay once.
- A key belongs to one customer: another customer reusing it must not get anything back.
- Tests cover a double-click, a retry, two concurrent requests and a key reused by another customer.

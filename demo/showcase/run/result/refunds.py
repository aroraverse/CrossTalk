"""Refund service for a small online shop (standard library only).

Used for the Crosstalk showcase: several coding agents fix a real bug here
together. The bug: a customer who double-clicks "Refund" (or a client that
retries after a timeout) gets paid twice.
"""

import sqlite3
import threading
import time


class RefundError(ValueError):
    """A refund request that cannot be honoured."""


class RefundService:
    def __init__(self, db_path=":memory:"):
        self.db = sqlite3.connect(db_path, check_same_thread=False, isolation_level=None, timeout=30)
        self._lock = threading.Lock()
        self.db.executescript(
            """
            CREATE TABLE IF NOT EXISTS orders (
                order_id    TEXT PRIMARY KEY,
                customer_id TEXT NOT NULL,
                paid_cents  INTEGER NOT NULL,
                refunded_cents INTEGER NOT NULL DEFAULT 0
            );
            CREATE TABLE IF NOT EXISTS payouts (
                payout_id   INTEGER PRIMARY KEY AUTOINCREMENT,
                order_id    TEXT NOT NULL,
                amount_cents INTEGER NOT NULL,
                created_at  REAL NOT NULL
            );
            CREATE TABLE IF NOT EXISTS idempotency_keys (
                key         TEXT PRIMARY KEY,
                customer_id TEXT NOT NULL,
                order_id    TEXT NOT NULL,
                amount_cents INTEGER NOT NULL,
                payout_id   INTEGER NOT NULL REFERENCES payouts(payout_id)
            );
            """
        )

    def add_order(self, order_id, customer_id, paid_cents):
        self.db.execute(
            "INSERT INTO orders (order_id, customer_id, paid_cents) VALUES (?, ?, ?)",
            (order_id, customer_id, paid_cents),
        )
        self.db.commit()

    def refund(self, order_id, customer_id, amount_cents, idempotency_key):
        """Pay a refund once per idempotency key; repeats return the first payout id.

        A key is bound to (customer, order, amount). Reusing it for anything
        else raises RefundError. Limit: the provider call is not atomic with
        the DB commit, so the provider should also receive the key.
        """
        if not idempotency_key:
            raise RefundError("idempotency key required")
        with self._lock:
            self.db.execute("BEGIN IMMEDIATE")
            try:
                prior = self.db.execute(
                    "SELECT customer_id, order_id, amount_cents, payout_id"
                    " FROM idempotency_keys WHERE key = ?",
                    (idempotency_key,),
                ).fetchone()
                if prior is not None:
                    if prior[:3] != (customer_id, order_id, amount_cents):
                        raise RefundError("idempotency key already used for a different request")
                    self.db.execute("COMMIT")
                    return prior[3]
                row = self.db.execute(
                    "SELECT customer_id, paid_cents, refunded_cents FROM orders WHERE order_id = ?",
                    (order_id,),
                ).fetchone()
                if row is None:
                    raise RefundError(f"unknown order {order_id}")
                owner, paid, refunded = row
                if owner != customer_id:
                    raise RefundError("order belongs to another customer")
                if amount_cents <= 0 or refunded + amount_cents > paid:
                    raise RefundError("refund exceeds what was paid")
                time.sleep(0.01)  # the payment provider call
                cur = self.db.execute(
                    "INSERT INTO payouts (order_id, amount_cents, created_at) VALUES (?, ?, ?)",
                    (order_id, amount_cents, time.time()),
                )
                self.db.execute(
                    "UPDATE orders SET refunded_cents = refunded_cents + ? WHERE order_id = ?",
                    (amount_cents, order_id),
                )
                self.db.execute(
                    "INSERT INTO idempotency_keys (key, customer_id, order_id, amount_cents, payout_id)"
                    " VALUES (?, ?, ?, ?, ?)",
                    (idempotency_key, customer_id, order_id, amount_cents, cur.lastrowid),
                )
                self.db.execute("COMMIT")
                return cur.lastrowid
            except BaseException:
                self.db.execute("ROLLBACK")
                raise

    def payouts(self, order_id):
        return [
            amount
            for (amount,) in self.db.execute(
                "SELECT amount_cents FROM payouts WHERE order_id = ? ORDER BY payout_id", (order_id,)
            )
        ]

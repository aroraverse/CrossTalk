"""Refund service for a small online shop (standard library only).

Used for the Crosstalk showcase: several coding agents fix a real bug here
together. The bug: a customer who double-clicks "Refund" (or a client that
retries after a timeout) gets paid twice.
"""

import sqlite3
import time


class RefundError(ValueError):
    """A refund request that cannot be honoured."""


class RefundService:
    def __init__(self, db_path=":memory:"):
        self.db = sqlite3.connect(db_path, check_same_thread=False)
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
            """
        )

    def add_order(self, order_id, customer_id, paid_cents):
        self.db.execute(
            "INSERT INTO orders (order_id, customer_id, paid_cents) VALUES (?, ?, ?)",
            (order_id, customer_id, paid_cents),
        )
        self.db.commit()

    def refund(self, order_id, customer_id, amount_cents):
        """Pay a refund back to the customer.

        BUG: nothing stops the same request from being processed twice, so a
        double-click or a client retry sends the money twice.
        """
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
        self.db.commit()
        return cur.lastrowid

    def payouts(self, order_id):
        return [
            amount
            for (amount,) in self.db.execute(
                "SELECT amount_cents FROM payouts WHERE order_id = ? ORDER BY payout_id", (order_id,)
            )
        ]

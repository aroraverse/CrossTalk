"""Existing tests. Run with:  python -m unittest -v"""

import unittest
from datetime import date

from checkout import (Coupon, ExpiredCouponError, Item, apply_coupon,
                      order_total, subtotal)


class CheckoutTests(unittest.TestCase):
    def test_subtotal(self):
        items = [Item("pen", 10.0, 3), Item("book", 250.0, 1)]
        self.assertEqual(subtotal(items), 280.0)

    def test_total_without_coupon_adds_tax(self):
        self.assertEqual(order_total([Item("book", 100.0, 1)]), 118.0)

    def test_total_with_coupon(self):
        coupon = Coupon("SAVE10", 10, date(2099, 1, 1))
        self.assertEqual(order_total([Item("book", 100.0, 1)], coupon), 106.2)

    # --- cap ---
    def test_cap_binds(self):
        c = Coupon("FEST20", 20, date(2099, 1, 1), max_discount=200)
        # 2000 -> 20% = 400, capped 200 -> 1800 * 1.18
        self.assertEqual(order_total([Item("tv", 2000.0, 1)], c), 2124.0)

    def test_cap_not_binding(self):
        c = Coupon("FEST20", 20, date(2099, 1, 1), max_discount=200)
        self.assertEqual(order_total([Item("b", 500.0, 1)], c), 472.0)

    def test_cap_exactly_at_threshold(self):
        c = Coupon("FEST20", 20, date(2099, 1, 1), max_discount=200)
        self.assertEqual(order_total([Item("b", 1000.0, 1)], c), 944.0)

    def test_zero_cap_means_no_discount(self):
        c = Coupon("X", 20, date(2099, 1, 1), max_discount=0)
        self.assertEqual(order_total([Item("b", 100.0, 1)], c), 118.0)

    def test_cap_larger_than_amount(self):
        c = Coupon("X", 50, date(2099, 1, 1), max_discount=10000)
        self.assertEqual(order_total([Item("b", 100.0, 1)], c), 59.0)

    def test_no_cap(self):
        c = Coupon("X", 20, date(2099, 1, 1))
        self.assertEqual(order_total([Item("tv", 2000.0, 1)], c), 1888.0)

    # --- expiry ---
    def test_expired_rejected(self):
        c = Coupon("OLD", 10, date(2024, 1, 1))
        with self.assertRaises(ExpiredCouponError):
            order_total([Item("b", 100.0, 1)], c, today=date(2024, 1, 2))

    def test_expires_today_ok(self):
        c = Coupon("T", 10, date(2024, 1, 1))
        self.assertEqual(
            order_total([Item("b", 100.0, 1)], c, today=date(2024, 1, 1)),
            106.2)

    def test_no_coupon_bypasses_expiry(self):
        self.assertEqual(
            order_total([Item("b", 100.0, 1)], None, today=date(2024, 1, 1)),
            118.0)

    def test_apply_coupon_direct_expired(self):
        c = Coupon("OLD", 10, date(2024, 1, 1))
        with self.assertRaises(ExpiredCouponError):
            apply_coupon(100.0, c, today=date(2024, 1, 2))

    # --- validation ---
    def test_percent_out_of_range(self):
        for p in (-1, 101, float("nan"), float("inf")):
            c = Coupon("X", p, date(2099, 1, 1))
            with self.assertRaises(ValueError):
                order_total([Item("b", 100.0, 1)], c)

    def test_negative_or_bad_cap(self):
        for cap in (-5, float("nan"), float("inf")):
            c = Coupon("X", 10, date(2099, 1, 1), max_discount=cap)
            with self.assertRaises(ValueError):
                order_total([Item("b", 100.0, 1)], c)

    def test_negative_price_and_qty_rejected(self):
        with self.assertRaises(ValueError):
            order_total([Item("b", -1.0, 1)])
        with self.assertRaises(ValueError):
            order_total([Item("b", 1.0, -1)])

    # --- Codex QA edge cases ---
    def test_apply_coupon_rejects_bad_amount(self):
        c = Coupon("X", 10, date(2099, 1, 1))
        for a in (-1, float("nan"), float("inf")):
            with self.assertRaises(ValueError):
                apply_coupon(a, c)

    def test_apply_coupon_direct_cap_and_zero(self):
        c = Coupon("X", 20, date(2099, 1, 1), max_discount=200)
        self.assertEqual(apply_coupon(2000.0, c), 1800.0)
        self.assertEqual(apply_coupon(1000.0, c), 800.0)
        self.assertEqual(apply_coupon(500.0, c), 400.0)
        self.assertEqual(apply_coupon(0, c), 0.0)
        z = Coupon("X", 20, date(2099, 1, 1), max_discount=0)
        self.assertEqual(apply_coupon(100.0, z), 100.0)

    def test_apply_coupon_expiry_boundary(self):
        c = Coupon("T", 10, date(2024, 1, 1))
        self.assertEqual(apply_coupon(100.0, c, today=date(2024, 1, 1)), 90.0)

    def test_subcent_cap_never_exceeded(self):
        c = Coupon("X", 100, date(2099, 1, 1), max_discount=0.005)
        self.assertEqual(apply_coupon(1.0, c), 1.0)  # cap rounds down to 0.00
        c2 = Coupon("X", 100, date(2099, 1, 1), max_discount=0.019)
        self.assertEqual(apply_coupon(1.0, c2), 0.99)

    def test_malformed_dates_rejected(self):
        with self.assertRaises(ValueError):
            order_total([Item("b", 1.0, 1)], Coupon("X", 10, "2099-01-01"))
        with self.assertRaises(ValueError):
            order_total([Item("b", 1.0, 1)], Coupon("X", 10, date(2099, 1, 1)),
                        today="2024-01-01")

    def test_qty_and_price_edge_cases(self):
        self.assertEqual(order_total([Item("b", 10.0, 0)]), 0.0)
        for q in (True, 1.5, "2"):
            with self.assertRaises(ValueError):
                order_total([Item("b", 1.0, q)])
        for p in (float("nan"), float("inf")):
            with self.assertRaises(ValueError):
                order_total([Item("b", p, 1)])

    def test_gst_half_cent_rounding(self):
        # 0.05 * 1.18 = 0.059 -> 0.06 ; 0.15*1.18 = 0.177 -> 0.18
        self.assertEqual(order_total([Item("b", 0.05, 1)]), 0.06)
        self.assertEqual(order_total([Item("b", 0.15, 1)]), 0.18)

    # --- arithmetic / rounding ---
    def test_float_precision(self):
        self.assertEqual(subtotal([Item("a", 0.1, 1), Item("b", 0.2, 1)]), 0.3)

    def test_fractional_percent(self):
        c = Coupon("X", 12.5, date(2099, 1, 1))
        self.assertEqual(order_total([Item("b", 100.0, 1)], c), 103.25)

    def test_half_up_rounding(self):
        # 0.25 * 1.18 = 0.295 -> half-up 0.30 (banker's/float gives 0.29)
        self.assertEqual(order_total([Item("b", 0.25, 1)]), 0.3)

    def test_empty_items_and_zero_subtotal(self):
        self.assertEqual(order_total([]), 0.0)

    def test_full_discount_gives_zero(self):
        c = Coupon("FREE", 100, date(2099, 1, 1))
        self.assertEqual(order_total([Item("b", 100.0, 2)], c), 0.0)


if __name__ == "__main__":
    unittest.main()

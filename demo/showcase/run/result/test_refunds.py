"""Existing tests. Run with:  python -m unittest -v"""

import threading
import unittest

from refunds import RefundError, RefundService


class RefundTests(unittest.TestCase):
    def setUp(self):
        self.svc = RefundService()
        self.svc.add_order("ord-1", "cus-ana", 5000)

    def test_full_refund(self):
        self.svc.refund("ord-1", "cus-ana", 5000, "k1")
        self.assertEqual(self.svc.payouts("ord-1"), [5000])

    def test_cannot_refund_more_than_paid(self):
        with self.assertRaises(RefundError):
            self.svc.refund("ord-1", "cus-ana", 6000, "k2")

    def test_cannot_refund_someone_elses_order(self):
        with self.assertRaises(RefundError):
            self.svc.refund("ord-1", "cus-bob", 100, "k3")
    def test_double_click_pays_once(self):
        first = self.svc.refund("ord-1", "cus-ana", 5000, "dbl")
        second = self.svc.refund("ord-1", "cus-ana", 5000, "dbl")
        self.assertEqual(first, second)
        self.assertEqual(self.svc.payouts("ord-1"), [5000])

    def test_client_retry_after_timeout_pays_once(self):
        first = self.svc.refund("ord-1", "cus-ana", 1000, "retry")
        for _ in range(3):
            self.assertEqual(self.svc.refund("ord-1", "cus-ana", 1000, "retry"), first)
        self.assertEqual(self.svc.payouts("ord-1"), [1000])

    def test_concurrent_requests_same_key_pay_once(self):
        results, errors = [], []
        barrier = threading.Barrier(2)

        def worker():
            try:
                barrier.wait()
                results.append(self.svc.refund("ord-1", "cus-ana", 2000, "conc"))
            except Exception as exc:  # noqa: BLE001
                errors.append(exc)

        threads = [threading.Thread(target=worker) for _ in range(2)]
        for t in threads:
            t.start()
        for t in threads:
            t.join()
        self.assertEqual(errors, [])
        self.assertEqual(len(results), 2)
        self.assertEqual(results[0], results[1])
        self.assertEqual(self.svc.payouts("ord-1"), [2000])

    def test_idempotency_key_cannot_be_reused_by_another_customer(self):
        # Alice successfully refunds with key 'kx'
        self.svc.add_order("ord-2", "cus-bob", 2000)
        first = self.svc.refund("ord-1", "cus-ana", 500, "kx")
        self.assertEqual(self.svc.payouts("ord-1"), [500])
        # Bob tries the SAME key on his own order with his own customer_id.
        with self.assertRaises(RefundError):
            self.svc.refund("ord-2", "cus-bob", 500, "kx")
        # Bob gets nothing; Alice's single payout is unchanged.
        self.assertEqual(self.svc.payouts("ord-1"), [500])
        self.assertEqual(self.svc.payouts("ord-2"), [])


if __name__ == "__main__":
    unittest.main()

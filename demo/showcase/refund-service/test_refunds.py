"""Existing tests. Run with:  python -m unittest -v"""

import unittest

from refunds import RefundError, RefundService


class RefundTests(unittest.TestCase):
    def setUp(self):
        self.svc = RefundService()
        self.svc.add_order("ord-1", "cus-ana", 5000)

    def test_full_refund(self):
        self.svc.refund("ord-1", "cus-ana", 5000)
        self.assertEqual(self.svc.payouts("ord-1"), [5000])

    def test_cannot_refund_more_than_paid(self):
        with self.assertRaises(RefundError):
            self.svc.refund("ord-1", "cus-ana", 6000)

    def test_cannot_refund_someone_elses_order(self):
        with self.assertRaises(RefundError):
            self.svc.refund("ord-1", "cus-bob", 100)


if __name__ == "__main__":
    unittest.main()

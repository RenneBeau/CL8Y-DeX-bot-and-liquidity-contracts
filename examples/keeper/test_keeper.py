import unittest

from keeper import calculate_offer


class CalculateOfferTests(unittest.TestCase):
    def test_balanced_inventory_requires_no_trade(self):
        self.assertIsNone(calculate_offer([100, 100], [1_000, 1_000], 2_500))

    def test_excess_token1_is_offered(self):
        self.assertEqual(calculate_offer([100, 200], [1_000, 1_000], 10_000), (1, 50))

    def test_excess_token0_is_offered(self):
        self.assertEqual(calculate_offer([200, 100], [1_000, 1_000], 10_000), (0, 50))

    def test_trade_cap_limits_amount(self):
        self.assertEqual(calculate_offer([100, 1_000], [1_000, 1_000], 1_000), (1, 100))


if __name__ == "__main__":
    unittest.main()

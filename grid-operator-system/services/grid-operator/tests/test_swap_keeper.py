import json
import os
import tempfile
import unittest
from unittest.mock import MagicMock, patch

from grid_operator.rpc import RpcError
from grid_operator.swap_keeper import (
    SwapTxTracker,
    build_rebalance,
    is_transient_error,
    plan_fingerprint,
    poll_pending,
    run_once,
)


def grid_status(**overrides):
    status = {
        "current_cell": 3,
        "target_weight_bps": 7500,
        "allocation_deviation_bps": 1200,
        "should_rebalance": True,
        "captured_twap": "1.75",
        "balances": ["60000000000", "60000000000"],
        "offer_token": None,
        "amount": None,
        "min_return": None,
        "pending_swap": False,
    }
    status.update(overrides)
    return status


class SwapTxTrackerTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.path = os.path.join(self.tmp.name, "state.json")

    def tearDown(self):
        self.tmp.cleanup()

    def test_new_tracker_when_file_does_not_exist(self):
        t = SwapTxTracker(self.path)
        self.assertIsNone(t.pending_hash)
        self.assertFalse(t.broadcasting)

    def test_loads_existing_state_on_init(self):
        with open(self.path, "w", encoding="utf-8") as f:
            json.dump(
                {
                    "pending_hash": "abc",
                    "pending_vault": "vault1",
                    "pending_plan": "plan123",
                    "pending_since": 42.0,
                    "suppressed_plan": "suppressed",
                    "broadcasting": True,
                },
                f,
            )
        t = SwapTxTracker(self.path)
        self.assertEqual(t.pending_hash, "abc")
        self.assertEqual(t.pending_vault, "vault1")
        self.assertEqual(t.pending_plan, "plan123")
        self.assertEqual(t.suppressed_plan, "suppressed")
        self.assertTrue(t.broadcasting)

    def test_save_is_atomic_and_readable(self):
        t = SwapTxTracker(self.path)
        t.pending_hash = "xyz"
        t.broadcasting = True
        t.save()
        with open(self.path, encoding="utf-8") as f:
            state = json.load(f)
        self.assertEqual(state["pending_hash"], "xyz")
        self.assertTrue(state["broadcasting"])


class BuildRebalanceTests(unittest.TestCase):
    def test_returns_rebalance_when_required_and_no_pending_swap(self):
        message = build_rebalance(grid_status(), 12345)
        self.assertEqual(message, {"rebalance": {"deadline": 12345}})

    def test_returns_none_when_not_required(self):
        self.assertIsNone(build_rebalance(grid_status(should_rebalance=False), 12345))

    def test_returns_none_when_swap_pending(self):
        self.assertIsNone(build_rebalance(grid_status(pending_swap=True), 12345))


class TransientErrorTests(unittest.TestCase):
    def test_transient_markers_are_recognized(self):
        self.assertTrue(is_transient_error("account sequence mismatch"))
        self.assertTrue(is_transient_error("context deadline exceeded"))
        self.assertFalse(is_transient_error("unauthorized"))
        self.assertFalse(is_transient_error("insufficient funds"))


class FingerprintTests(unittest.TestCase):
    def test_fingerprint_is_deterministic_and_plan_sensitive(self):
        plan = grid_status()
        message = {"rebalance": {"deadline": 12345}}
        first = plan_fingerprint(plan, message, "VAULT1")
        second = plan_fingerprint(plan, message, "vault1")
        self.assertEqual(first, second)
        different = plan_fingerprint(plan, message, "vault2")
        self.assertNotEqual(first, different)

    def test_fingerprint_is_insensitive_to_deadline(self):
        plan = grid_status()
        early = plan_fingerprint(plan, {"rebalance": {"deadline": 1}}, "vault1")
        late = plan_fingerprint(plan, {"rebalance": {"deadline": 999_999}}, "vault1")
        self.assertEqual(early, late)


class RunOnceTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.path = os.path.join(self.tmp.name, "state.json")

    def tearDown(self):
        self.tmp.cleanup()

    def args(self, **overrides):
        values = {
            "vault": "vault1",
            "deadline_seconds": 120,
            "broadcast": False,
            "tx_poll_seconds": 0.01,
            "tx_timeout_seconds": 1,
            "confirmation_blocks": 0,
        }
        values.update(overrides)
        return type("Args", (), values)()

    def test_dry_run_reports_no_rebalance(self):
        terrad = MagicMock()
        terrad.smart_query.return_value = grid_status(should_rebalance=False)
        tracker = SwapTxTracker(self.path)
        with patch("builtins.print") as mock_print:
            run_once(self.args(), terrad, tracker)
        mock_print.assert_any_call(
            "no rebalance: should_rebalance=False pending_swap=False cell=3 deviation_bps=1200"
        )
        terrad.sign_execute.assert_not_called()

    def test_dry_run_prints_message_without_signing(self):
        terrad = MagicMock()
        terrad.smart_query.return_value = grid_status()
        tracker = SwapTxTracker(self.path)
        with patch("builtins.print") as mock_print:
            run_once(self.args(), terrad, tracker)
        terrad.sign_execute.assert_not_called()
        printed = [call.args[0] for call in mock_print.call_args_list]
        self.assertTrue(any("dry-run only" in text for text in printed))
        self.assertTrue(any("rebalance" in text and "deadline" in text for text in printed))

    def test_query_failure_is_tolerated(self):
        terrad = MagicMock()
        terrad.smart_query.side_effect = RpcError("endpoint down")
        tracker = SwapTxTracker(self.path)
        with patch("builtins.print") as mock_print:
            run_once(self.args(), terrad, tracker)
        mock_print.assert_any_call("grid_status query failed: endpoint down")

    def test_pending_tx_is_polled_not_duplicated(self):
        terrad = MagicMock()
        tracker = SwapTxTracker(self.path)
        tracker.pending_hash = "0xabc"
        with patch("grid_operator.swap_keeper.poll_pending") as mock_poll:
            run_once(self.args(), terrad, tracker)
        mock_poll.assert_called_once()
        terrad.smart_query.assert_not_called()

    def test_broadcast_flow_signs_and_polls(self):
        terrad = MagicMock()
        terrad.smart_query.return_value = grid_status()
        terrad.preflight.return_value = {}
        terrad.sign_execute.return_value = b"signed"
        terrad.broadcast.return_value = {"tx_response": {"code": 0, "txhash": "0xtx"}}
        tracker = SwapTxTracker(self.path)
        with patch("grid_operator.swap_keeper.poll_pending") as mock_poll:
            run_once(self.args(broadcast=True), terrad, tracker)
        terrad.preflight.assert_called_once()
        terrad.sign_execute.assert_called_once()
        terrad.broadcast.assert_called_once()
        self.assertEqual(tracker.pending_hash, "0xtx")
        mock_poll.assert_called_once()


if __name__ == "__main__":
    unittest.main()

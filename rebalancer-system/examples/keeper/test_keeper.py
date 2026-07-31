import json
import os
import tempfile
import unittest
from unittest.mock import MagicMock, patch
from urllib.error import HTTPError

from keeper import (
    TxTracker,
    DeterministicTxError,
    build_rebalance,
    plan_fingerprint,
    broadcast,
    preflight,
    query_final_tx,
    poll_pending,
    run_once,
    parse_args,
    get_json,
    smart_query,
    tx_command,
    run_command,
)


class TxTrackerTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.path = os.path.join(self.tmp.name, "state.json")

    def tearDown(self):
        self.tmp.cleanup()

    def test_new_tracker_when_file_does_not_exist(self):
        t = TxTracker(self.path)
        self.assertIsNone(t.pending_hash)
        self.assertIsNone(t.pending_plan)
        self.assertIsNone(t.pending_since)
        self.assertIsNone(t.suppressed_plan)

    def test_loads_existing_state_on_init(self):
        with open(self.path, "w", encoding="utf-8") as f:
            json.dump(
                {
                    "pending_hash": "abc",
                    "pending_plan": "plan123",
                    "pending_since": 42.0,
                    "suppressed_plan": "suppressed",
                },
                f,
            )
        t = TxTracker(self.path)
        self.assertEqual(t.pending_hash, "abc")
        self.assertEqual(t.pending_plan, "plan123")
        self.assertEqual(t.pending_since, 42.0)
        self.assertEqual(t.suppressed_plan, "suppressed")

    def test_save_creates_file_with_mode_600(self):
        t = TxTracker(self.path)
        t.pending_hash = "def"
        t.pending_plan = "plan456"
        t.pending_since = 99.0
        t.suppressed_plan = None
        t.save()
        self.assertTrue(os.path.exists(self.path))
        mode = os.stat(self.path).st_mode & 0o777
        self.assertEqual(mode, 0o600)
        with open(self.path, encoding="utf-8") as f:
            state = json.load(f)
        self.assertEqual(state["pending_hash"], "def")
        self.assertEqual(state["pending_plan"], "plan456")
        self.assertEqual(state["pending_since"], 99.0)
        self.assertIsNone(state["suppressed_plan"])

    def test_save_none_path_is_noop(self):
        t = TxTracker(path=None)
        self.assertIsNone(t.path)
        t.save()

    def test_save_cleared_state(self):
        t = TxTracker(self.path)
        t.pending_hash = "old"
        t.pending_plan = "old"
        t.pending_since = 1.0
        t.suppressed_plan = "old"
        t.save()
        t.pending_hash = None
        t.pending_plan = None
        t.pending_since = None
        t.suppressed_plan = None
        t.save()
        with open(self.path, encoding="utf-8") as f:
            state = json.load(f)
        self.assertIsNone(state["pending_hash"])
        self.assertIsNone(state["pending_plan"])
        self.assertIsNone(state["pending_since"])
        self.assertIsNone(state["suppressed_plan"])


class BuildRebalanceTests(unittest.TestCase):
    def test_returns_none_when_should_rebalance_is_false(self):
        plan = {"should_rebalance": False}
        self.assertIsNone(build_rebalance(plan, 1000))

    def test_sync_reference_when_offer_token_is_none(self):
        plan = {"should_rebalance": True, "offer_token": None}
        self.assertEqual(build_rebalance(plan, 1000), {"sync_reference": {}})

    def test_rebalance_when_offer_token_present(self):
        plan = {"should_rebalance": True, "offer_token": "token1"}
        self.assertEqual(build_rebalance(plan, 999), {"rebalance": {"deadline": 999}})


class PlanFingerprintTests(unittest.TestCase):
    def test_deterministic_output(self):
        plan = {
            "captured_twap": "123.456",
            "balances": ["100", "200"],
            "reference_price": "1.5",
        }
        message = {"rebalance": {"deadline": 1000}}
        fp1 = plan_fingerprint(plan, message)
        fp2 = plan_fingerprint(plan, message)
        self.assertEqual(fp1, fp2)

    def test_different_plan_yields_different_fingerprint(self):
        plan_a = {
            "captured_twap": "100",
            "balances": ["100", "200"],
            "reference_price": "1.5",
        }
        plan_b = {
            "captured_twap": "200",
            "balances": ["100", "200"],
            "reference_price": "1.5",
        }
        message = {"rebalance": {"deadline": 1000}}
        self.assertNotEqual(
            plan_fingerprint(plan_a, message),
            plan_fingerprint(plan_b, message),
        )

    def test_uses_next_of_message_actions(self):
        plan = {
            "captured_twap": "1",
            "balances": ["1", "2"],
            "reference_price": "1.0",
        }
        msg_sync = {"sync_reference": {}}
        msg_reb = {"rebalance": {"deadline": 100}}
        self.assertNotEqual(
            plan_fingerprint(plan, msg_sync),
            plan_fingerprint(plan, msg_reb),
        )


class QueryFinalTxTests(unittest.TestCase):
    def test_returns_lcd_response_when_available(self):
        lcd_data = {
            "tx_response": {
                "code": "0",
                "raw_log": "[]",
                "height": "100",
            }
        }
        with patch("keeper.get_json", return_value=lcd_data) as mock_get:
            result = query_final_tx("http://lcd", "http://rpc", "txhash1")
        self.assertEqual(result, {"code": 0, "raw_log": "[]", "height": "100"})
        mock_get.assert_called_once()

    def test_falls_back_to_rpc_when_lcd_returns_empty(self):
        lcd_data = {"tx_response": {}}
        rpc_data = {
            "result": {
                "tx_result": {"code": 3, "log": "failed"},
                "height": "200",
            }
        }
        with patch("keeper.get_json", side_effect=[lcd_data, rpc_data]) as mock_get:
            result = query_final_tx("http://lcd", "http://rpc", "txhash2")
        self.assertEqual(result, {"code": 3, "raw_log": "failed", "height": "200"})
        self.assertEqual(mock_get.call_count, 2)

    def test_falls_back_to_rpc_on_lcd_404(self):
        rpc_data = {
            "result": {
                "tx_result": {"code": 0, "log": ""},
                "height": "300",
            }
        }
        with patch("keeper.get_json") as mock_get:
            mock_get.side_effect = [
                HTTPError("url", 404, "Not Found", {}, None),
                rpc_data,
            ]
            result = query_final_tx("http://lcd", "http://rpc", "txhash3")
        self.assertEqual(result, {"code": 0, "raw_log": "", "height": "300"})
        self.assertEqual(mock_get.call_count, 2)

    def test_returns_none_when_both_404(self):
        with patch("keeper.get_json") as mock_get:
            mock_get.side_effect = HTTPError("url", 404, "Not Found", {}, None)
            result = query_final_tx("http://lcd", "http://rpc", "txhash4")
        self.assertIsNone(result)

    def test_returns_none_when_rpc_returns_no_result(self):
        with patch("keeper.get_json") as mock_get:
            mock_get.side_effect = [
                {"tx_response": {}},
                {"result": None},
            ]
            result = query_final_tx("http://lcd", "http://rpc", "txhash5")
        self.assertIsNone(result)

    def test_re_raises_non_404_lcd_error(self):
        with patch("keeper.get_json") as mock_get:
            mock_get.side_effect = HTTPError("url", 500, "Server Error", {}, None)
            with self.assertRaises(HTTPError):
                query_final_tx("http://lcd", "http://rpc", "txhash6")


class PollPendingTests(unittest.TestCase):
    def test_returns_true_on_success(self):
        args = MagicMock(lcd="lcd", rpc="rpc", tx_timeout_seconds=60, tx_poll_seconds=1)
        tracker = MagicMock(pending_hash="hash1", pending_plan="fp")
        query_tx = MagicMock(return_value={"code": 0, "raw_log": "", "height": "50"})
        sleep = MagicMock()

        result = poll_pending(args, tracker, query_tx=query_tx, sleep=sleep)

        self.assertTrue(result)
        self.assertIsNone(tracker.pending_hash)
        self.assertIsNone(tracker.pending_plan)
        self.assertIsNone(tracker.pending_since)
        self.assertIsNone(tracker.suppressed_plan)
        tracker.save.assert_called_once()

    def test_suppresses_plan_on_delivertx_failure(self):
        args = MagicMock(lcd="lcd", rpc="rpc", tx_timeout_seconds=60, tx_poll_seconds=1)
        tracker = MagicMock(pending_hash="hash2", pending_plan="fp_bad")
        query_tx = MagicMock(return_value={"code": 5, "raw_log": "out of gas", "height": "51"})
        sleep = MagicMock()

        with self.assertRaises(DeterministicTxError) as ctx:
            poll_pending(args, tracker, query_tx=query_tx, sleep=sleep)
        self.assertIn("hash2", str(ctx.exception))
        self.assertIn("out of gas", str(ctx.exception))
        self.assertIsNone(tracker.pending_hash)
        self.assertIsNone(tracker.pending_plan)
        self.assertIsNone(tracker.pending_since)
        self.assertEqual(tracker.suppressed_plan, "fp_bad")
        tracker.save.assert_called_once()

    def test_timeout_returns_false(self):
        args = MagicMock(lcd="lcd", rpc="rpc", tx_timeout_seconds=5, tx_poll_seconds=1)
        tracker = MagicMock(pending_hash="hash3", pending_plan="fp")
        query_tx = MagicMock(return_value=None)
        sleep = MagicMock()

        with patch("keeper.time.monotonic", side_effect=[0, 10]):
            result = poll_pending(args, tracker, query_tx=query_tx, sleep=sleep)

        self.assertFalse(result)
        self.assertEqual(tracker.pending_hash, "hash3")
        tracker.save.assert_not_called()

    def test_polls_multiple_times_before_success(self):
        args = MagicMock(lcd="lcd", rpc="rpc", tx_timeout_seconds=30, tx_poll_seconds=1)
        tracker = MagicMock(pending_hash="hash4", pending_plan="fp4")
        query_tx = MagicMock(side_effect=[None, None, {"code": 0, "raw_log": "", "height": "55"}])
        sleep = MagicMock()

        result = poll_pending(args, tracker, query_tx=query_tx, sleep=sleep)

        self.assertTrue(result)
        self.assertEqual(query_tx.call_count, 3)
        self.assertEqual(sleep.call_count, 2)


def _make_args(**overrides):
    base = dict(
        broadcast=True,
        lcd="lcd",
        rpc="rpc",
        vault="vault",
        tx_timeout_seconds=60,
        tx_poll_seconds=1,
        poll_seconds=15,
        deadline_seconds=120,
        terrad="terrad",
        key="test1",
        keyring_backend="test",
        chain_id="localterra",
        gas_adjustment="1.4",
        gas_prices="28.325uluna",
        state_file=".keeper-state.json",
        once=False,
    )
    base.update(overrides)
    return MagicMock(**base)


class RunOnceTests(unittest.TestCase):
    def test_polls_pending_when_tracker_has_hash(self):
        args = _make_args()
        tracker = MagicMock(pending_hash="hash1")

        with patch("keeper.poll_pending") as mock_poll:
            with patch("keeper.smart_query") as mock_query:
                run_once(args, tracker)

        mock_poll.assert_called_once_with(args, tracker)
        mock_query.assert_not_called()

    def test_no_rebalance_when_should_rebalance_is_false(self):
        args = _make_args()
        tracker = MagicMock(pending_hash=None)

        plan = {"should_rebalance": False, "price_deviation_bps": 50}
        with patch("keeper.smart_query", return_value=plan) as mock_query:
            run_once(args, tracker)

        tracker.save.assert_called_once()
        self.assertIsNone(tracker.suppressed_plan)

    def test_skips_when_suppressed_plan_matches(self):
        args = _make_args(broadcast=False)
        tracker = MagicMock(pending_hash=None)

        plan = {
            "should_rebalance": True,
            "offer_token": "token1",
            "captured_twap": "1.0",
            "balances": ["100", "200"],
            "reference_price": "1.0",
        }

        with patch("keeper.smart_query", return_value=plan):
            with patch("keeper.time.time", return_value=1000):
                with patch("builtins.print"):
                    run_once(args, tracker)

        fp = plan_fingerprint(plan, {"rebalance": {"deadline": 1000 + 120}})
        tracker.suppressed_plan = fp
        tracker.save.reset_mock()

        with patch("keeper.smart_query", return_value=plan):
            with patch("keeper.time.time", return_value=1000):
                with patch("builtins.print") as mock_print:
                    run_once(args, tracker)

        mock_print.assert_any_call(
            "rebalance suppressed after deterministic failure; plan is unchanged"
        )

    def test_dry_run_prints_and_returns(self):
        args = _make_args(broadcast=False)
        tracker = MagicMock(pending_hash=None, suppressed_plan=None)
        plan = {
            "should_rebalance": True,
            "offer_token": "token1",
            "captured_twap": "1.5",
            "balances": ["100", "100"],
            "reference_price": "1.5",
        }

        with patch("keeper.smart_query", return_value=plan):
            with patch("keeper.time.time", return_value=500):
                with patch("builtins.print") as mock_print:
                    run_once(args, tracker)

        mock_print.assert_any_call('{\n  "rebalance": {\n    "deadline": 620\n  }\n}')

    def test_broadcast_success_path(self):
        args = _make_args()
        tracker = MagicMock(pending_hash=None, suppressed_plan=None)
        plan = {
            "should_rebalance": True,
            "offer_token": "token1",
            "captured_twap": "1.5",
            "balances": ["100", "100"],
            "reference_price": "1.5",
        }

        with patch("keeper.smart_query", return_value=plan):
            with patch("keeper.time.time", return_value=500):
                with patch("keeper.time.monotonic", return_value=100.0):
                    with patch("keeper.preflight") as mock_pre:
                        with patch("keeper.broadcast", return_value="txhash1") as mock_bc:
                            with patch("keeper.poll_pending") as mock_poll:
                                run_once(args, tracker)

        mock_pre.assert_called_once_with(args.vault, {"rebalance": {"deadline": 620}}, args)
        mock_bc.assert_called_once_with(args.vault, {"rebalance": {"deadline": 620}}, args)
        self.assertEqual(tracker.pending_hash, "txhash1")
        self.assertEqual(tracker.pending_since, 100.0)
        tracker.save.assert_called()
        mock_poll.assert_called_once_with(args, tracker)

    def test_broadcast_suppresses_on_deterministic_failure(self):
        args = _make_args()
        tracker = MagicMock(pending_hash=None, suppressed_plan=None)
        plan = {
            "should_rebalance": True,
            "offer_token": "token1",
            "captured_twap": "2.0",
            "balances": ["100", "100"],
            "reference_price": "2.0",
        }

        with patch("keeper.smart_query", return_value=plan):
            with patch("keeper.time.time", return_value=500):
                with patch("keeper.preflight"):
                    with patch("keeper.broadcast", side_effect=DeterministicTxError("CheckTx code 13: failed")):
                        with self.assertRaises(DeterministicTxError):
                            run_once(args, tracker)

        self.assertIsNotNone(tracker.suppressed_plan)
        tracker.save.assert_called()

    def test_sync_reference_broadcast_path(self):
        args = _make_args()
        tracker = MagicMock(pending_hash=None, suppressed_plan=None)
        plan = {
            "should_rebalance": True,
            "offer_token": None,
            "captured_twap": "1.0",
            "balances": ["100", "100"],
            "reference_price": "1.0",
        }

        with patch("keeper.smart_query", return_value=plan):
            with patch("keeper.time.time", return_value=1000):
                with patch("keeper.preflight") as mock_pre:
                    with patch("keeper.broadcast", return_value="txhash_sync") as mock_bc:
                        with patch("keeper.poll_pending"):
                            with patch("keeper.time.monotonic", return_value=42.0):
                                run_once(args, tracker)

        mock_pre.assert_called_once_with(args.vault, {"sync_reference": {}}, args)
        mock_bc.assert_called_once_with(args.vault, {"sync_reference": {}}, args)
        self.assertEqual(tracker.pending_hash, "txhash_sync")


class GetJsonTests(unittest.TestCase):
    @patch("keeper.urllib.request.urlopen")
    def test_returns_parsed_json(self, mock_urlopen):
        mock_response = MagicMock()
        mock_response.read.return_value = b'{"key": "value"}'
        mock_response.__enter__.return_value = mock_response
        mock_urlopen.return_value = mock_response

        result = get_json("http://example.com")
        self.assertEqual(result, {"key": "value"})

    @patch("keeper.urllib.request.urlopen")
    def test_passes_url_to_urlopen(self, mock_urlopen):
        mock_response = MagicMock()
        mock_response.read.return_value = b"null"
        mock_response.__enter__.return_value = mock_response
        mock_urlopen.return_value = mock_response

        get_json("http://test.url/data")
        args, kwargs = mock_urlopen.call_args
        self.assertEqual(kwargs["timeout"], 15)


class SmartQueryTests(unittest.TestCase):
    @patch("keeper.get_json")
    def test_encodes_and_queries(self, mock_get_json):
        mock_get_json.return_value = {"data": {"result": "ok"}}
        result = smart_query("http://lcd", "contract1", {"query": {}})
        self.assertEqual(result, {"result": "ok"})
        url = mock_get_json.call_args[0][0]
        self.assertIn("cosmwasm/wasm/v1/contract/contract1/smart/", url)


class TxCommandTests(unittest.TestCase):
    def test_returns_command_list(self):
        args = _make_args()
        cmd = tx_command("vault1", {"rebalance": {"deadline": 100}}, args)
        self.assertIn("terrad", cmd)
        self.assertIn("vault1", cmd)
        self.assertIn(json.dumps({"rebalance": {"deadline": 100}}, separators=(",", ":")), cmd)
        for flag in ["--from", "--keyring-backend", "--chain-id", "--node", "--gas-prices"]:
            self.assertIn(flag, cmd)


class RunCommandTests(unittest.TestCase):
    @patch("keeper.subprocess.run")
    def test_raises_on_nonzero_exit(self, mock_run):
        mock_run.return_value = MagicMock(returncode=1, stderr="error!", stdout="")
        with self.assertRaises(DeterministicTxError):
            run_command(["some", "command"])

    @patch("keeper.subprocess.run")
    def test_returns_result_on_success(self, mock_run):
        mock_run.return_value = MagicMock(returncode=0, stdout='{"ok": true}', stderr="")
        result = run_command(["some", "command"])
        self.assertEqual(result.stdout, '{"ok": true}')


class BroadcastTests(unittest.TestCase):
    @patch("keeper.run_command")
    def test_returns_tx_hash_on_success(self, mock_run):
        mock_run.return_value = MagicMock(
            stdout=json.dumps({"txhash": "txhash1", "code": 0})
        )
        result = broadcast("vault1", {}, MagicMock())
        self.assertEqual(result, "txhash1")

    @patch("keeper.run_command")
    def test_uses_nested_tx_response(self, mock_run):
        mock_run.return_value = MagicMock(
            stdout=json.dumps({"tx_response": {"txhash": "txhash2", "code": 0}})
        )
        result = broadcast("vault1", {}, MagicMock())
        self.assertEqual(result, "txhash2")

    @patch("keeper.run_command")
    def test_raises_deterministic_error_on_checktx_failure(self, mock_run):
        mock_run.return_value = MagicMock(
            stdout=json.dumps({"code": 4, "raw_log": "mempool full"})
        )
        with self.assertRaises(DeterministicTxError) as ctx:
            broadcast("vault1", {}, MagicMock())
        self.assertIn("mempool full", str(ctx.exception))

    @patch("keeper.run_command")
    def test_raises_runtime_error_on_invalid_json(self, mock_run):
        mock_run.return_value = MagicMock(stdout="not json", stderr="")
        with self.assertRaises(RuntimeError):
            broadcast("vault1", {}, MagicMock())

    @patch("keeper.run_command")
    def test_raises_runtime_on_missing_tx_hash(self, mock_run):
        mock_run.return_value = MagicMock(
            stdout=json.dumps({"code": 0})
        )
        with self.assertRaises(RuntimeError) as ctx:
            broadcast("vault1", {}, MagicMock())
        self.assertIn("no transaction hash", str(ctx.exception))


class PreflightTests(unittest.TestCase):
    @patch("keeper.tx_command")
    @patch("keeper.run_command")
    def test_appends_dry_run_flag(self, mock_run, mock_tx):
        mock_tx.return_value = ["terrad", "tx", "wasm", "execute", "vault1", "msg"]
        mock_run.return_value = MagicMock(stdout="", stderr="")
        preflight("vault1", {}, MagicMock())
        mock_run.assert_called_once()
        self.assertIn("--dry-run", mock_run.call_args[0][0])


class ParseArgsTests(unittest.TestCase):
    def test_vault_is_required(self):
        with self.assertRaises(SystemExit):
            parse_args()

    def test_rejects_non_positive_deadline(self):
        with patch("sys.argv", ["keeper.py", "--vault", "vault1", "--deadline-seconds", "0"]):
            with self.assertRaises(SystemExit):
                parse_args()

    def test_minimal_valid_args(self):
        with patch("sys.argv", ["keeper.py", "--vault", "vault1"]):
            args = parse_args()
        self.assertEqual(args.vault, "vault1")
        self.assertEqual(args.deadline_seconds, 120)
        self.assertEqual(args.tx_timeout_seconds, 60.0)
        self.assertEqual(args.tx_poll_seconds, 2.0)
        self.assertEqual(args.poll_seconds, 15)
        self.assertFalse(args.broadcast)
        self.assertFalse(args.once)

    def test_accepts_float_tx_poll(self):
        with patch("sys.argv", ["keeper.py", "--vault", "vault1", "--tx-poll-seconds", "0.5"]):
            args = parse_args()
        self.assertEqual(args.tx_poll_seconds, 0.5)

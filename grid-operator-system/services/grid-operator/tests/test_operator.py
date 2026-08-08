import base64
import copy
import json
import tempfile
import unittest
from unittest.mock import patch
from pathlib import Path

from grid_operator.db import Database
from grid_operator.config import Config
from grid_operator.indexer import Indexer, ReorgError
from grid_operator.keeper import Keeper
from grid_operator.rpc import RpcError, Terrad
from grid_operator.reliability import StateIdentityError
from grid_operator.cli import resolve_intervention


FIXTURES = Path(__file__).parent / "fixtures"


def fixture(name):
    return json.loads((FIXTURES / name).read_text())


def attrs(**values):
    return {"type": "wasm", "attributes": [{"key": key, "value": str(value)} for key, value in values.items()]}


def fill(pair="pairA", order_id=7, side="ask", amount0=10, amount1=20, maker="vault1"):
    return attrs(_contract_address=pair, action="limit_order_fill", maker=maker,
                 order_id=order_id, side=side, token0_amount=amount0, token1_amount=amount1)


class ConfigTests(unittest.TestCase):
    def valid_environment(self):
        return {"GRID_RPC_URL": "http://rpc", "GRID_CHAIN_ID": "chain"}

    def test_rejects_zero_poll_timeout_and_loop_values(self):
        for name in ("GRID_TX_POLL_SECONDS", "GRID_TX_TIMEOUT_SECONDS", "GRID_LOOP_SECONDS"):
            with patch.dict("os.environ", {**self.valid_environment(), name: "0"}, clear=True):
                with self.assertRaises(ValueError, msg=name):
                    Config.from_env()

    def test_external_signer_protocol_uses_argv_and_strict_json(self):
        unsigned = {"body": {}, "auth_info": {}, "signatures": []}
        signed = base64.b64encode(json.dumps(unsigned).encode()).decode()
        responses = [
            MagicResult(b'{"address":"terra1keeper"}'),
            MagicResult(b'{"address":"terra1keeper"}'),
            MagicResult(json.dumps(unsigned).encode()),
            MagicResult(json.dumps({"signed_tx": signed}).encode()),
        ]
        terrad = Terrad("terrad", "rpc", "chain", "key", "os",
                        signer_command=("/signer", "--profile", "grid"))
        with patch("grid_operator.rpc.subprocess.run", side_effect=responses) as run:
            self.assertEqual(terrad.key_address(), "terra1keeper")
            terrad.sign_execute("vault", {"reconcile": {}})
        signer_calls = [call for call in run.call_args_list if call.args[0][0] == "/signer"]
        self.assertTrue(signer_calls)
        self.assertTrue(all(call.args[0] == ["/signer", "--profile", "grid"] for call in signer_calls))
        request = json.loads(signer_calls[-1].kwargs["input"])
        self.assertEqual(set(request), {"version", "action", "chain_id", "signer", "unsigned_tx"})

    def test_rejects_invalid_deployment_and_negative_finality(self):
        for name, value in (("GRID_DEPLOYMENT_HEIGHT", "0"), ("GRID_FINALITY_DEPTH", "-1")):
            with patch.dict("os.environ", {**self.valid_environment(), name: value}, clear=True):
                with self.assertRaises(ValueError, msg=name):
                    Config.from_env()


class MagicResult:
    def __init__(self, stdout, returncode=0):
        self.stdout, self.stderr, self.returncode = stdout, b"", returncode

class FakeRPC:
    def __init__(self, blocks, results, latest=None):
        self.blocks, self.results = blocks, results
        self.latest = latest if latest is not None else max(blocks)

    def latest_height(self):
        return self.latest

    def block(self, height):
        return copy.deepcopy(self.blocks[height])

    def block_results(self, height):
        return copy.deepcopy(self.results[height])


def response(height, events, tx=b"tx"):
    parent = f"HASH{height - 1}"
    block = {"block_id": {"hash": f"HASH{height}"}, "block": {
        "header": {"height": str(height), "last_block_id": {"hash": parent}},
        "data": {"txs": [base64.b64encode(tx).decode()]}}}
    result = {"txs_results": [{"code": 0, "events": events}]}
    return block, result


class IndexerTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.path = Path(self.temp.name) / "db.sqlite"
        self.db = Database(self.path)
        self.db.migrate()
        self.db.conn.execute(
            "INSERT INTO vaults(address,bot_id,pair_address) VALUES('vault1',1,'pairA')"
        )
        self.db.conn.execute(
            "INSERT INTO orders(pair_address,order_id,vault_address,bot_id,side,active) "
            "VALUES('pairA',7,'vault1',1,'ask',1)"
        )

    def tearDown(self):
        self.db.conn.close()
        self.temp.cleanup()

    def indexer(self, rpc, depth=0, start=10):
        return Indexer(self.db, rpc, "chain", start, ("vault1",), depth)

    def test_fixture_duplicate_ingestion_is_idempotent_and_hash_is_computed(self):
        block = fixture("block_10.json")
        result = fixture("block_results_10.json")
        rpc = FakeRPC({10: block}, {10: result})
        indexer = self.indexer(rpc)
        self.assertEqual(indexer.scan(), 1)
        indexer._scan_height(10)
        row = self.db.conn.execute("SELECT tx_hash,input_amount,output_amount FROM raw_events").fetchone()
        import hashlib
        self.assertEqual(row["tx_hash"], hashlib.sha256(b"tx-10").hexdigest().upper())
        self.assertEqual((row["input_amount"], row["output_amount"]), ("10", "20"))
        self.assertEqual(self.db.conn.execute("SELECT COUNT(*) FROM raw_events").fetchone()[0], 1)

    def test_fill_from_nonconfigured_pair_is_rejected(self):
        block, result = response(10, [fill("pairA", 7), fill("pairB", 7, "bid", 3, 4)])
        self.indexer(FakeRPC({10: block}, {10: result})).scan()
        rows = self.db.conn.execute("SELECT pair_address,input_amount,output_amount FROM aggregates ORDER BY pair_address").fetchall()
        self.assertEqual([tuple(row) for row in rows], [("pairA", "10", "20")])

    def test_forged_fill_from_non_pair_contract_is_rejected(self):
        block, result = response(10, [fill("attacker", 7)])
        self.indexer(FakeRPC({10: block}, {10: result})).scan()
        self.assertEqual(self.db.conn.execute("SELECT COUNT(*) FROM raw_events").fetchone()[0], 0)

    def test_fill_for_unknown_order_is_rejected(self):
        block, result = response(10, [fill("pairA", 999)])
        self.indexer(FakeRPC({10: block}, {10: result})).scan()
        self.assertEqual(self.db.conn.execute("SELECT COUNT(*) FROM raw_events").fetchone()[0], 0)

    def test_partial_and_terminal_fills_rebuild_exact_aggregate(self):
        b10, r10 = response(10, [fill(amount0=3, amount1=5)], b"one")
        b11, r11 = response(11, [fill(amount0=7, amount1=11)], b"two")
        rpc = FakeRPC({10: b10, 11: b11}, {10: r10, 11: r11})
        self.indexer(rpc).scan()
        row = self.db.conn.execute("SELECT * FROM aggregates").fetchone()
        self.assertEqual((row["input_amount"], row["output_amount"], row["fill_count"], row["through_height"]),
                         ("10", "16", 2, 11))
        self.db.conn.execute("DELETE FROM aggregates")
        self.db.rebuild_aggregates()
        self.assertEqual(self.db.conn.execute("SELECT fill_count FROM aggregates").fetchone()[0], 2)

    def test_finality_depth_excludes_chain_tip(self):
        blocks, results = {}, {}
        for height in (10, 11, 12):
            blocks[height], results[height] = response(height, [fill(order_id=height)], str(height).encode())
        indexer = self.indexer(FakeRPC(blocks, results, latest=12), depth=2)
        self.assertEqual(indexer.scan(), 1)
        self.assertEqual(self.db.cursor("scanned"), 10)

    def test_restart_continues_after_durable_scan_cursor(self):
        b10, r10 = response(10, [fill(order_id=10)], b"10")
        first = self.indexer(FakeRPC({10: b10}, {10: r10}))
        first.scan()
        b11, r11 = response(11, [fill(order_id=11)], b"11")
        restarted = Indexer(Database(self.path), FakeRPC({10: b10, 11: b11}, {10: r10, 11: r11}),
                            "chain", 10, ("vault1",), 0)
        restarted.db.migrate()
        self.assertEqual(restarted.scan(), 1)
        self.assertEqual(restarted.db.cursor("scanned"), 11)
        restarted.db.conn.close()

    def test_observed_finalized_reorg_stops_scanning(self):
        b10, r10 = response(10, [fill()], b"10")
        rpc = FakeRPC({10: b10}, {10: r10})
        indexer = self.indexer(rpc)
        indexer.scan()
        rpc.blocks[10]["block_id"]["hash"] = "REORGED"
        with self.assertRaises(ReorgError):
            indexer.scan()

    def test_manager_placement_event_maps_pair_local_order(self):
        events = [attrs(_contract_address="vault1", action="record_grid_orders", bot_id=1),
                  attrs(_contract_address="pairA", action="place_limit_order", limit_order_placed=99, side="bid")]
        block, result = response(10, events)
        self.indexer(FakeRPC({10: block}, {10: result})).scan()
        row = self.db.conn.execute("SELECT vault_address,side FROM orders WHERE pair_address='pairA' AND order_id=99").fetchone()
        self.assertEqual(tuple(row), ("vault1", "bid"))

    def test_vault_discovery_uses_bot_one_and_refreshes_active_orders(self):
        class Queries:
            def smart_query(self, contract, message):
                if "bot" in message:
                    self.assert_bot = message["bot"]["bot_id"]
                    return {"pair": "pairA"}
                return [{"order_id": 4, "rung_index": 2, "side": "Ask", "price": "1.5", "remaining": "9"}]
        queries = Queries()
        indexer = self.indexer(FakeRPC({}, {}, latest=0))
        indexer.register_vaults()
        indexer.refresh_orders(queries, "vault1", 12)
        row = self.db.conn.execute(
            "SELECT pair_address,bot_id,side,remaining FROM orders WHERE order_id=4"
        ).fetchone()
        self.assertEqual(queries.assert_bot, 1)
        self.assertEqual(tuple(row), ("pairA", 1, "ask", "9"))

    def test_migration_enables_wal_and_required_tables(self):
        self.assertEqual(self.db.conn.execute("PRAGMA journal_mode").fetchone()[0], "wal")
        tables = {row[0] for row in self.db.conn.execute("SELECT name FROM sqlite_master WHERE type='table'")}
        self.assertTrue({"blocks", "raw_events", "vaults", "orders", "aggregates", "batches",
                         "tx_attempts", "cursors", "discovered_vaults"}.issubset(tables))
        batch_columns = {
            row[1] for row in self.db.conn.execute("PRAGMA table_info(batches)")
        }
        self.assertTrue({"failure_count", "next_retry_at"}.issubset(batch_columns))
        self.assertIn("operator_audit", tables)
        self.assertEqual(self.db.conn.execute("PRAGMA user_version").fetchone()[0], 5)

    def test_schema_one_database_migrates_retry_columns(self):
        legacy_path = Path(self.temp.name) / "legacy.sqlite"
        legacy = Database(legacy_path)
        legacy.conn.execute(
            "CREATE TABLE batches (id INTEGER PRIMARY KEY, vault_address TEXT NOT NULL, "
            "bot_id INTEGER NOT NULL, through_height INTEGER NOT NULL, state TEXT NOT NULL, "
            "created_at INTEGER NOT NULL, confirmed_at INTEGER, tx_hash TEXT, error TEXT)"
        )
        legacy.conn.execute("PRAGMA user_version = 1")
        legacy.migrate()
        columns = {row[1] for row in legacy.conn.execute("PRAGMA table_info(batches)")}
        self.assertTrue({"failure_count", "next_retry_at"}.issubset(columns))
        self.assertEqual(legacy.conn.execute("PRAGMA user_version").fetchone()[0], 5)
        tables = {row[0] for row in legacy.conn.execute("SELECT name FROM sqlite_master WHERE type='table'")}
        self.assertIn("discovered_vaults", tables)
        legacy.conn.close()

    def test_schema_v2_database_migrates_discovered_vaults(self):
        legacy_path = Path(self.temp.name) / "v2.sqlite"
        legacy = Database(legacy_path)
        legacy.conn.executescript(
            "CREATE TABLE blocks (chain_id TEXT NOT NULL, height INTEGER NOT NULL, hash TEXT NOT NULL, "
            "parent_hash TEXT, time TEXT, scanned_at INTEGER NOT NULL, PRIMARY KEY(chain_id, height));"
            "CREATE TABLE vaults (address TEXT PRIMARY KEY, bot_id INTEGER NOT NULL DEFAULT 1, "
            "pair_address TEXT, enabled INTEGER NOT NULL DEFAULT 1, last_order_refresh_height INTEGER);"
            "CREATE TABLE orders (pair_address TEXT NOT NULL, order_id INTEGER NOT NULL, vault_address TEXT NOT NULL, "
            "bot_id INTEGER NOT NULL DEFAULT 1, side TEXT, rung_index INTEGER, price TEXT, remaining TEXT, "
            "active INTEGER NOT NULL DEFAULT 1, updated_height INTEGER, "
            "PRIMARY KEY(pair_address, order_id));"
            "CREATE TABLE raw_events (id INTEGER PRIMARY KEY, chain_id TEXT NOT NULL, height INTEGER NOT NULL, "
            "block_hash TEXT NOT NULL, tx_hash TEXT NOT NULL, tx_index INTEGER NOT NULL, event_index INTEGER NOT NULL, "
            "pair_address TEXT NOT NULL, order_id INTEGER NOT NULL, vault_address TEXT NOT NULL, bot_id INTEGER NOT NULL, "
            "side TEXT NOT NULL, input_amount TEXT NOT NULL, output_amount TEXT NOT NULL, raw_json TEXT NOT NULL, "
            "reconciled_batch_id INTEGER);"
            "CREATE TABLE aggregates (pair_address TEXT NOT NULL, order_id INTEGER NOT NULL, vault_address TEXT NOT NULL, "
            "bot_id INTEGER NOT NULL, input_amount TEXT NOT NULL, output_amount TEXT NOT NULL, fill_count INTEGER NOT NULL, "
            "first_height INTEGER NOT NULL, through_height INTEGER NOT NULL, PRIMARY KEY(pair_address, order_id));"
            "CREATE TABLE batches (id INTEGER PRIMARY KEY, vault_address TEXT NOT NULL, bot_id INTEGER NOT NULL, "
            "through_height INTEGER NOT NULL, state TEXT NOT NULL, created_at INTEGER NOT NULL, "
            "confirmed_at INTEGER, tx_hash TEXT, error TEXT, failure_count INTEGER NOT NULL DEFAULT 0, "
            "next_retry_at INTEGER);"
            "CREATE TABLE batch_items (batch_id INTEGER NOT NULL, pair_address TEXT NOT NULL, order_id INTEGER NOT NULL, "
            "input_amount TEXT NOT NULL, output_amount TEXT NOT NULL, fill_count INTEGER NOT NULL, "
            "PRIMARY KEY(batch_id, pair_address, order_id));"
            "CREATE TABLE batch_events (batch_id INTEGER NOT NULL, event_id INTEGER NOT NULL UNIQUE, "
            "PRIMARY KEY(batch_id, event_id));"
            "CREATE TABLE tx_attempts (id INTEGER PRIMARY KEY, batch_id INTEGER NOT NULL, state TEXT NOT NULL, "
            "signed_tx BLOB NOT NULL, tx_hash TEXT, check_code INTEGER, deliver_code INTEGER, "
            "created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, response_json TEXT, error TEXT);"
            "CREATE TABLE cursors (name TEXT PRIMARY KEY, height INTEGER NOT NULL, value TEXT, updated_at INTEGER NOT NULL);"
            "PRAGMA user_version = 2;"
        )
        legacy.migrate()
        tables = {row[0] for row in legacy.conn.execute("SELECT name FROM sqlite_master WHERE type='table'")}
        self.assertIn("discovered_vaults", tables)
        self.assertEqual(legacy.conn.execute("PRAGMA user_version").fetchone()[0], 5)
        legacy.conn.close()

    def test_database_identity_refuses_changed_chain_vault_or_signer(self):
        identity = {"schema_version": 1, "chain_id": "chain", "vaults": ["vault1"],
                    "signer": "os:key1", "protocol_kind": "limit-grid"}
        empty = Database(Path(self.temp.name) / "identity.sqlite")
        empty.migrate()
        empty.validate_identity(identity)
        for changed in ({**identity, "chain_id": "other"},
                        {**identity, "vaults": ["vault2"]},
                        {**identity, "signer": "os:key2"}):
            with self.assertRaises(StateIdentityError):
                empty.validate_identity(changed)
        empty.conn.close()


class FakeTerrad:
    def __init__(self, broadcast=None, queries=None, broadcast_error=None):
        self.broadcast_response = broadcast or {"code": 0, "txhash": "ABC"}
        self.queries = list(queries or [{"code": 0, "height": "20", "txhash": "ABC"}])
        self.broadcast_error = broadcast_error
        self.broadcasts = 0
        self.signs = 0

    def sign_execute(self, vault, message):
        self.signs += 1
        return json.dumps(message).encode()

    def broadcast(self, signed):
        self.broadcasts += 1
        if self.broadcast_error:
            raise self.broadcast_error
        return self.broadcast_response

    def query_tx(self, tx_hash):
        if not self.queries:
            raise RpcError("not found")
        item = self.queries.pop(0)
        if isinstance(item, Exception):
            raise item
        return item


class KeeperTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.db = Database(Path(self.temp.name) / "db.sqlite")
        self.db.migrate()
        self.db.conn.execute("INSERT INTO vaults(address,bot_id,pair_address) VALUES('vault1',1,'pairA')")
        self.add_event(1, 10, "pairA", 7, "3", "5")

    def tearDown(self):
        self.db.conn.close()
        self.temp.cleanup()

    def add_event(self, event_id, height, pair, order_id, amount_in, amount_out):
        self.db.conn.execute(
            "INSERT INTO raw_events(id,chain_id,height,block_hash,tx_hash,tx_index,event_index,pair_address,order_id," 
            "vault_address,bot_id,side,input_amount,output_amount,raw_json) VALUES(?, 'chain',?,'h',?,0,0,?,?, 'vault1',1,'ask',?,?, '{}')",
            (event_id, height, f"tx{event_id}", pair, order_id, amount_in, amount_out))
        self.db.rebuild_aggregates()

    def test_frozen_batch_is_bounded_and_survives_restart(self):
        self.add_event(2, 11, "pairA", 8, "2", "4")
        keeper = Keeper(self.db, FakeTerrad(), max_orders=1)
        batch = keeper.freeze_batch("vault1")
        restarted = Keeper(Database(self.db.path), FakeTerrad(), max_orders=1)
        restarted.db.migrate()
        self.assertEqual(restarted.freeze_batch("vault1"), batch)
        self.assertEqual(restarted.db.conn.execute("SELECT COUNT(*) FROM batch_items WHERE batch_id=?", (batch,)).fetchone()[0], 1)
        restarted.db.conn.close()

    def test_message_contains_only_order_ids(self):
        keeper = Keeper(self.db, FakeTerrad())
        _, message = keeper._message(keeper.freeze_batch("vault1"))
        self.assertEqual(message, {"reconcile": {"bot_id": 1, "order_ids": [7]}})
        self.assertNotIn("output_amount", json.dumps(message))

    def test_checktx_failure_does_not_advance_checkpoint(self):
        terrad = FakeTerrad(broadcast={"code": 12, "txhash": "BAD", "raw_log": "fee"})
        keeper = Keeper(self.db, terrad)
        result = keeper.process_batch(keeper.freeze_batch("vault1"))
        self.assertEqual(result, "check_failed")
        self.assertEqual(self.db.cursor("confirmed:vault1"), 0)
        self.assertIsNone(self.db.conn.execute("SELECT reconciled_batch_id FROM raw_events").fetchone()[0])

    def test_repeated_checktx_failure_enters_intervention_after_backoff(self):
        now = [100]
        terrad = FakeTerrad(broadcast={"code": 12, "txhash": "BAD", "raw_log": "fee"})
        keeper = Keeper(self.db, terrad, wall_clock=lambda: now[0])
        batch = keeper.freeze_batch("vault1")

        self.assertEqual(keeper.process_batch(batch), "check_failed")
        self.assertEqual(keeper.process_batch(batch), "backoff")
        now[0] += 60
        self.assertEqual(keeper.process_batch(batch), "check_failed")
        now[0] += 120
        self.assertEqual(keeper.process_batch(batch), "check_failed")
        self.assertEqual(keeper.process_batch(batch), "intervention")
        self.assertEqual(terrad.broadcasts, 3)
        row = self.db.conn.execute(
            "SELECT state,failure_count,next_retry_at FROM batches WHERE id=?", (batch,)
        ).fetchone()
        self.assertEqual(tuple(row), ("intervention", 3, None))

    def test_delivertx_failure_does_not_advance_checkpoint(self):
        terrad = FakeTerrad(queries=[{"code": 9, "height": "20", "txhash": "ABC", "raw_log": "reverted"}])
        keeper = Keeper(self.db, terrad)
        result = keeper.process_batch(keeper.freeze_batch("vault1"))
        self.assertEqual(result, "deliver_failed")
        self.assertEqual(self.db.cursor("confirmed:vault1"), 0)

    def test_reverted_grid_page_does_not_confirm_successful_transaction(self):
        transaction = {
            "code": 0,
            "height": "20",
            "txhash": "ABC",
            "logs": [{"events": [attrs(action="reverted_grid_page", bot_id=1)]}],
        }
        keeper = Keeper(self.db, FakeTerrad(queries=[transaction]))
        batch = keeper.freeze_batch("vault1")
        self.assertEqual(keeper.process_batch(batch), "page_reverted")
        self.assertEqual(self.db.cursor("confirmed:vault1"), 0)
        self.assertIsNone(
            self.db.conn.execute("SELECT reconciled_batch_id FROM raw_events").fetchone()[0]
        )
        self.assertEqual(
            self.db.conn.execute("SELECT state FROM batches WHERE id=?", (batch,)).fetchone()[0],
            "ready",
        )

    def test_timeout_then_eventual_inclusion_polls_without_rebroadcast(self):
        now = [0]
        def clock():
            return now[0]
        def sleep(seconds):
            now[0] += seconds
        terrad = FakeTerrad(queries=[RpcError("missing"), RpcError("missing")])
        keeper = Keeper(self.db, terrad, poll_seconds=1, timeout_seconds=1, sleep=sleep, clock=clock)
        batch = keeper.freeze_batch("vault1")
        self.assertEqual(keeper.process_batch(batch), "timeout")
        terrad.queries = [{"code": 0, "height": "21", "txhash": "ABC"}]
        self.assertEqual(keeper.process_batch(batch), "confirmed")
        self.assertEqual(terrad.broadcasts, 1)
        self.assertEqual(self.db.cursor("confirmed:vault1"), 10)

    def test_shallow_reorg_waits_for_confirmation_depth(self):
        heights = iter((21, 22))
        terrad = FakeTerrad(queries=[
            {"code": 0, "height": "20", "txhash": "ABC"},
            RpcError("temporarily missing"),
            {"code": 0, "height": "20", "txhash": "ABC"},
        ])
        keeper = Keeper(
            self.db,
            terrad,
            confirmation_blocks=2,
            latest_height=lambda: next(heights),
            sleep=lambda _: None,
        )

        self.assertEqual(keeper.process_batch(keeper.freeze_batch("vault1")), "confirmed")
        self.assertEqual(self.db.cursor("confirmed:vault1"), 10)
        self.assertEqual(terrad.broadcasts, 1)

    def test_ambiguous_broadcast_is_not_rebroadcast_after_restart(self):
        terrad = FakeTerrad(broadcast_error=RpcError("connection reset"))
        keeper = Keeper(self.db, terrad)
        batch = keeper.freeze_batch("vault1")
        self.assertEqual(keeper.process_batch(batch), "unknown")
        restarted_terrad = FakeTerrad()
        restarted = Keeper(Database(self.db.path), restarted_terrad)
        restarted.db.migrate()
        self.assertEqual(restarted.process_batch(batch), "unknown")
        self.assertEqual(restarted_terrad.broadcasts, 0)
        restarted.db.conn.close()

    def test_missing_broadcast_code_is_unknown_and_not_retried(self):
        terrad = FakeTerrad(broadcast={"txhash": "ABC"})
        keeper = Keeper(self.db, terrad)
        batch = keeper.freeze_batch("vault1")
        self.assertEqual(keeper.process_batch(batch), "unknown")
        self.assertEqual(keeper.process_batch(batch), "unknown")
        self.assertEqual(terrad.broadcasts, 1)

    def test_mismatched_polled_hash_preserves_pending_batch(self):
        now = [0]
        terrad = FakeTerrad(queries=[{"code": 0, "height": "20", "txhash": "OTHER"}])
        keeper = Keeper(
            self.db, terrad, poll_seconds=1, timeout_seconds=1,
            clock=lambda: now[0], sleep=lambda seconds: now.__setitem__(0, now[0] + seconds),
        )
        batch = keeper.freeze_batch("vault1")
        self.assertEqual(keeper.process_batch(batch), "timeout")
        self.assertIsNone(self.db.conn.execute("SELECT reconciled_batch_id FROM raw_events").fetchone()[0])

    def test_restart_in_broadcasting_crash_window_becomes_unknown(self):
        keeper = Keeper(self.db, FakeTerrad())
        batch = keeper.freeze_batch("vault1")
        now = 1
        self.db.conn.execute("UPDATE batches SET state='broadcasting' WHERE id=?", (batch,))
        self.db.conn.execute(
            "INSERT INTO tx_attempts(batch_id,state,signed_tx,created_at,updated_at) VALUES(?,'broadcasting',X'01',?,?)",
            (batch, now, now))
        terrad = FakeTerrad()
        self.assertEqual(Keeper(self.db, terrad).process_batch(batch), "unknown")
        self.assertEqual(terrad.broadcasts, 0)

    def test_checkpoint_and_event_confirmation_are_atomic(self):
        keeper = Keeper(self.db, FakeTerrad())
        batch = keeper.freeze_batch("vault1")
        original = self.db.set_cursor
        def fail(*args, **kwargs):
            raise RuntimeError("disk fault")
        self.db.set_cursor = fail
        with self.assertRaises(RuntimeError):
            keeper.process_batch(batch)
        self.db.set_cursor = original
        self.assertIsNone(self.db.conn.execute("SELECT reconciled_batch_id FROM raw_events").fetchone()[0])
        self.assertNotEqual(self.db.conn.execute("SELECT state FROM batches WHERE id=?", (batch,)).fetchone()[0], "confirmed")

    def test_success_refreshes_orders_and_aggregates_partial_fills(self):
        self.add_event(2, 11, "pairA", 7, "7", "11")
        called = []
        result = Keeper(self.db, FakeTerrad()).keep_once(("vault1",), called.append)
        self.assertEqual(result["vault1"], "confirmed")
        self.assertEqual(called, ["vault1"])
        self.assertEqual(self.db.conn.execute("SELECT COUNT(*) FROM aggregates").fetchone()[0], 0)

    def test_late_fill_before_confirm_is_consumed_exactly_once(self):
        terrad = FakeTerrad(queries=[{"code": 0, "height": "20", "txhash": "ABC"}])
        keeper = Keeper(self.db, terrad)
        batch = keeper.freeze_batch("vault1")
        self.add_event(2, 19, "pairA", 7, "7", "11")

        self.assertEqual(keeper.process_batch(batch), "confirmed")
        rows = self.db.conn.execute(
            "SELECT id,reconciled_batch_id FROM raw_events ORDER BY id"
        ).fetchall()
        self.assertEqual([(row["id"], row["reconciled_batch_id"]) for row in rows],
                         [(1, batch), (2, batch)])
        self.assertIsNone(keeper.freeze_batch("vault1"))

    def test_confirm_preserves_fill_after_execution_height(self):
        keeper = Keeper(self.db, FakeTerrad(queries=[{"code": 0, "height": "20", "txhash": "ABC"}]))
        batch = keeper.freeze_batch("vault1")
        self.add_event(2, 21, "pairA", 7, "7", "11")

        self.assertEqual(keeper.process_batch(batch), "confirmed")
        self.assertIsNotNone(keeper.freeze_batch("vault1"))
        self.assertIsNone(self.db.conn.execute(
            "SELECT reconciled_batch_id FROM raw_events WHERE id=2"
        ).fetchone()[0])

    def test_reason_specific_intervention_resolve_is_audited_and_backed_up(self):
        keeper = Keeper(self.db, FakeTerrad(broadcast={"code": 12, "txhash": "BAD"}),
                        wall_clock=lambda: 1000)
        batch = keeper.freeze_batch("vault1")
        for delay in (0, 60, 180):
            keeper.wall_clock = lambda delay=delay: 1000 + delay
            keeper.process_batch(batch)

        terrad = FakeTerrad()
        terrad.smart_query = lambda _vault, message: {"pair": "pairA"} if "bot" in message else []
        terrad.account_state = lambda: {"address": "keeper", "account_number": "1", "sequence": "9"}
        result = resolve_intervention(self.db, terrad, batch, "check_failed")
        self.assertTrue(Path(result["backup"]).exists())
        self.assertEqual(self.db.conn.execute("SELECT state FROM batches WHERE id=?", (batch,)).fetchone()[0],
                         "ready")
        self.assertEqual(self.db.conn.execute(
            "SELECT action FROM operator_audit ORDER BY id DESC LIMIT 1").fetchone()[0],
            "clear-intervention")

    def test_unknown_broadcast_cannot_use_intervention_resolve(self):
        batch = Keeper(self.db, FakeTerrad(broadcast_error=RpcError("reset"))).freeze_batch("vault1")
        Keeper(self.db, FakeTerrad(broadcast_error=RpcError("reset"))).process_batch(batch)
        with self.assertRaises(ValueError):
            resolve_intervention(self.db, FakeTerrad(), batch, "check_failed")

    def test_late_fill_is_not_marked_on_failure_or_unknown(self):
        for mode in ("failure", "unknown"):
            with self.subTest(mode=mode):
                path = Path(self.temp.name) / f"{mode}.sqlite"
                db = Database(path)
                db.migrate()
                db.conn.execute("INSERT INTO vaults(address,bot_id,pair_address) VALUES('vault1',1,'pairA')")
                db.conn.execute(
                    "INSERT INTO raw_events(id,chain_id,height,block_hash,tx_hash,tx_index,event_index,pair_address,order_id,"
                    "vault_address,bot_id,side,input_amount,output_amount,raw_json) "
                    "VALUES(1,'chain',10,'h','one',0,0,'pairA',7,'vault1',1,'ask','1','2','{}')")
                terrad = (FakeTerrad(broadcast={"code": 12, "txhash": "BAD"}) if mode == "failure"
                          else FakeTerrad(broadcast_error=RpcError("reset")))
                keeper = Keeper(db, terrad)
                batch = keeper.freeze_batch("vault1")
                db.conn.execute(
                    "INSERT INTO raw_events(id,chain_id,height,block_hash,tx_hash,tx_index,event_index,pair_address,order_id,"
                    "vault_address,bot_id,side,input_amount,output_amount,raw_json) "
                    "VALUES(2,'chain',11,'h','two',0,0,'pairA',7,'vault1',1,'ask','3','4','{}')")
                keeper.process_batch(batch)
                self.assertEqual(db.conn.execute(
                    "SELECT COUNT(*) FROM raw_events WHERE reconciled_batch_id IS NOT NULL").fetchone()[0], 0)
                db.conn.close()


if __name__ == "__main__":
    unittest.main()

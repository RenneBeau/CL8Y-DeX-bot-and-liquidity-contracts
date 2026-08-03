import base64
import copy
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import MagicMock, patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from grid_operator.db import Database
from grid_operator.discovery_keeper import DiscoveryKeeper
from grid_operator.indexer import Indexer
from grid_operator.protocol import rebalance_protocol


def attrs(**values):
    return {"type": "wasm", "attributes": [{"key": key, "value": str(value)} for key, value in values.items()]}


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


def instantiate(contract, code_id, height):
    return response(
        height,
        [attrs(_contract_address=contract, code_id=code_id, action="instantiate",
               owner="admin", pair="pairA")],
    )


class IndexerDiscoveryTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.path = Path(self.temp.name) / "db.sqlite"
        self.db = Database(self.path)
        self.db.migrate()

    def tearDown(self):
        self.db.conn.close()
        self.temp.cleanup()

    def indexer(self, rpc, code_ids, start=10):
        return Indexer(self.db, rpc, "chain", start, (), 0, code_ids=code_ids)

    def test_records_grid_instantiate_matching_code_id(self):
        block, result = instantiate("gridvault1", 7, 10)
        self.indexer(FakeRPC({10: block}, {10: result}), {"grid": "7"}).scan()
        row = self.db.conn.execute(
            "SELECT address,kind,discovered_height,enabled FROM discovered_vaults"
        ).fetchone()
        self.assertEqual(tuple(row), ("gridvault1", "grid", 10, 1))

    def test_records_rebalance_instantiate_matching_code_id(self):
        block, result = instantiate("botvault1", 9, 10)
        self.indexer(FakeRPC({10: block}, {10: result}), {"rebalance": "9"}).scan()
        row = self.db.conn.execute(
            "SELECT address,kind FROM discovered_vaults"
        ).fetchone()
        self.assertEqual(tuple(row), ("botvault1", "rebalance"))

    def test_ignores_non_matching_code_id(self):
        block, result = instantiate("gridvault1", 9, 10)
        self.indexer(FakeRPC({10: block}, {10: result}), {"grid": "7"}).scan()
        self.assertEqual(
            self.db.conn.execute("SELECT COUNT(*) FROM discovered_vaults").fetchone()[0], 0
        )

    def test_migrate_event_is_not_discovered(self):
        events = [attrs(_contract_address="gridvault1", code_id="7", action="migrate")]
        block, result = response(10, events)
        self.indexer(FakeRPC({10: block}, {10: result}), {"grid": "7"}).scan()
        self.assertEqual(
            self.db.conn.execute("SELECT COUNT(*) FROM discovered_vaults").fetchone()[0], 0
        )

    def test_discovery_is_idempotent_on_rescan(self):
        block, result = instantiate("gridvault1", 7, 10)
        indexer = self.indexer(FakeRPC({10: block}, {10: result}), {"grid": "7"})
        indexer.scan()
        indexer.scan()
        self.assertEqual(
            self.db.conn.execute("SELECT COUNT(*) FROM discovered_vaults").fetchone()[0], 1
        )

    def test_no_discovery_when_no_code_ids_configured(self):
        block, result = instantiate("gridvault1", 7, 10)
        Indexer(self.db, FakeRPC({10: block}, {10: result}), "chain", 10, (), 0).scan()
        self.assertEqual(
            self.db.conn.execute("SELECT COUNT(*) FROM discovered_vaults").fetchone()[0], 0
        )


class DiscoveryKeeperTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.db = Database(Path(self.temp.name) / "db.sqlite")
        self.db.migrate()
        self.db.conn.executemany(
            "INSERT INTO discovered_vaults(address,kind,discovered_height) VALUES(?,?,?)",
            [("gridvault1", "grid", 10), ("botvault1", "rebalance", 11), ("gridvault2", "grid", 12)],
        )
        self.args = type("Args", (), {
            "state_dir": str(Path(self.temp.name) / "state"),
            "once": False,
        })()
        self.terrad = MagicMock()

    def tearDown(self):
        self.db.conn.close()
        self.temp.cleanup()

    def test_keeps_every_discovered_vault_serially_with_protocol(self):
        keeper = DiscoveryKeeper(self.db, self.terrad, self.args)
        with patch("grid_operator.discovery_keeper.keep_vault") as mock_keep:
            keeper.run_once()
        self.assertEqual(mock_keep.call_count, 3)
        calls = [(call.args[3], call.args[4].kind) for call in mock_keep.call_args_list]
        self.assertEqual(calls, [
            ("gridvault1", "grid"),
            ("gridvault2", "grid"),
            ("botvault1", "rebalance"),
        ])

    def test_disabled_vault_is_skipped(self):
        self.db.conn.execute("UPDATE discovered_vaults SET enabled=0 WHERE address='gridvault2'")
        keeper = DiscoveryKeeper(self.db, self.terrad, self.args)
        with patch("grid_operator.discovery_keeper.keep_vault") as mock_keep:
            keeper.run_once()
        self.assertEqual(mock_keep.call_count, 2)
        kept = [call.args[3] for call in mock_keep.call_args_list]
        self.assertNotIn("gridvault2", kept)

    def test_per_vault_state_file_is_distinct(self):
        keeper = DiscoveryKeeper(self.db, self.terrad, self.args)
        first = keeper.tracker_for("gridvault1")
        second = keeper.tracker_for("gridvault2")
        self.assertNotEqual(first.path, second.path)
        self.assertTrue(second.path.endswith("gridvault2.json"))


class RebalanceProtocolTests(unittest.TestCase):
    def test_sync_reference_when_offer_token_is_none(self):
        plan = {"should_rebalance": True, "offer_token": None}
        self.assertEqual(
            rebalance_protocol.build_message(plan, 12345), {"sync_reference": {}}
        )

    def test_rebalance_when_offer_token_present(self):
        plan = {"should_rebalance": True, "offer_token": "tokenA"}
        self.assertEqual(
            rebalance_protocol.build_message(plan, 12345),
            {"rebalance": {"deadline": 12345}},
        )

    def test_none_when_not_required(self):
        self.assertIsNone(rebalance_protocol.build_message(
            {"should_rebalance": False}, 12345
        ))

    def test_fingerprint_is_plan_and_vault_sensitive(self):
        plan = {"should_rebalance": True, "offer_token": None, "price_deviation_bps": 700}
        args = type("Args", (), {"chain_id": "chain", "config_version": "1",
                                 "deadline_seconds": 120})()
        first = rebalance_protocol.fingerprint(
            plan, {"sync_reference": {}}, "vault1", args
        )
        second = rebalance_protocol.fingerprint(
            plan, {"sync_reference": {}}, "vault2", args
        )
        self.assertNotEqual(first, second)
        self.assertTrue(first.startswith("v2:"))


if __name__ == "__main__":
    unittest.main()

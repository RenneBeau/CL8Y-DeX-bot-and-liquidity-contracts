import base64
import hashlib
import json
import time

from .db import Database


class ReorgError(RuntimeError):
    pass


class EventError(RuntimeError):
    pass


def _text(value) -> str:
    if value is None:
        return ""
    value = str(value)
    try:
        decoded = base64.b64decode(value, validate=True).decode()
        if decoded and all(ch.isprintable() for ch in decoded):
            return decoded
    except (ValueError, UnicodeError):
        pass
    return value


def attributes(event: dict) -> dict[str, str]:
    return {_text(item.get("key")): _text(item.get("value")) for item in event.get("attributes", [])}


def _first(attrs: dict, *names: str) -> str:
    for name in names:
        if attrs.get(name) not in (None, ""):
            return attrs[name]
    return ""


class Indexer:
    def __init__(self, db: Database, rpc, chain_id: str, deployment_height: int,
                 vaults: tuple[str, ...], finality_depth: int,
                 code_ids: dict[str, str] | None = None):
        self.db, self.rpc, self.chain_id = db, rpc, chain_id
        self.deployment_height, self.vault_addresses = deployment_height, vaults
        self.finality_depth = finality_depth
        # kind -> on-chain code id (e.g. {"grid": "123", "rebalance": "456"}).
        self.code_ids = {kind: str(code_id) for kind, code_id in (code_ids or {}).items()}

    def register_vaults(self) -> None:
        self.db.conn.executemany(
            "INSERT INTO vaults(address,bot_id) VALUES(?,1) ON CONFLICT(address) DO UPDATE SET enabled=1",
            [(address,) for address in self.vault_addresses],
        )

    def refresh_orders(self, terrad, vault: str, height: int | None = None) -> None:
        bot = terrad.smart_query(vault, {"bot": {"bot_id": 1}})
        pair = bot["pair"]
        orders = terrad.smart_query(vault, {"orders": {"bot_id": 1}})
        if isinstance(orders, dict):
            orders = orders.get("orders", [])
        with self.db.transaction(immediate=True) as conn:
            conn.execute("UPDATE vaults SET pair_address=?,last_order_refresh_height=? WHERE address=?",
                         (pair, height, vault))
            seen = []
            for order in orders:
                order_id = int(order["order_id"])
                side = str(order["side"]).lower()
                conn.execute(
                    "INSERT INTO orders(pair_address,order_id,vault_address,bot_id,side,rung_index,price,remaining,active,updated_height) "
                    "VALUES(?,?,?,1,?,?,?,?,1,?) ON CONFLICT(pair_address,order_id) DO UPDATE SET "
                    "vault_address=excluded.vault_address,side=excluded.side,rung_index=excluded.rung_index," 
                    "price=excluded.price,remaining=excluded.remaining,active=1,updated_height=excluded.updated_height",
                    (pair, order_id, vault, side, order.get("rung_index"), str(order.get("price", "")),
                     str(order.get("remaining", "")), height),
                )
                seen.append(order_id)
            if seen:
                marks = ",".join("?" for _ in seen)
                conn.execute(f"UPDATE orders SET active=0,updated_height=? WHERE vault_address=? AND pair_address=? AND order_id NOT IN ({marks})",
                             (height, vault, pair, *seen))
            else:
                conn.execute("UPDATE orders SET active=0,updated_height=? WHERE vault_address=? AND pair_address=?",
                             (height, vault, pair))

    def _validate_observed_tip(self, scanned: int) -> None:
        if scanned < self.deployment_height:
            return
        row = self.db.conn.execute(
            "SELECT hash FROM blocks WHERE chain_id=? AND height=?", (self.chain_id, scanned)
        ).fetchone()
        if not row:
            raise ReorgError(f"scan cursor {scanned} has no block record")
        observed = self.rpc.block(scanned)["block_id"]["hash"]
        if observed != row["hash"]:
            raise ReorgError(f"finalized block {scanned} changed from {row['hash']} to {observed}")

    def scan(self, limit: int | None = None) -> int:
        self.register_vaults()
        scanned = self.db.cursor("scanned", self.deployment_height - 1)
        self._validate_observed_tip(scanned)
        finalized = max(0, self.rpc.latest_height() - self.finality_depth)
        stop = finalized if limit is None else min(finalized, scanned + limit)
        for height in range(max(scanned + 1, self.deployment_height), stop + 1):
            self._scan_height(height)
        return max(0, stop - scanned)

    def _scan_height(self, height: int) -> None:
        block_result = self.rpc.block(height)
        results = self.rpc.block_results(height)
        block = block_result["block"]
        block_hash = block_result["block_id"]["hash"]
        parent_hash = (block.get("header", {}).get("last_block_id") or {}).get("hash") or ""
        previous = self.db.conn.execute(
            "SELECT hash FROM blocks WHERE chain_id=? AND height=?", (self.chain_id, height - 1)
        ).fetchone()
        if previous and parent_hash and previous["hash"] != parent_hash:
            raise ReorgError(f"block {height} does not descend from indexed block {height - 1}")
        txs = (block.get("data") or {}).get("txs") or []
        tx_results = results.get("txs_results") or []
        if len(txs) != len(tx_results):
            raise EventError(f"block {height} transaction/result count mismatch")
        with self.db.transaction(immediate=True) as conn:
            existing = conn.execute("SELECT hash FROM blocks WHERE chain_id=? AND height=?",
                                    (self.chain_id, height)).fetchone()
            if existing and existing["hash"] != block_hash:
                raise ReorgError(f"block {height} hash changed")
            conn.execute("INSERT OR IGNORE INTO blocks VALUES(?,?,?,?,?,?)",
                         (self.chain_id, height, block_hash, parent_hash,
                          block.get("header", {}).get("time"), int(time.time())))
            for tx_index, (encoded_tx, tx_result) in enumerate(zip(txs, tx_results)):
                tx_hash = hashlib.sha256(base64.b64decode(encoded_tx)).hexdigest().upper()
                self._ingest_tx(conn, height, block_hash, tx_index, tx_hash, tx_result)
            self.db.set_cursor("scanned", height, block_hash, conn)
            self.db.rebuild_aggregates(conn)

    def _ingest_tx(self, conn, height: int, block_hash: str, tx_index: int,
                   tx_hash: str, tx_result: dict) -> None:
        if int(tx_result.get("code", 0)) != 0:
            return
        parsed = [(index, event, attributes(event)) for index, event in enumerate(tx_result.get("events") or [])]
        vault_set = set(self.vault_addresses)
        manager_events = [(i, a) for i, _, a in parsed
                          if _first(a, "_contract_address", "contract_address") in vault_set]
        # Pair placement IDs and manager record events coexist in the same successful tx.
        managers = {_first(a, "_contract_address", "contract_address") for _, a in manager_events
                    if a.get("action") in ("record_grid_orders", "create_grid_bot") and a.get("bot_id", "1") == "1"}
        if len(managers) == 1:
            vault = next(iter(managers))
            known_pair_row = conn.execute("SELECT pair_address FROM vaults WHERE address=?", (vault,)).fetchone()
            known_pair = known_pair_row[0] if known_pair_row else None
            for _, _, attrs in parsed:
                pair = _first(attrs, "_contract_address", "contract_address")
                order_id = _first(attrs, "limit_order_placed", "order_id")
                if order_id.isdigit() and pair and pair != vault and (not known_pair or pair == known_pair):
                    conn.execute(
                        "INSERT INTO orders(pair_address,order_id,vault_address,bot_id,side,active,updated_height) "
                        "VALUES(?,?,?,1,?,1,?) ON CONFLICT(pair_address,order_id) DO UPDATE SET "
                        "vault_address=excluded.vault_address,active=1,updated_height=excluded.updated_height",
                        (pair, int(order_id), vault, _first(attrs, "side", "order_side").lower() or None, height),
                    )
        for event_index, event, attrs in parsed:
            if attrs.get("action") != "limit_order_fill":
                continue
            pair = _first(attrs, "_contract_address", "contract_address", "pair")
            maker = _first(attrs, "maker", "owner")
            order_text = _first(attrs, "order_id", "limit_order_id")
            if maker not in vault_set or not pair or not order_text.isdigit():
                continue
            vault_row = conn.execute(
                "SELECT pair_address FROM vaults WHERE address=? AND enabled=1", (maker,)
            ).fetchone()
            if not vault_row or not vault_row["pair_address"] or vault_row["pair_address"] != pair:
                continue
            order_id = int(order_text)
            order = conn.execute(
                "SELECT vault_address,bot_id,side FROM orders "
                "WHERE pair_address=? AND order_id=? AND active=1",
                (pair, order_id),
            ).fetchone()
            if not order or order["vault_address"] != maker:
                continue
            vault = maker
            side = _first(attrs, "side", "order_side").lower() or order["side"]
            if side not in ("ask", "bid"):
                raise EventError(f"fill {pair}/{order_id} has no valid side")
            token0 = _first(attrs, "token0_amount", "token_0_amount", "amount0")
            token1 = _first(attrs, "token1_amount", "token_1_amount", "amount1")
            if not token0.isdigit() or not token1.isdigit():
                raise EventError(f"fill {pair}/{order_id} has invalid amounts")
            input_amount, output_amount = (token0, token1) if side == "ask" else (token1, token0)
            conn.execute("UPDATE orders SET side=?,updated_height=? WHERE pair_address=? AND order_id=?",
                         (side, height, pair, order_id))
            conn.execute(
                "INSERT OR IGNORE INTO raw_events(chain_id,height,block_hash,tx_hash,tx_index,event_index,pair_address,order_id," 
                "vault_address,bot_id,side,input_amount,output_amount,raw_json) VALUES(?,?,?,?,?,?,?,?,?,1,?,?,?,?)",
                (self.chain_id, height, block_hash, tx_hash, tx_index, event_index, pair, order_id,
                 vault, side, input_amount, output_amount, json.dumps(event, sort_keys=True, separators=(",", ":"))),
            )
        if self.code_ids:
            kind_by_code = {code_id: kind for kind, code_id in self.code_ids.items()}
            for _, _, attrs in parsed:
                if attrs.get("action") != "instantiate":
                    continue
                kind = kind_by_code.get(attrs.get("code_id"))
                if kind is None:
                    continue
                contract = _first(attrs, "_contract_address", "contract_address")
                if not contract:
                    continue
                conn.execute(
                    "INSERT INTO discovered_vaults(address,kind,discovered_height,enabled) "
                    "VALUES(?,?,?,1) ON CONFLICT(address) DO NOTHING",
                    (contract, kind, height),
                )

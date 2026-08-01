import json
import time

from .db import Database
from .rpc import RpcError


ACTIVE_BATCH_STATES = ("ready", "signed", "broadcasting", "broadcast", "timeout", "unknown")


def _has_reverted_grid_page(response: dict) -> bool:
    tx_response = response.get("tx_response", response)
    events = list(tx_response.get("events") or [])
    for log in tx_response.get("logs") or []:
        events.extend(log.get("events") or [])
    return any(
        attribute.get("key") == "action" and attribute.get("value") == "reverted_grid_page"
        for event in events
        for attribute in event.get("attributes") or []
    )


class Keeper:
    def __init__(self, db: Database, terrad, max_orders: int = 20,
                 poll_seconds: int = 6, timeout_seconds: int = 180,
                 sleep=time.sleep, clock=time.monotonic):
        if max_orders < 1:
            raise ValueError("max_orders must be positive")
        self.db, self.terrad, self.max_orders = db, terrad, max_orders
        self.poll_seconds, self.timeout_seconds = poll_seconds, timeout_seconds
        self.sleep, self.clock = sleep, clock

    def freeze_batch(self, vault: str) -> int | None:
        with self.db.transaction(immediate=True) as conn:
            existing = conn.execute(
                f"SELECT id FROM batches WHERE vault_address=? AND state IN ({','.join('?' * len(ACTIVE_BATCH_STATES))}) ORDER BY id LIMIT 1",
                (vault, *ACTIVE_BATCH_STATES),
            ).fetchone()
            if existing:
                return int(existing["id"])
            order_rows = conn.execute(
                "SELECT e.pair_address,e.order_id FROM raw_events e "
                "LEFT JOIN batch_events be ON be.event_id=e.id "
                "WHERE e.vault_address=? AND e.reconciled_batch_id IS NULL AND be.event_id IS NULL "
                "GROUP BY e.pair_address,e.order_id ORDER BY MIN(e.height),e.pair_address,e.order_id LIMIT ?",
                (vault, self.max_orders),
            ).fetchall()
            if not order_rows:
                return None
            events = []
            items = []
            through = 0
            for order in order_rows:
                rows = conn.execute(
                    "SELECT id,height,input_amount,output_amount FROM raw_events WHERE vault_address=? "
                    "AND pair_address=? AND order_id=? AND reconciled_batch_id IS NULL ORDER BY id",
                    (vault, order["pair_address"], order["order_id"]),
                ).fetchall()
                events.extend(int(row["id"]) for row in rows)
                through = max(through, *(int(row["height"]) for row in rows))
                items.append((order["pair_address"], order["order_id"],
                              str(sum(int(row["input_amount"]) for row in rows)),
                              str(sum(int(row["output_amount"]) for row in rows)), len(rows)))
            cursor = conn.execute(
                "INSERT INTO batches(vault_address,bot_id,through_height,state,created_at) VALUES(?,1,?,'ready',?)",
                (vault, through, int(time.time())),
            )
            batch_id = int(cursor.lastrowid)
            conn.executemany("INSERT INTO batch_items VALUES(?,?,?,?,?,?)",
                             [(batch_id, *item) for item in items])
            conn.executemany("INSERT INTO batch_events VALUES(?,?)", [(batch_id, event_id) for event_id in events])
            return batch_id

    def _message(self, batch_id: int) -> tuple[str, dict]:
        batch = self.db.conn.execute("SELECT * FROM batches WHERE id=?", (batch_id,)).fetchone()
        rows = self.db.conn.execute(
            "SELECT order_id FROM batch_items WHERE batch_id=? ORDER BY pair_address,order_id",
            (batch_id,),
        ).fetchall()
        return batch["vault_address"], {
            "reconcile": {"bot_id": 1, "order_ids": [row["order_id"] for row in rows]}
        }

    def _latest_attempt(self, batch_id: int):
        return self.db.conn.execute("SELECT * FROM tx_attempts WHERE batch_id=? ORDER BY id DESC LIMIT 1",
                                    (batch_id,)).fetchone()

    def process_batch(self, batch_id: int) -> str:
        batch = self.db.conn.execute("SELECT * FROM batches WHERE id=?", (batch_id,)).fetchone()
        if not batch:
            raise ValueError(f"unknown batch {batch_id}")
        if batch["state"] == "confirmed":
            return "confirmed"
        attempt = self._latest_attempt(batch_id)
        if batch["state"] == "broadcasting":
            # A previous process may have reached the node but died before saving its response.
            with self.db.transaction(immediate=True) as conn:
                conn.execute("UPDATE batches SET state='unknown',error='process restarted during broadcast' WHERE id=?",
                             (batch_id,))
                if attempt:
                    conn.execute("UPDATE tx_attempts SET state='unknown',error='process restarted during broadcast',updated_at=? WHERE id=?",
                                 (int(time.time()), attempt["id"]))
            return "unknown"
        if batch["state"] == "unknown":
            return "unknown"  # Operator intervention is required; never blindly rebroadcast.
        if attempt and attempt["tx_hash"] and attempt["state"] in ("broadcast", "timeout"):
            return self._poll(batch_id, int(attempt["id"]), attempt["tx_hash"])
        vault, message = self._message(batch_id)
        try:
            signed = self.terrad.sign_execute(vault, message)
        except Exception as exc:
            self.db.conn.execute("UPDATE batches SET error=? WHERE id=?", (str(exc), batch_id))
            return "sign_failed"
        now = int(time.time())
        with self.db.transaction(immediate=True) as conn:
            cursor = conn.execute(
                "INSERT INTO tx_attempts(batch_id,state,signed_tx,created_at,updated_at) VALUES(?,'signed',?,?,?)",
                (batch_id, signed, now, now),
            )
            attempt_id = int(cursor.lastrowid)
            conn.execute("UPDATE batches SET state='signed',error=NULL WHERE id=?", (batch_id,))
        # Persist 'broadcasting' before touching the network. A crash/timeout cannot cause an automatic replay.
        with self.db.transaction(immediate=True) as conn:
            conn.execute("UPDATE tx_attempts SET state='broadcasting',updated_at=? WHERE id=?", (int(time.time()), attempt_id))
            conn.execute("UPDATE batches SET state='broadcasting' WHERE id=?", (batch_id,))
        try:
            response = self.terrad.broadcast(signed)
        except Exception as exc:
            with self.db.transaction(immediate=True) as conn:
                conn.execute("UPDATE tx_attempts SET state='unknown',error=?,updated_at=? WHERE id=?",
                             (str(exc), int(time.time()), attempt_id))
                conn.execute("UPDATE batches SET state='unknown',error=? WHERE id=?", (str(exc), batch_id))
            return "unknown"
        tx_response = response.get("tx_response", response)
        code = int(tx_response.get("code", 0) or 0)
        tx_hash = tx_response.get("txhash") or tx_response.get("hash")
        with self.db.transaction(immediate=True) as conn:
            conn.execute("UPDATE tx_attempts SET tx_hash=?,check_code=?,response_json=?,updated_at=? WHERE id=?",
                         (tx_hash, code, json.dumps(response, sort_keys=True), int(time.time()), attempt_id))
            conn.execute("UPDATE batches SET tx_hash=? WHERE id=?", (tx_hash, batch_id))
            if code != 0 or not tx_hash:
                error = tx_response.get("raw_log") or tx_response.get("log") or "CheckTx failed or omitted tx hash"
                conn.execute("UPDATE tx_attempts SET state='check_failed',error=? WHERE id=?", (error, attempt_id))
                conn.execute("UPDATE batches SET state='ready',error=? WHERE id=?", (error, batch_id))
            else:
                conn.execute("UPDATE tx_attempts SET state='broadcast' WHERE id=?", (attempt_id,))
                conn.execute("UPDATE batches SET state='broadcast' WHERE id=?", (batch_id,))
        if code != 0 or not tx_hash:
            return "check_failed"
        return self._poll(batch_id, attempt_id, tx_hash)

    def _poll(self, batch_id: int, attempt_id: int, tx_hash: str) -> str:
        start = self.clock()
        while True:
            try:
                response = self.terrad.query_tx(tx_hash)
            except RpcError:
                response = None
            if response:
                tx_response = response.get("tx_response", response)
                code = int(tx_response.get("code", 0) or 0)
                if code == 0:
                    if _has_reverted_grid_page(response):
                        error = "grid cancel or claim page reverted"
                        with self.db.transaction(immediate=True) as conn:
                            conn.execute(
                                "UPDATE tx_attempts SET state='page_reverted',deliver_code=0,error=?,response_json=?,updated_at=? WHERE id=?",
                                (error, json.dumps(response, sort_keys=True), int(time.time()), attempt_id),
                            )
                            conn.execute("UPDATE batches SET state='ready',error=? WHERE id=?",
                                         (error, batch_id))
                        return "page_reverted"
                    self._confirm(batch_id, attempt_id, tx_hash, response)
                    return "confirmed"
                error = tx_response.get("raw_log") or tx_response.get("log") or "DeliverTx failed"
                with self.db.transaction(immediate=True) as conn:
                    conn.execute("UPDATE tx_attempts SET state='deliver_failed',deliver_code=?,error=?,response_json=?,updated_at=? WHERE id=?",
                                 (code, error, json.dumps(response, sort_keys=True), int(time.time()), attempt_id))
                    conn.execute("UPDATE batches SET state='ready',error=? WHERE id=?", (error, batch_id))
                return "deliver_failed"
            if self.clock() - start >= self.timeout_seconds:
                with self.db.transaction(immediate=True) as conn:
                    conn.execute("UPDATE tx_attempts SET state='timeout',updated_at=? WHERE id=?", (int(time.time()), attempt_id))
                    conn.execute("UPDATE batches SET state='timeout',error='inclusion polling timed out' WHERE id=?", (batch_id,))
                return "timeout"
            self.sleep(self.poll_seconds)

    def _confirm(self, batch_id: int, attempt_id: int, tx_hash: str, response: dict) -> None:
        with self.db.transaction(immediate=True) as conn:
            batch = conn.execute("SELECT vault_address,through_height FROM batches WHERE id=?", (batch_id,)).fetchone()
            conn.execute("UPDATE raw_events SET reconciled_batch_id=? WHERE id IN "
                         "(SELECT event_id FROM batch_events WHERE batch_id=?)", (batch_id, batch_id))
            conn.execute("UPDATE tx_attempts SET state='confirmed',deliver_code=0,response_json=?,updated_at=? WHERE id=?",
                         (json.dumps(response, sort_keys=True), int(time.time()), attempt_id))
            conn.execute("UPDATE batches SET state='confirmed',confirmed_at=?,tx_hash=?,error=NULL WHERE id=?",
                         (int(time.time()), tx_hash, batch_id))
            self.db.set_cursor(f"confirmed:{batch['vault_address']}", batch["through_height"], tx_hash, conn)
            self.db.rebuild_aggregates(conn)

    def keep_once(self, vaults: tuple[str, ...], refresh=None) -> dict[str, str]:
        results = {}
        # The loop is intentionally serial: one keeper key must never sign concurrently.
        for vault in vaults:
            batch_id = self.freeze_batch(vault)
            if batch_id is None:
                results[vault] = "idle"
                continue
            result = self.process_batch(batch_id)
            results[vault] = result
            if result == "confirmed" and refresh:
                refresh(vault)
        return results

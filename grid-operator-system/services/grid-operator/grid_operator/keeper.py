import json
import math
import time
from enum import Enum

from .db import Database
from .rpc import RpcError


ACTIVE_BATCH_STATES = (
    "ready", "signed", "broadcasting", "broadcast", "timeout", "unknown", "intervention"
)
MAX_FAILURES = 3
RETRY_BACKOFF_SECONDS = 60


class TxResultCode(str, Enum):
    CHECK_FAILED = "check_failed"
    DELIVER_FAILED = "deliver_failed"
    PAGE_REVERTED = "page_reverted"
    UNKNOWN_BROADCAST = "unknown_broadcast"


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
                 sleep=time.sleep, clock=time.monotonic, wall_clock=time.time,
                 confirmation_blocks: int = 0, latest_height=None):
        if max_orders < 1:
            raise ValueError("max_orders must be positive")
        if not math.isfinite(poll_seconds) or not math.isfinite(timeout_seconds) \
                or poll_seconds <= 0 or timeout_seconds <= 0:
            raise ValueError("poll_seconds and timeout_seconds must be positive")
        if confirmation_blocks < 0:
            raise ValueError("confirmation_blocks must not be negative")
        if confirmation_blocks and latest_height is None:
            raise ValueError("latest_height is required when confirmation_blocks is nonzero")
        self.db, self.terrad, self.max_orders = db, terrad, max_orders
        self.poll_seconds, self.timeout_seconds = poll_seconds, timeout_seconds
        self.sleep, self.clock = sleep, clock
        self.wall_clock = wall_clock
        self.confirmation_blocks = confirmation_blocks
        self.latest_height = latest_height

    def _record_failure(self, batch_id: int, attempt_id: int, attempt_state: str,
                         error: str, response: dict | None = None, deliver_code: int | None = None,
                         result_code: TxResultCode | None = None) -> str:
        batch = self.db.conn.execute("SELECT failure_count FROM batches WHERE id=?", (batch_id,)).fetchone()
        failures = int(batch["failure_count"]) + 1
        state = "intervention" if failures >= MAX_FAILURES else "ready"
        next_retry = None if state == "intervention" else int(self.wall_clock()) + RETRY_BACKOFF_SECONDS * (2 ** (failures - 1))
        with self.db.transaction(immediate=True) as conn:
            conn.execute(
                "UPDATE tx_attempts SET state=?,deliver_code=?,error=?,response_json=COALESCE(?,response_json),updated_at=? WHERE id=?",
                (attempt_state, deliver_code, error,
                 json.dumps(response, sort_keys=True) if response is not None else None,
                 int(self.wall_clock()), attempt_id),
            )
            conn.execute(
                "UPDATE batches SET state=?,error=?,failure_count=?,next_retry_at=? WHERE id=?",
                (state, f"{(result_code or TxResultCode.DELIVER_FAILED).value}:{error}", failures, next_retry, batch_id),
            )
        return state

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
            events: list[int] = []
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
        if batch["state"] == "intervention":
            return "intervention"
        if batch["state"] == "ready" and batch["next_retry_at"] is not None \
                and int(self.wall_clock()) < int(batch["next_retry_at"]):
            return "backoff"
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
        if not isinstance(tx_response, dict) or "code" not in tx_response:
            error = "broadcast response omitted transaction code"
            with self.db.transaction(immediate=True) as conn:
                conn.execute("UPDATE tx_attempts SET state='unknown',error=?,response_json=?,updated_at=? WHERE id=?",
                             (error, json.dumps(response, sort_keys=True), int(time.time()), attempt_id))
                conn.execute("UPDATE batches SET state='unknown',error=? WHERE id=?", (error, batch_id))
            return "unknown"
        try:
            code = int(tx_response["code"])
        except (TypeError, ValueError):
            code = None
        tx_hash = tx_response.get("txhash") or tx_response.get("hash")
        if code is None or not tx_hash:
            error = "broadcast response contained invalid code or omitted tx hash"
            with self.db.transaction(immediate=True) as conn:
                conn.execute("UPDATE tx_attempts SET state='unknown',error=?,response_json=?,updated_at=? WHERE id=?",
                             (error, json.dumps(response, sort_keys=True), int(time.time()), attempt_id))
                conn.execute("UPDATE batches SET state='unknown',error=? WHERE id=?", (error, batch_id))
            return "unknown"
        with self.db.transaction(immediate=True) as conn:
            conn.execute("UPDATE tx_attempts SET tx_hash=?,check_code=?,response_json=?,updated_at=? WHERE id=?",
                         (tx_hash, code, json.dumps(response, sort_keys=True), int(time.time()), attempt_id))
            conn.execute("UPDATE batches SET tx_hash=? WHERE id=?", (tx_hash, batch_id))
            if code != 0:
                error = tx_response.get("raw_log") or tx_response.get("log") or "CheckTx failed or omitted tx hash"
            else:
                conn.execute("UPDATE tx_attempts SET state='broadcast' WHERE id=?", (attempt_id,))
                conn.execute("UPDATE batches SET state='broadcast' WHERE id=?", (batch_id,))
        if code != 0:
            self._record_failure(batch_id, attempt_id, "check_failed", error,
                                 result_code=TxResultCode.CHECK_FAILED)
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
                try:
                    tx_response = response.get("tx_response", response)
                    observed_hash = tx_response.get("txhash") or tx_response.get("hash")
                    if "code" not in tx_response or "height" not in tx_response or not observed_hash:
                        raise ValueError
                    tx_height = int(tx_response["height"])
                    code = int(tx_response["code"])
                    if tx_height <= 0 or observed_hash.lower() != tx_hash.lower():
                        raise ValueError
                except (AttributeError, TypeError, ValueError):
                    response = None
            if response:
                if self.confirmation_blocks and (
                    not tx_height or self.latest_height() < tx_height + self.confirmation_blocks
                ):
                    if self.clock() - start >= self.timeout_seconds:
                        with self.db.transaction(immediate=True) as conn:
                            conn.execute("UPDATE tx_attempts SET state='timeout',updated_at=? WHERE id=?",
                                         (int(time.time()), attempt_id))
                            conn.execute("UPDATE batches SET state='timeout',error='confirmation polling timed out' WHERE id=?",
                                         (batch_id,))
                        return "timeout"
                    self.sleep(self.poll_seconds)
                    continue
                if code == 0:
                    if _has_reverted_grid_page(response):
                        error = "grid cancel or claim page reverted"
                        self._record_failure(batch_id, attempt_id, "page_reverted", error, response, 0,
                                             TxResultCode.PAGE_REVERTED)
                        return "page_reverted"
                    self._confirm(batch_id, attempt_id, tx_hash, response)
                    return "confirmed"
                error = tx_response.get("raw_log") or tx_response.get("log") or "DeliverTx failed"
                self._record_failure(batch_id, attempt_id, "deliver_failed", error, response, code,
                                     TxResultCode.DELIVER_FAILED)
                return "deliver_failed"
            if self.clock() - start >= self.timeout_seconds:
                with self.db.transaction(immediate=True) as conn:
                    conn.execute("UPDATE tx_attempts SET state='timeout',updated_at=? WHERE id=?", (int(time.time()), attempt_id))
                    conn.execute("UPDATE batches SET state='timeout',error='inclusion polling timed out' WHERE id=?", (batch_id,))
                return "timeout"
            self.sleep(self.poll_seconds)

    def _confirm(self, batch_id: int, attempt_id: int, tx_hash: str, response: dict) -> None:
        tx_response = response.get("tx_response", response)
        try:
            tx_height = int(tx_response["height"])
            observed_hash = tx_response.get("txhash") or tx_response.get("hash")
            code = int(tx_response["code"])
        except (AttributeError, KeyError, TypeError, ValueError) as exc:
            raise ValueError("cannot confirm from incomplete transaction state") from exc
        if tx_height <= 0 or code != 0 or not observed_hash or observed_hash.lower() != tx_hash.lower():
            raise ValueError("cannot confirm from mismatched transaction state")
        with self.db.transaction(immediate=True) as conn:
            batch = conn.execute("SELECT vault_address,through_height FROM batches WHERE id=?", (batch_id,)).fetchone()
            # The on-chain reconcile covers these order identities at execution height,
            # including fills indexed after the immutable batch snapshot was frozen.
            conn.execute(
                "UPDATE raw_events SET reconciled_batch_id=? WHERE reconciled_batch_id IS NULL "
                "AND vault_address=? AND height<=? AND EXISTS (SELECT 1 FROM batch_items i "
                "WHERE i.batch_id=? AND i.pair_address=raw_events.pair_address "
                "AND i.order_id=raw_events.order_id)",
                (batch_id, batch["vault_address"], tx_height, batch_id),
            )
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

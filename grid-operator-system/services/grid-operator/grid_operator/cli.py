import argparse
import json
import signal
import sys
import time
import os
import shutil
from datetime import datetime, timezone
from pathlib import Path

from .config import Config
from .db import Database
from .indexer import Indexer
from .keeper import Keeper
from .rpc import TendermintRPC, Terrad
from .reliability import ProcessLock, StateIdentityError, StateLockError
from .swap_keeper import main as swap_keeper_main


def components(config: Config):
    db = Database(config.db_path)
    db.migrate()
    terrad = Terrad(config.terrad, config.rpc_url, config.chain_id, config.key_name,
                    config.keyring_backend, config.gas_adjustment, config.fees,
                    config.terrad_home, config.signer_command)
    db.validate_identity({
        "schema_version": 1,
        "chain_id": config.chain_id,
        "vaults": sorted(v.lower() for v in config.vaults),
        "signer": f"{config.keyring_backend}:{config.key_name}:{terrad.key_address()}",
        "protocol_kind": "limit-grid",
    })
    rpc = TendermintRPC(config.rpc_url)
    indexer = Indexer(db, rpc, config.chain_id, config.deployment_height,
                      config.vaults, config.finality_depth)
    indexer.register_vaults()
    keeper = Keeper(db, terrad, config.max_orders_per_batch,
                    config.poll_seconds, config.tx_timeout_seconds,
                    confirmation_blocks=config.finality_depth,
                    latest_height=rpc.latest_height)
    return db, terrad, indexer, keeper


def discover(indexer: Indexer, terrad: Terrad, vaults: tuple[str, ...]) -> None:
    for vault in vaults:
        indexer.refresh_orders(terrad, vault)


def status(db: Database, config: Config) -> dict:
    latest = db.conn.execute("SELECT MAX(height) FROM blocks WHERE chain_id=?", (config.chain_id,)).fetchone()[0]
    states = {row["state"]: row["n"] for row in db.conn.execute(
        "SELECT state,COUNT(*) n FROM batches GROUP BY state")}
    return {
        "chain_id": config.chain_id,
        "scanned_height": db.cursor("scanned", config.deployment_height - 1),
        "latest_stored_block": latest,
        "pending_events": db.conn.execute("SELECT COUNT(*) FROM raw_events WHERE reconciled_batch_id IS NULL").fetchone()[0],
        "pending_orders": db.conn.execute("SELECT COUNT(*) FROM aggregates").fetchone()[0],
        "batch_states": states,
        "unknown_broadcasts": db.conn.execute("SELECT COUNT(*) FROM tx_attempts WHERE state='unknown'").fetchone()[0],
        "vaults": [dict(row) for row in db.conn.execute(
            "SELECT address,bot_id,pair_address,last_order_refresh_height FROM vaults WHERE enabled=1 ORDER BY address")],
    }


def diagnose(db: Database, terrad: Terrad, batch_id: int | None = None) -> dict:
    report = db.diagnose()
    rows = report["unresolved"]
    if batch_id is not None:
        rows = [row for row in rows if row["id"] == batch_id]
        if not rows:
            raise ValueError(f"batch {batch_id} is not unresolved")
    chain = []
    for row in rows:
        tx_hash = row["attempt_hash"] or row["tx_hash"]
        tx = None
        if tx_hash:
            try:
                tx = terrad.query_tx(tx_hash)
            except Exception as exc:
                tx = {"status": "unknown", "error_type": type(exc).__name__}
        try:
            vault = {
                "bot": terrad.smart_query(row["vault_address"], {"bot": {"bot_id": 1}}),
                "orders": terrad.smart_query(row["vault_address"], {"orders": {"bot_id": 1}}),
            }
        except Exception as exc:
            vault = {"status": "unknown", "error_type": type(exc).__name__}
        chain.append({"batch_id": row["id"], "transaction": tx, "vault": vault})
    try:
        account = terrad.account_state()
    except Exception as exc:
        account = {"status": "unknown", "error_type": type(exc).__name__}
    report["chain"] = chain
    report["account"] = account
    db.audit("diagnose-chain", f"batch:{batch_id or 'all'}", report)
    return report


def resolve_intervention(db: Database, terrad: Terrad, batch_id: int, reason: str) -> dict:
    row = db.conn.execute(
        "SELECT b.*,t.id attempt_id,t.state attempt_state,t.tx_hash attempt_hash,t.check_code,t.deliver_code "
        "FROM batches b LEFT JOIN tx_attempts t ON t.id=(SELECT MAX(id) FROM tx_attempts WHERE batch_id=b.id) "
        "WHERE b.id=?", (batch_id,),
    ).fetchone()
    if not row or row["state"] != "intervention":
        raise ValueError("resolve only clears a named intervention batch")
    expected = {"check_failed", "deliver_failed", "page_reverted"}
    if reason not in expected or not str(row["error"] or "").startswith(reason + ":"):
        raise ValueError("reason does not match the recorded intervention")
    tx_hash = row["attempt_hash"] or row["tx_hash"]
    if reason in {"deliver_failed", "page_reverted"}:
        if not tx_hash:
            raise ValueError("recorded on-chain failure has no transaction hash")
        tx = terrad.query_tx(tx_hash)
        response = tx.get("tx_response", tx)
        if not isinstance(response, dict) or not response.get("height"):
            raise ValueError("transaction status remains unresolved or ambiguous")
        code = int(response.get("code", -1))
        if reason == "deliver_failed" and code == 0:
            raise ValueError("transaction status contradicts the recorded failure")
        if reason == "page_reverted" and code != 0:
            raise ValueError("transaction status contradicts the recorded page result")
    # Both queries must succeed before reason-specific suppression is cleared.
    terrad.smart_query(row["vault_address"], {"bot": {"bot_id": 1}})
    terrad.smart_query(row["vault_address"], {"orders": {"bot_id": 1}})
    account = terrad.account_state()
    stamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    backup = str(Path(db.path).with_name(Path(db.path).name + f".{stamp}.bak"))
    db.backup(backup)
    with db.transaction(immediate=True) as conn:
        conn.execute("UPDATE batches SET state='ready',error=NULL,failure_count=0,next_retry_at=NULL WHERE id=?",
                     (batch_id,))
        db.audit("clear-intervention", f"batch:{batch_id}",
                 {"reason": reason, "backup": backup, "account": account}, conn)
    return {"batch_id": batch_id, "state": "ready", "backup": backup, "reason": reason}


def restore_database(config: Config, terrad: Terrad, source_path: str) -> dict:
    source = Database(source_path)
    try:
        integrity = [row[0] for row in source.conn.execute("PRAGMA integrity_check(100)")]
        if integrity != ["ok"] or source.conn.execute("PRAGMA foreign_key_check").fetchone():
            raise ValueError("backup failed SQLite integrity diagnostics")
        identity = {
            "schema_version": 1,
            "chain_id": config.chain_id,
            "vaults": sorted(v.lower() for v in config.vaults),
            "signer": f"{config.keyring_backend}:{config.key_name}:{terrad.key_address()}",
            "protocol_kind": "limit-grid",
        }
        source.validate_identity(identity)
        account = terrad.account_state()
        for vault in config.vaults:
            terrad.smart_query(vault, {"bot": {"bot_id": 1}})
            terrad.smart_query(vault, {"orders": {"bot_id": 1}})
        unresolved_unknown = source.conn.execute(
            "SELECT 1 FROM batches WHERE state IN ('unknown','broadcasting') LIMIT 1"
        ).fetchone()
        if unresolved_unknown:
            raise ValueError("backup contains an unknown broadcast and cannot be restored")
        stamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
        preserved = None
        if os.path.exists(config.db_path):
            preserved = str(config.db_path) + f".quarantine.{stamp}"
            try:
                current = Database(config.db_path)
                current.backup(preserved)
                current.conn.close()
            except Exception:
                shutil.copy2(config.db_path, preserved)
        temporary = str(config.db_path) + ".restore.tmp"
        target = Database(temporary)
        source.conn.backup(target.conn)
        target.conn.close()
        os.replace(temporary, config.db_path)
    finally:
        source.conn.close()
    restored = Database(config.db_path)
    restored.audit("restore-database", "database",
                   {"source": source_path, "preserved": preserved, "account": account})
    restored.conn.close()
    return {"source": source_path, "preserved": preserved, "integrity": "ok"}


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(prog="grid-operator")
    parser.add_argument("command", choices=(
        "run", "index", "keep", "keep-swap", "keep-discover", "migrate", "status",
        "diagnose", "resolve", "clear-intervention", "restore"))
    parser.add_argument("--scan-limit", type=int)
    parser.add_argument("--batch-id", type=int)
    parser.add_argument("--reason", choices=("check_failed", "deliver_failed", "page_reverted"))
    parser.add_argument("--backup")
    args, unknown = parser.parse_known_args(argv)
    if args.command == "keep-swap":
        return swap_keeper_main(unknown)
    if args.command == "keep-discover":
        from .discovery_keeper import main as discovery_keeper_main
        return discovery_keeper_main(unknown)
    config = Config.from_env()
    if args.command in ("index", "keep", "run") and not config.vaults:
        print("GRID_VAULTS is empty: set at least one limit-grid vault address", file=sys.stderr)
        return 2
    if args.command == "restore":
        if not args.backup:
            print("restore requires --backup", file=sys.stderr)
            return 2
        try:
            with ProcessLock(str(config.db_path)):
                terrad = Terrad(config.terrad, config.rpc_url, config.chain_id, config.key_name,
                                config.keyring_backend, config.gas_adjustment, config.fees,
                                config.terrad_home, config.signer_command)
                print(json.dumps(restore_database(config, terrad, args.backup), sort_keys=True))
            return 0
        except Exception as error:
            print(f"restore refused: {error}", file=sys.stderr)
            return 2
    try:
        process_lock = ProcessLock(str(config.db_path))
        db, terrad, indexer, keeper = components(config)
    except (StateLockError, StateIdentityError) as error:
        print(f"operator startup refused: {error}", file=sys.stderr)
        return 2
    if args.command == "migrate":
        process_lock.close()
        return 0
    if args.command == "status":
        print(json.dumps(status(db, config), indent=2, sort_keys=True))
        return 0
    if args.command == "diagnose":
        try:
            print(json.dumps(diagnose(db, terrad, args.batch_id), indent=2, sort_keys=True))
            return 0
        except ValueError as error:
            print(f"diagnosis refused: {error}", file=sys.stderr)
            return 2
    if args.command in ("resolve", "clear-intervention"):
        if args.batch_id is None or args.reason is None:
            print("resolve requires --batch-id and --reason", file=sys.stderr)
            return 2
        try:
            print(json.dumps(resolve_intervention(db, terrad, args.batch_id, args.reason), sort_keys=True))
            return 0
        except Exception as error:
            print(f"resolve refused: {error}", file=sys.stderr)
            return 2
    if args.command == "index":
        discover(indexer, terrad, config.vaults)
        print(json.dumps({"blocks_scanned": indexer.scan(args.scan_limit)}))
        return 0
    if args.command == "keep":
        print(json.dumps(keeper.keep_once(
            config.vaults, lambda vault: indexer.refresh_orders(terrad, vault, db.cursor("scanned"))), sort_keys=True))
        return 0
    stopping = False

    def stop(_signum, _frame):
        nonlocal stopping
        stopping = True

    signal.signal(signal.SIGTERM, stop)
    signal.signal(signal.SIGINT, stop)
    discover(indexer, terrad, config.vaults)
    while not stopping:
        indexer.scan(args.scan_limit)
        keeper.keep_once(config.vaults,
                         lambda vault: indexer.refresh_orders(terrad, vault, db.cursor("scanned")))
        if not stopping:
            time.sleep(config.loop_seconds)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

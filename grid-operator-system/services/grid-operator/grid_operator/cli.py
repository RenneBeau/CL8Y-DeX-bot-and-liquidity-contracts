import argparse
import json
import signal
import sys
import time

from .config import Config
from .db import Database
from .indexer import Indexer
from .keeper import Keeper
from .rpc import TendermintRPC, Terrad
from .swap_keeper import main as swap_keeper_main


def components(config: Config):
    db = Database(config.db_path)
    db.migrate()
    rpc = TendermintRPC(config.rpc_url)
    terrad = Terrad(config.terrad, config.rpc_url, config.chain_id, config.key_name,
                    config.keyring_backend, config.gas_adjustment, config.fees)
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


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(prog="grid-operator")
    parser.add_argument("command", choices=(
        "run", "index", "keep", "keep-swap", "keep-discover", "migrate", "status"))
    parser.add_argument("--scan-limit", type=int)
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
    db, terrad, indexer, keeper = components(config)
    if args.command == "migrate":
        return 0
    if args.command == "status":
        print(json.dumps(status(db, config), indent=2, sort_keys=True))
        return 0
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

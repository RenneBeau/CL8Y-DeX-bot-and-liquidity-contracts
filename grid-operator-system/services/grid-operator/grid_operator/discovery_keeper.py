"""Discovery keeper for CL8Y grid and rebalance vaults.

Scans the chain for vault instantiations and keeps every vault it finds in a
single process:

- ``grid`` (``grid-vault-swap``): matched by ``GRID_SWAP_CODE_ID``, kept via
  ``{"grid_status": {}}`` + permissionless ``{"rebalance": ...}``.
- ``rebalance`` (``bot-vault``): matched by ``GRID_REBALANCE_CODE_ID``, kept
  via ``{"rebalance_plan": {}}`` + ``{"rebalance": ...}`` or
  ``{"sync_reference": {}}``. The keeper key must be the address each vault
  authorizes in its ``config.keeper``.

Signing is serial across all vaults: the single keeper key never signs
concurrently. Each vault owns a fail-closed JSON tracker under
``GRID_SWAP_STATE_DIR``, so an unresolved broadcast is never automatically
rebroadcast. Dry-run is the default; pass ``--broadcast`` to sign and submit.
"""

import argparse
import os
import time
from pathlib import Path

from .db import Database
from .indexer import Indexer
from .protocol import PROTOCOLS
from .rpc import TendermintRPC, Terrad
from .swap_keeper import SwapTxTracker, keep_vault


class DiscoveryKeeper:
    def __init__(self, db: Database, terrad: Terrad, args):
        self.db = db
        self.terrad = terrad
        self.args = args
        self.state_dir = Path(args.state_dir)

    def tracker_for(self, vault: str) -> SwapTxTracker:
        safe = vault.strip().lower()
        return SwapTxTracker(str(self.state_dir / f"{safe}.json"))

    def discovered_vaults(self) -> list[tuple[str, str]]:
        rows = self.db.conn.execute(
            "SELECT address,kind FROM discovered_vaults WHERE enabled=1 "
            "ORDER BY kind,address"
        ).fetchall()
        return [(row["address"], row["kind"]) for row in rows]

    def run_once(self) -> None:
        vaults = self.discovered_vaults()
        if not vaults:
            print("no discovered vaults yet")
            return
        print(f"keeping {len(vaults)} discovered vaults: "
              f"{', '.join(f'{kind}:{vault}' for vault, kind in vaults)}")
        for vault, kind in vaults:
            protocol = PROTOCOLS.get(kind)
            if protocol is None:
                print(f"unknown vault kind {kind} for {vault}; skipping")
                continue
            tracker = self.tracker_for(vault)
            try:
                keep_vault(self.args, self.terrad, tracker, vault, protocol)
            except Exception as error:
                print(f"discovery keeper error for {vault}: {error}")
                if getattr(self.args, "once", False):
                    raise


def parse_args(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--db", default=os.getenv("GRID_DB_PATH", "./grid-operator.sqlite3"))
    parser.add_argument("--rpc", default=os.getenv("GRID_RPC_URL"))
    parser.add_argument("--chain-id", default=os.getenv("GRID_CHAIN_ID"))
    parser.add_argument("--deployment-height", type=int,
                        default=int(os.getenv("GRID_DEPLOYMENT_HEIGHT", "1")))
    parser.add_argument("--finality-depth", type=int,
                        default=int(os.getenv("GRID_FINALITY_DEPTH", "10")))
    parser.add_argument("--code-id", default=os.getenv("GRID_SWAP_CODE_ID"))
    parser.add_argument("--rebalance-code-id", default=os.getenv("GRID_REBALANCE_CODE_ID"))
    parser.add_argument("--key", default=os.getenv("GRID_KEY_NAME", "grid-keeper"))
    parser.add_argument("--keyring-backend", default=os.getenv("GRID_KEYRING_BACKEND", "os"))
    parser.add_argument("--terrad", default=os.getenv("GRID_TERRAD", "terrad"))
    parser.add_argument("--fees", default=os.getenv("GRID_FEES", ""))
    parser.add_argument("--gas-adjustment", default=os.getenv("GRID_GAS_ADJUSTMENT", "1.4"))
    parser.add_argument("--config-version", default=os.getenv("GRID_SWAP_CONFIG_VERSION", "1"))
    parser.add_argument("--deadline-seconds", type=int,
                        default=int(os.getenv("GRID_SWAP_DEADLINE_SECONDS", "120")))
    parser.add_argument("--poll-seconds", type=int,
                        default=int(os.getenv("GRID_SWAP_POLL_SECONDS", "15")))
    parser.add_argument("--tx-poll-seconds", type=float,
                        default=float(os.getenv("GRID_SWAP_TX_POLL_SECONDS", "2")))
    parser.add_argument("--tx-timeout-seconds", type=float,
                        default=float(os.getenv("GRID_SWAP_TX_TIMEOUT_SECONDS", "60")))
    parser.add_argument("--confirmation-blocks", type=int,
                        default=int(os.getenv("GRID_SWAP_CONFIRMATION_BLOCKS", "2")))
    parser.add_argument("--state-dir",
                        default=os.getenv("GRID_SWAP_STATE_DIR", ".grid-swap-discovery-state"))
    parser.add_argument("--once", action="store_true")
    parser.add_argument("--broadcast", action="store_true")
    args = parser.parse_args(argv)
    if not args.rpc:
        parser.error("--rpc or GRID_RPC_URL is required")
    if not args.chain_id:
        parser.error("--chain-id or GRID_CHAIN_ID is required")
    if not args.code_id and not args.rebalance_code_id:
        parser.error("--code-id or --rebalance-code-id is required")
    if args.deadline_seconds <= 0 or args.tx_poll_seconds <= 0 or args.tx_timeout_seconds <= 0 \
            or args.confirmation_blocks < 0:
        parser.error("deadline and transaction polling values must be positive")
    return args


def main(argv=None) -> int:
    args = parse_args(argv)
    code_ids = {}
    if args.code_id:
        code_ids["grid"] = args.code_id
    if args.rebalance_code_id:
        code_ids["rebalance"] = args.rebalance_code_id
    db = Database(args.db)
    db.migrate()
    rpc = TendermintRPC(args.rpc)
    terrad = Terrad(args.terrad, args.rpc, args.chain_id, args.key, args.keyring_backend,
                    args.gas_adjustment, args.fees)
    indexer = Indexer(db, rpc, args.chain_id, args.deployment_height, (),
                      args.finality_depth, code_ids=code_ids)
    keeper = DiscoveryKeeper(db, terrad, args)
    while True:
        try:
            indexer.scan()
            keeper.run_once()
        except Exception as error:
            print(f"discovery keeper error: {error}")
            if args.once:
                return 1
        if args.once:
            return 0
        time.sleep(args.poll_seconds)

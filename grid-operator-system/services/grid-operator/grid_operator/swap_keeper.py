"""Permissionless keeper for the CL8Y grid-vault-swap.

The swap-only vault has no limit orders: it holds CW20 balances and, when the
TWAP crosses a grid level, anyone may call ``{"rebalance": {"deadline": n}}``
to pay the pair Swap taker and re-balance toward the target cell. This keeper
polls ``{"grid_status": {}}`` on the configured swap vault (one vault per
process) and, when the vault reports ``should_rebalance`` with no swap already
pending, submits the rebalance transaction.

The keeper is fully permissionless and self-funds its own gas. State is kept in
a fail-closed JSON tracker: an unresolved broadcast is never automatically
rebroadcast (operator intervention required), matching the durability
guarantees of the limit-order grid-operator keeper.

Dry-run is the default. Pass --broadcast to sign and submit.
"""

import argparse
import json
import math
import os
import time

from .protocol import fingerprint_v1 as plan_fingerprint
from .protocol import grid_protocol
from .rpc import RpcError, Terrad
from .reliability import ProcessLock, StateIdentityError

__all__ = ["plan_fingerprint"]


class DeterministicTxError(RuntimeError):
    """A CheckTx or DeliverTx rejection that retrying unchanged cannot fix."""


TRANSIENT_MARKERS = (
    "account sequence mismatch",
    "connection refused",
    "connection reset",
    "context deadline exceeded",
    "mempool full",
    "temporarily unavailable",
    "timed out",
    "timeout",
)


def is_transient_error(detail: str) -> bool:
    detail = detail.lower()
    return any(marker in detail for marker in TRANSIENT_MARKERS)


class SwapTxTracker:
    """Persists the single in-flight rebalance transaction per vault."""

    SCHEMA_VERSION = 1

    def __init__(self, path: str, identity: dict | None = None):
        self.path = path
        self.identity = identity
        self.pending_hash = None
        self.pending_vault = None
        self.pending_plan = None
        self.pending_since = None
        self.suppressed_plan = None
        self.broadcasting = False
        if path and os.path.exists(path):
            try:
                with open(path, encoding="utf-8") as state_file:
                    state = json.load(state_file)
            except (OSError, json.JSONDecodeError) as exc:
                raise StateIdentityError(
                    f"cannot safely read state {path}; preserve it and recover manually: {exc}"
                ) from exc
            if not isinstance(state, dict):
                raise StateIdentityError(f"state {path} is not a JSON object; preserve it for recovery")
            if identity is not None:
                stored = state.get("identity")
                if state.get("schema_version") != self.SCHEMA_VERSION or stored is None:
                    raise StateIdentityError(
                        f"legacy state {path} has no trusted identity; explicit migration is required"
                    )
                if stored != identity:
                    raise StateIdentityError(
                        f"state identity mismatch for {path}: stored={stored!r}, configured={identity!r}"
                    )
            self.pending_hash = state.get("pending_hash")
            self.pending_vault = state.get("pending_vault")
            self.pending_plan = state.get("pending_plan")
            self.pending_since = state.get("pending_since")
            self.suppressed_plan = state.get("suppressed_plan")
            self.broadcasting = bool(state.get("broadcasting", False))

    def save(self):
        if not self.path:
            return
        directory = os.path.dirname(os.path.abspath(self.path))
        os.makedirs(directory, mode=0o700, exist_ok=True)
        temporary = self.path + ".tmp"
        with open(temporary, "w", encoding="utf-8") as state_file:
            json.dump(
                {
                    "schema_version": self.SCHEMA_VERSION,
                    "identity": self.identity,
                    "pending_hash": self.pending_hash,
                    "pending_vault": self.pending_vault,
                    "pending_plan": self.pending_plan,
                    "pending_since": self.pending_since,
                    "suppressed_plan": self.suppressed_plan,
                    "broadcasting": self.broadcasting,
                },
                state_file,
            )
            state_file.flush()
            os.fsync(state_file.fileno())
        os.chmod(temporary, 0o600)
        os.replace(temporary, self.path)
        directory_fd = os.open(directory, os.O_RDONLY)
        try:
            os.fsync(directory_fd)
        finally:
            os.close(directory_fd)


def build_rebalance(plan: dict, deadline: int):
    """Translate a GridStatus response into an execute message, or None."""
    if not plan.get("should_rebalance"):
        return None
    if plan.get("pending_swap"):
        # Another relayer already submitted a swap; do not pile on.
        return None
    return {"rebalance": {"deadline": deadline}}


def broadcast(vault: str, message: dict, terrad: Terrad):
    signed = terrad.sign_execute(vault, message)
    response = terrad.broadcast(signed)
    tx_response = response.get("tx_response", response)
    if not isinstance(tx_response, dict) or "code" not in tx_response:
        raise RpcError("sync broadcast response omitted transaction code")
    code = int(tx_response["code"])
    if code != 0:
        detail = tx_response.get("raw_log") or tx_response.get("log") or "CheckTx failed"
        if is_transient_error(detail):
            raise RpcError(f"CheckTx code {code}: {detail}")
        raise DeterministicTxError(f"CheckTx code {code}: {detail}")
    tx_hash = tx_response.get("txhash") or tx_response.get("hash")
    if not tx_hash:
        raise RpcError("sync broadcast returned no transaction hash")
    return tx_hash, response


def poll_pending(args, tracker, terrad, sleep=time.sleep, clock=time.monotonic):
    deadline = clock() + args.tx_timeout_seconds
    while True:
        try:
            result = terrad.query_tx(tracker.pending_hash)
        except RpcError:
            result = None
        if result:
            try:
                tx_response = result.get("tx_response", result)
                observed_hash = tx_response.get("txhash") or tx_response.get("hash")
                if "code" not in tx_response or "height" not in tx_response or not observed_hash:
                    raise ValueError("incomplete transaction response")
                tx_height = int(tx_response["height"])
                code = int(tx_response["code"])
                if tx_height <= 0 or observed_hash.lower() != tracker.pending_hash.lower():
                    raise ValueError("invalid height or mismatched transaction hash")
            except (AttributeError, TypeError, ValueError):
                result = None
        if result:
            if args.confirmation_blocks and tx_height:
                latest = int(terrad.latest_height())
                if latest < tx_height + args.confirmation_blocks:
                    if clock() >= deadline:
                        return False
                    sleep(args.tx_poll_seconds)
                    continue
            tx_hash = tracker.pending_hash
            plan = tracker.pending_plan
            tracker.pending_hash = None
            tracker.pending_vault = None
            tracker.pending_plan = None
            tracker.pending_since = None
            tracker.broadcasting = False
            if code != 0:
                tracker.suppressed_plan = plan
                tracker.save()
                raise DeterministicTxError(
                    f"DeliverTx {tx_hash} code {code}: "
                    f"{tx_response.get('raw_log') or tx_response.get('log')}"
                )
            tracker.suppressed_plan = None
            tracker.save()
            return True
        if clock() >= deadline:
            print(f"transaction {tracker.pending_hash} is still unresolved; retaining it")
            return False
        sleep(args.tx_poll_seconds)


def keep_vault(args, terrad, tracker, vault, protocol=None):
    """Run one keeper pass for a single vault under a protocol.

    ``protocol`` defaults to the grid-swap protocol so ``keep-swap`` keeps its
    exact current behavior.
    """
    if protocol is None:
        protocol = grid_protocol
    if tracker.broadcasting and not tracker.pending_hash:
        print("previous broadcast outcome is unknown; operator intervention required")
        return
    if tracker.pending_hash:
        poll_pending(args, tracker, terrad)
        return

    try:
        plan = protocol.plan(terrad, vault)
    except RpcError as exc:
        print(f"{protocol.query_label} query failed: {exc}")
        return

    message = protocol.build_message(plan, int(time.time()) + args.deadline_seconds)
    if message is None:
        tracker.suppressed_plan = None
        tracker.save()
        print(protocol.noop_message(plan))
        return

    fingerprint = protocol.fingerprint(plan, message, vault, args)
    if tracker.suppressed_plan == fingerprint:
        print("rebalance suppressed after deterministic failure; plan is unchanged")
        return

    print(json.dumps(message, indent=2))
    if not args.broadcast:
        print("dry-run only; pass --broadcast to preflight, sign, and submit")
        return

    try:
        terrad.preflight(vault, message)
        tracker.broadcasting = True
        tracker.pending_vault = vault
        tracker.pending_plan = fingerprint
        tracker.pending_since = time.time()
        tracker.save()
        tx_hash, _response = broadcast(vault, message, terrad)
    except DeterministicTxError:
        tracker.broadcasting = False
        tracker.pending_vault = None
        tracker.pending_plan = None
        tracker.pending_since = None
        tracker.suppressed_plan = fingerprint
        tracker.save()
        raise
    except RpcError:
        # The durable marker was written before send. The node may have accepted
        # the transaction even when the client saw an error or invalid reply.
        raise
    tracker.pending_hash = tx_hash
    tracker.broadcasting = False
    tracker.save()
    print(f"broadcast tx: {tx_hash}")
    poll_pending(args, tracker, terrad)


def run_once(args, terrad, tracker):
    keep_vault(args, terrad, tracker, args.vault)


def parse_args(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--vault", default=os.getenv("GRID_SWAP_VAULTS"))
    parser.add_argument("--rpc", default=os.getenv("GRID_SWAP_RPC_URL", "http://127.0.0.1:26657"))
    parser.add_argument("--chain-id", default=os.getenv("GRID_SWAP_CHAIN_ID", "localterra"))
    parser.add_argument("--key", default=os.getenv("GRID_SWAP_KEY_NAME", "grid-keeper"))
    parser.add_argument("--keyring-backend", default=os.getenv("GRID_SWAP_KEYRING_BACKEND", "os"))
    parser.add_argument("--terrad", default=os.getenv("GRID_SWAP_TERRAD", "terrad"))
    parser.add_argument("--fees", default=os.getenv("GRID_SWAP_FEES", ""))
    parser.add_argument("--gas-adjustment", default=os.getenv("GRID_SWAP_GAS_ADJUSTMENT", "1.4"))
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
    parser.add_argument("--state-file", default=os.getenv("GRID_SWAP_STATE_FILE",
                                                          ".grid-swap-keeper-state.json"))
    parser.add_argument("--once", action="store_true")
    parser.add_argument("--broadcast", action="store_true")
    args = parser.parse_args(argv)
    if not args.vault:
        parser.error("--vault or GRID_SWAP_VAULTS is required")
    if args.deadline_seconds <= 0 or not math.isfinite(args.tx_poll_seconds) \
            or not math.isfinite(args.tx_timeout_seconds) or args.tx_poll_seconds <= 0 \
            or args.tx_timeout_seconds <= 0 \
            or args.poll_seconds <= 0 or args.confirmation_blocks < 0:
        parser.error("deadline and transaction polling values must be positive")
    return args


def main(argv=None) -> int:
    args = parse_args(argv)
    try:
        with ProcessLock(args.state_file):
            terrad = Terrad(args.terrad, args.rpc, args.chain_id, args.key, args.keyring_backend,
                            args.gas_adjustment, args.fees)
            identity = {
                "chain_id": args.chain_id,
                "vault": args.vault.lower(),
                "signer": f"{args.keyring_backend}:{args.key}:{terrad.key_address()}",
                "protocol_kind": "grid-swap",
            }
            tracker = SwapTxTracker(args.state_file, identity)
            if not os.path.exists(args.state_file):
                tracker.save()
            while True:
                try:
                    run_once(args, terrad, tracker)
                except Exception as error:
                    print(f"keeper error: {error}")
                    if args.once:
                        return 1
                if args.once:
                    return 0
                time.sleep(args.poll_seconds)
    except (StateIdentityError, RuntimeError) as error:
        print(f"keeper startup refused: {error}")
        return 2

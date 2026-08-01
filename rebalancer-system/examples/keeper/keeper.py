#!/usr/bin/env python3
"""CL8Y bot-vault keeper with final transaction tracking.

Dry-run is the default. Pass --broadcast to sign with a terrad keyring entry.
"""

import argparse
import base64
import json
import os
import subprocess
import time
import urllib.error
import urllib.parse
import urllib.request


class DeterministicTxError(RuntimeError):
    """A CheckTx or DeliverTx rejection that retrying unchanged cannot fix."""


def is_transient_error(detail):
    detail = detail.lower()
    return any(marker in detail for marker in (
        "account sequence mismatch",
        "connection refused",
        "connection reset",
        "context deadline exceeded",
        "mempool full",
        "temporarily unavailable",
        "timed out",
        "timeout",
    ))


class TxTracker:
    def __init__(self, path=None):
        self.path = path
        self.pending_hash = None
        self.pending_plan = None
        self.pending_since = None
        self.suppressed_plan = None
        self.broadcasting = False
        if path and os.path.exists(path):
            with open(path, encoding="utf-8") as state_file:
                state = json.load(state_file)
            self.pending_hash = state.get("pending_hash")
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
                    "pending_hash": self.pending_hash,
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


def get_json(url):
    with urllib.request.urlopen(url, timeout=15) as response:
        return json.load(response)


def smart_query(lcd, contract, message):
    encoded = base64.b64encode(
        json.dumps(message, separators=(",", ":")).encode()
    ).decode()
    url = (
        f"{lcd.rstrip('/')}/cosmwasm/wasm/v1/contract/"
        f"{urllib.parse.quote(contract, safe='')}/smart/{encoded}"
    )
    return get_json(url)["data"]


def plan_fingerprint(plan, message):
    return json.dumps(
        {
            "captured_twap": plan["captured_twap"],
            "balances": plan["balances"],
            "reference_price": plan["reference_price"],
            "action": next(iter(message)),
        },
        sort_keys=True,
        separators=(",", ":"),
    )


def build_rebalance(plan, deadline):
    if not plan["should_rebalance"]:
        return None
    if plan.get("offer_token") is None:
        return {"sync_reference": {}}
    return {"rebalance": {"deadline": deadline}}


def tx_command(vault, message, args):
    return [
        args.terrad,
        "tx",
        "wasm",
        "execute",
        vault,
        json.dumps(message, separators=(",", ":")),
        "--from",
        args.key,
        "--keyring-backend",
        args.keyring_backend,
        "--chain-id",
        args.chain_id,
        "--node",
        args.rpc,
        "--gas",
        "auto",
        "--gas-adjustment",
        args.gas_adjustment,
        "--gas-prices",
        args.gas_prices,
        "--yes",
        "--output",
        "json",
    ]


def run_command(command):
    result = subprocess.run(command, text=True, capture_output=True, check=False, timeout=60)
    if result.returncode:
        detail = result.stderr.strip() or result.stdout.strip()
        if is_transient_error(detail):
            raise RuntimeError(detail or "temporary terrad failure")
        raise DeterministicTxError(detail or "terrad command failed")
    return result


def preflight(vault, message, args):
    run_command(tx_command(vault, message, args) + ["--dry-run"])


def broadcast(vault, message, args):
    result = run_command(
        tx_command(vault, message, args) + ["--broadcast-mode", "sync"]
    )
    try:
        tx = json.loads(result.stdout)
        code = int(tx.get("code", tx.get("tx_response", {}).get("code", 0)))
    except (ValueError, TypeError, json.JSONDecodeError) as error:
        raise RuntimeError("invalid sync broadcast response") from error
    if code != 0:
        detail = tx.get("raw_log") or tx.get("tx_response", {}).get("raw_log", "")
        if is_transient_error(detail):
            raise RuntimeError(f"CheckTx code {code}: {detail}")
        raise DeterministicTxError(f"CheckTx code {code}: {detail}")
    tx_hash = tx.get("txhash") or tx.get("tx_response", {}).get("txhash")
    if not tx_hash:
        raise RuntimeError("sync broadcast returned no transaction hash")
    return tx_hash


def query_final_tx(lcd, rpc, tx_hash):
    lcd_url = (
        f"{lcd.rstrip('/')}/cosmos/tx/v1beta1/txs/"
        f"{urllib.parse.quote(tx_hash, safe='')}"
    )
    try:
        response = get_json(lcd_url).get("tx_response", {})
        if response:
            return {
                "code": int(response.get("code", 0)),
                "raw_log": response.get("raw_log", ""),
                "height": response.get("height"),
            }
    except urllib.error.HTTPError as error:
        if error.code != 404:
            raise

    rpc_url = f"{rpc.rstrip('/')}/tx?{urllib.parse.urlencode({'hash': '0x' + tx_hash})}"
    try:
        response = get_json(rpc_url).get("result")
    except urllib.error.HTTPError as error:
        if error.code == 404:
            return None
        raise
    if not response:
        return None
    result = response.get("tx_result", {})
    return {
        "code": int(result.get("code", 0)),
        "raw_log": result.get("log", ""),
        "height": response.get("height"),
    }


def poll_pending(args, tracker, query_tx=query_final_tx, sleep=time.sleep):
    deadline = time.monotonic() + args.tx_timeout_seconds
    while True:
        result = query_tx(args.lcd, args.rpc, tracker.pending_hash)
        if result is not None:
            tx_hash = tracker.pending_hash
            plan = tracker.pending_plan
            tracker.pending_hash = None
            tracker.pending_plan = None
            tracker.pending_since = None
            tracker.broadcasting = False
            if result["code"] != 0:
                tracker.suppressed_plan = plan
                tracker.save()
                raise DeterministicTxError(
                    f"DeliverTx {tx_hash} code {result['code']}: {result['raw_log']}"
                )
            tracker.suppressed_plan = None
            tracker.save()
            print(f"confirmed tx {tx_hash} at height {result['height']}")
            return True
        if time.monotonic() >= deadline:
            print(
                f"transaction {tracker.pending_hash} is still unresolved; "
                "retaining it without rebroadcast"
            )
            return False
        sleep(args.tx_poll_seconds)


def run_once(args, tracker):
    if getattr(tracker, "broadcasting", False) is True and not tracker.pending_hash:
        print("previous broadcast outcome is unknown; operator intervention required")
        return
    if tracker.pending_hash:
        poll_pending(args, tracker)
        return

    plan = smart_query(args.lcd, args.vault, {"rebalance_plan": {}})
    if not plan["should_rebalance"]:
        tracker.suppressed_plan = None
        tracker.save()
        print(f"no rebalance: price deviation {plan['price_deviation_bps']} bps")
        return

    deadline = int(time.time()) + args.deadline_seconds
    message = build_rebalance(plan, deadline)
    fingerprint = plan_fingerprint(plan, message)
    if tracker.suppressed_plan == fingerprint:
        print("rebalance suppressed after deterministic failure; plan is unchanged")
        return

    print(json.dumps(message, indent=2))
    if not args.broadcast:
        print("dry-run only; pass --broadcast to preflight, sign, and submit")
        return

    try:
        preflight(args.vault, message, args)
        tracker.broadcasting = True
        tracker.pending_plan = fingerprint
        tracker.pending_since = time.time()
        tracker.save()
        tx_hash = broadcast(args.vault, message, args)
    except DeterministicTxError:
        tracker.broadcasting = False
        tracker.pending_plan = None
        tracker.pending_since = None
        tracker.suppressed_plan = fingerprint
        tracker.save()
        raise
    tracker.pending_hash = tx_hash
    tracker.broadcasting = False
    tracker.save()
    print(f"broadcast tx: {tx_hash}")
    poll_pending(args, tracker)


def parse_args():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--vault", default=os.getenv("KEEPER_VAULT_ADDRESS"))
    parser.add_argument("--lcd", default=os.getenv("KEEPER_LCD_URL", "http://127.0.0.1:1317"))
    parser.add_argument("--rpc", default=os.getenv("KEEPER_RPC_URL", "http://127.0.0.1:26657"))
    parser.add_argument("--chain-id", default=os.getenv("KEEPER_CHAIN_ID", "localterra"))
    parser.add_argument("--key", default=os.getenv("KEEPER_KEY_NAME", "test1"))
    parser.add_argument("--keyring-backend", default=os.getenv("KEEPER_KEYRING_BACKEND", "test"))
    parser.add_argument("--terrad", default=os.getenv("KEEPER_TERRAD", "terrad"))
    parser.add_argument("--gas-prices", default=os.getenv("KEEPER_GAS_PRICES", "28.325uluna"))
    parser.add_argument("--gas-adjustment", default=os.getenv("KEEPER_GAS_ADJUSTMENT", "1.4"))
    parser.add_argument("--deadline-seconds", type=int, default=int(os.getenv("KEEPER_DEADLINE_SECONDS", "120")))
    parser.add_argument("--poll-seconds", type=int, default=int(os.getenv("KEEPER_POLL_SECONDS", "15")))
    parser.add_argument("--tx-poll-seconds", type=float, default=float(os.getenv("KEEPER_TX_POLL_SECONDS", "2")))
    parser.add_argument("--tx-timeout-seconds", type=float, default=float(os.getenv("KEEPER_TX_TIMEOUT_SECONDS", "60")))
    parser.add_argument("--state-file", default=os.getenv("KEEPER_STATE_FILE", ".keeper-state.json"))
    parser.add_argument("--once", action="store_true")
    parser.add_argument("--broadcast", action="store_true")
    args = parser.parse_args()
    if not args.vault:
        parser.error("--vault or KEEPER_VAULT_ADDRESS is required")
    if args.deadline_seconds <= 0 or args.tx_poll_seconds <= 0 or args.tx_timeout_seconds <= 0:
        parser.error("deadline and transaction polling values must be positive")
    return args


def main():
    args = parse_args()
    tracker = TxTracker(args.state_file)
    while True:
        try:
            run_once(args, tracker)
        except Exception as error:  # Keep a long-running keeper alive on endpoint errors.
            print(f"keeper error: {error}")
            if args.once:
                raise
        if args.once:
            return
        time.sleep(args.poll_seconds)


if __name__ == "__main__":
    main()

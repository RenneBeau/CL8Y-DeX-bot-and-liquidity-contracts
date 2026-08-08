#!/usr/bin/env python3
"""CL8Y bot-vault keeper with final transaction tracking.

Dry-run is the default. Pass --broadcast to sign with a terrad keyring entry.
"""

import argparse
import base64
import decimal
import fcntl
import hashlib
import json
import math
import os
import subprocess
import time
import urllib.error
import urllib.parse
import urllib.request
from enum import Enum
import shutil


class ErrorCode(str, Enum):
    COMMAND_REJECTED = "command_rejected"
    CHECK_REJECTED = "check_rejected"
    DELIVER_REJECTED = "deliver_rejected"
    TRANSPORT = "transport"
    AMBIGUOUS_BROADCAST = "ambiguous_broadcast"


class KeeperError(RuntimeError):
    def __init__(self, code, detail):
        super().__init__(detail)
        self.code = ErrorCode(code)


class DeterministicTxError(KeeperError):
    """A CheckTx or DeliverTx rejection that retrying unchanged cannot fix."""

    def __init__(self, detail, code=ErrorCode.COMMAND_REJECTED):
        super().__init__(code, detail)


class StateIdentityError(RuntimeError):
    pass


class StateLockError(RuntimeError):
    pass


class ProcessLock:
    def __init__(self, state_path):
        self.path = os.path.abspath(state_path) + ".lock"
        os.makedirs(os.path.dirname(self.path), mode=0o700, exist_ok=True)
        self.fd: int | None = os.open(self.path, os.O_RDWR | os.O_CREAT, 0o600)
        try:
            fcntl.flock(self.fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError as error:
            os.close(self.fd)
            self.fd = None
            raise StateLockError(
                f"state is already locked by another process: {state_path}"
            ) from error

    def close(self):
        if self.fd is not None:
            os.close(self.fd)
            self.fd = None

    def __enter__(self):
        return self

    def __exit__(self, *_args):
        self.close()


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
    SCHEMA_VERSION = 2
    MAX_BACKUPS = 3

    def __init__(self, path=None, identity=None):
        self.path = path
        self.identity = identity
        self.pending_hash = None
        self.pending_plan = None
        self.pending_since = None
        self.suppressed_plan = None
        self.broadcasting = False
        if path and os.path.exists(path):
            try:
                with open(path, encoding="utf-8") as state_file:
                    state = json.load(state_file)
            except (OSError, json.JSONDecodeError) as error:
                self._quarantine(path)
                raise StateIdentityError(
                    f"cannot safely read state {path}; quarantined copy retained; recover explicitly: {error}"
                ) from error
            if not isinstance(state, dict):
                self._quarantine(path)
                raise StateIdentityError(f"state {path} is not a JSON object; preserve it for recovery")
            checksum = state.pop("checksum", None)
            if checksum is not None and checksum != self._checksum(state):
                self._quarantine(path)
                raise StateIdentityError(f"state checksum mismatch for {path}; quarantined copy retained")
            if identity is not None:
                if state.get("schema_version") != self.SCHEMA_VERSION or state.get("identity") is None \
                        or checksum is None:
                    raise StateIdentityError(
                        f"legacy state {path} has no trusted identity; explicit migration is required"
                    )
                if state["identity"] != identity:
                    raise StateIdentityError(
                        f"state identity mismatch for {path}: stored={state['identity']!r}, configured={identity!r}"
                    )
            self.pending_hash = state.get("pending_hash")
            self.pending_plan = state.get("pending_plan")
            self.pending_since = state.get("pending_since")
            self.suppressed_plan = state.get("suppressed_plan")
            self.broadcasting = bool(state.get("broadcasting", False))

    @staticmethod
    def _checksum(state):
        canonical = json.dumps(state, sort_keys=True, separators=(",", ":")).encode()
        return "sha256:" + hashlib.sha256(canonical).hexdigest()

    @staticmethod
    def _quarantine(path):
        stamp = time.strftime("%Y%m%dT%H%M%SZ", time.gmtime())
        destination = f"{path}.corrupt.{stamp}"
        try:
            shutil.copy2(path, destination)
            os.chmod(destination, 0o600)
        except OSError:
            pass

    def save(self):
        if not self.path:
            return
        directory = os.path.dirname(os.path.abspath(self.path))
        os.makedirs(directory, mode=0o700, exist_ok=True)
        temporary = self.path + ".tmp"
        state = {
            "schema_version": self.SCHEMA_VERSION,
            "identity": self.identity,
            "pending_hash": self.pending_hash,
            "pending_plan": self.pending_plan,
            "pending_since": self.pending_since,
            "suppressed_plan": self.suppressed_plan,
            "broadcasting": self.broadcasting,
        }
        state["checksum"] = self._checksum(state)
        with open(temporary, "w", encoding="utf-8") as state_file:
            json.dump(state, state_file, sort_keys=True, separators=(",", ":"))
            state_file.flush()
            os.fsync(state_file.fileno())
        os.chmod(temporary, 0o600)
        if os.path.exists(self.path):
            for number in range(self.MAX_BACKUPS, 1, -1):
                older, newer = f"{self.path}.bak.{number - 1}", f"{self.path}.bak.{number}"
                if os.path.exists(older):
                    os.replace(older, newer)
            shutil.copy2(self.path, self.path + ".bak.1")
            os.chmod(self.path + ".bak.1", 0o600)
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


FINGERPRINT_VERSION = 2


def _canonical_value(value, key=""):
    if isinstance(value, dict):
        return {name: _canonical_value(item, name) for name, item in sorted(value.items())}
    if isinstance(value, list):
        return [_canonical_value(item, key) for item in value]
    if isinstance(value, str):
        if any(marker in key for marker in ("address", "token", "recipient", "pair", "vault")):
            return value.strip().lower()
        try:
            number = decimal.Decimal(value)
        except decimal.InvalidOperation:
            return value
        if not number.is_finite():
            return value
        normalized = format(number.normalize(), "f")
        return "0" if normalized in ("-0", "") else normalized
    return value


def plan_fingerprint(plan, message, vault="", chain_id="", config_version="",
                     deadline_seconds=None):
    identity = {
        "version": FINGERPRINT_VERSION,
        "chain_id": chain_id.strip().lower(),
        "vault": vault.strip().lower(),
        "config_version": str(config_version),
        "deadline_seconds": deadline_seconds,
        "action": next(iter(message)),
        "plan": plan,
    }
    canonical = json.dumps(
        _canonical_value(identity), sort_keys=True, separators=(",", ":")
    ).encode()
    return f"v{FINGERPRINT_VERSION}:" + hashlib.sha256(canonical).hexdigest()


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
        "--home",
        args.terrad_home,
    ]


def _credential(args):
    path = getattr(args, "credential_file", None)
    if not isinstance(path, str) or not path:
        return None
    with open(path, encoding="utf-8") as credential:
        return credential.read().rstrip("\r\n") + "\n"


def run_command(command, input_text=None):
    result = subprocess.run(command, input=input_text, text=True, capture_output=True,
                            check=False, timeout=60)
    if result.returncode:
        detail = result.stderr.strip() or result.stdout.strip()
        if is_transient_error(detail):
            raise KeeperError(ErrorCode.TRANSPORT, detail or "temporary terrad failure")
        raise DeterministicTxError(detail or "terrad command failed", ErrorCode.COMMAND_REJECTED)
    return result


def signer_address(args):
    result = run_command([
        args.terrad, "keys", "show", args.key, "--keyring-backend",
        args.keyring_backend, "--address", "--home", args.terrad_home,
    ], _credential(args))
    address = result.stdout.strip().lower()
    if not address:
        raise RuntimeError("terrad key lookup returned no signer address")
    return address


def preflight(vault, message, args):
    run_command(tx_command(vault, message, args) + ["--generate-only"], _credential(args))


def broadcast(vault, message, args):
    result = run_command(
        tx_command(vault, message, args) + ["--broadcast-mode", "sync"], _credential(args)
    )
    try:
        tx = json.loads(result.stdout)
        response = tx.get("tx_response", tx)
        if not isinstance(response, dict) or "code" not in response:
            raise ValueError("missing transaction code")
        code = int(response["code"])
    except (ValueError, TypeError, json.JSONDecodeError) as error:
        raise KeeperError(ErrorCode.AMBIGUOUS_BROADCAST,
                          "invalid sync broadcast response") from error
    if code != 0:
        detail = response.get("raw_log", "")
        if is_transient_error(detail):
            raise KeeperError(ErrorCode.TRANSPORT, f"CheckTx code {code}: {detail}")
        raise DeterministicTxError(f"CheckTx code {code}: {detail}", ErrorCode.CHECK_REJECTED)
    tx_hash = response.get("txhash") or response.get("hash")
    if not tx_hash:
        raise KeeperError(ErrorCode.AMBIGUOUS_BROADCAST,
                          "sync broadcast returned no transaction hash")
    return tx_hash


def query_final_tx(lcd, rpc, tx_hash):
    lcd_url = (
        f"{lcd.rstrip('/')}/cosmos/tx/v1beta1/txs/"
        f"{urllib.parse.quote(tx_hash, safe='')}"
    )
    try:
        response = get_json(lcd_url)["tx_response"]
        observed_hash = response.get("txhash") or response.get("hash")
        if "code" not in response or "height" not in response or not observed_hash:
            raise ValueError("incomplete LCD transaction response")
        if observed_hash.lower() != tx_hash.lower() or int(response["height"]) <= 0:
            raise ValueError("invalid LCD transaction identity")
        return {
            "code": int(response["code"]),
            "raw_log": response.get("raw_log", ""),
            "height": response["height"],
            "hash": observed_hash,
        }
    except Exception:
        # LCD transport, HTTP, and schema failures all fall through to RPC.
        pass

    rpc_url = f"{rpc.rstrip('/')}/tx?{urllib.parse.urlencode({'hash': '0x' + tx_hash})}"
    try:
        response = get_json(rpc_url).get("result")
    except urllib.error.HTTPError as error:
        if error.code in (404, 500):
            return None
        raise
    if not response:
        return None
    try:
        result = response["tx_result"]
        observed_hash = response.get("hash") or response.get("txhash")
        if "code" not in result or "height" not in response or not observed_hash:
            return None
        if observed_hash.lower() != tx_hash.lower() or int(response["height"]) <= 0:
            return None
        return {
            "code": int(result["code"]),
            "raw_log": result.get("log", ""),
            "height": response["height"],
            "hash": observed_hash,
        }
    except (AttributeError, KeyError, TypeError, ValueError):
        return None


def query_latest_height(rpc):
    response = get_json(f"{rpc.rstrip('/')}/status")
    return int(response["result"]["sync_info"]["latest_block_height"])


def query_account(lcd, address):
    result = get_json(
        f"{lcd.rstrip('/')}/cosmos/auth/v1beta1/accounts/"
        f"{urllib.parse.quote(address, safe='')}"
    ).get("account")
    while isinstance(result, dict) and "base_account" in result:
        result = result["base_account"]
    if not isinstance(result, dict) or "account_number" not in result or "sequence" not in result:
        raise RuntimeError("account query returned incomplete identity")
    return {"address": address, "account_number": str(result["account_number"]),
            "sequence": str(result["sequence"])}


def _audit(path, action, detail):
    audit_path = path + ".audit.jsonl"
    record = {"timestamp": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
              "action": action, "detail": detail}
    fd = os.open(audit_path, os.O_WRONLY | os.O_APPEND | os.O_CREAT, 0o600)
    try:
        os.write(fd, (json.dumps(record, sort_keys=True, separators=(",", ":")) + "\n").encode())
        os.fsync(fd)
    finally:
        os.close(fd)


def _timestamped_backup(path):
    stamp = time.strftime("%Y%m%dT%H%M%SZ", time.gmtime())
    destination = f"{path}.{stamp}.bak"
    shutil.copy2(path, destination)
    os.chmod(destination, 0o600)
    return destination


def diagnose_state(args, tracker, address):
    report = {
        "state_file": os.path.abspath(args.state_file),
        "checksum": "ok",
        "broadcasting": tracker.broadcasting,
        "pending_hash": tracker.pending_hash,
        "suppressed_plan": tracker.suppressed_plan,
        "backups": [f"{args.state_file}.bak.{number}" for number in range(1, tracker.MAX_BACKUPS + 1)
                    if os.path.exists(f"{args.state_file}.bak.{number}")],
    }
    try:
        report["account"] = query_account(args.lcd, address)
        report["vault_plan"] = smart_query(args.lcd, args.vault, {"rebalance_plan": {}})
        if tracker.pending_hash:
            report["transaction"] = query_final_tx(args.lcd, args.rpc, tracker.pending_hash)
    except Exception as error:
        report["chain_error"] = {"type": type(error).__name__}
    _audit(args.state_file, "diagnose", report)
    return report


def diagnose_corrupt_state(args, identity, address, error):
    report = {"state_file": os.path.abspath(args.state_file), "checksum": "failed",
              "error_type": type(error).__name__, "backups": []}
    for number in range(1, TxTracker.MAX_BACKUPS + 1):
        path = f"{args.state_file}.bak.{number}"
        if not os.path.exists(path):
            continue
        try:
            candidate = TxTracker(path, identity)
            report["backups"].append({"number": number, "valid": True,
                                      "pending_hash": candidate.pending_hash,
                                      "broadcasting": candidate.broadcasting})
        except StateIdentityError:
            report["backups"].append({"number": number, "valid": False})
    try:
        report["account"] = query_account(args.lcd, address)
        report["vault_plan"] = smart_query(args.lcd, args.vault, {"rebalance_plan": {}})
    except Exception as chain_error:
        report["chain_error"] = {"type": type(chain_error).__name__}
    _audit(args.state_file, "diagnose-corruption", report)
    return report


def resolve_state(args, tracker, address, reason):
    account = query_account(args.lcd, address)
    plan = smart_query(args.lcd, args.vault, {"rebalance_plan": {}})
    if tracker.broadcasting and not tracker.pending_hash:
        raise StateIdentityError("unknown broadcast cannot be resolved safely; rebroadcast remains forbidden")
    tx = query_final_tx(args.lcd, args.rpc, tracker.pending_hash) if tracker.pending_hash else None
    if tracker.pending_hash:
        if reason != "pending-final" or tx is None:
            raise StateIdentityError("pending transaction remains unresolved or reason does not match")
        if query_latest_height(args.rpc) < int(tx["height"]) + args.confirmation_blocks:
            raise StateIdentityError("pending transaction is not yet final")
    elif reason != "deterministic-failure" or not tracker.suppressed_plan:
        raise StateIdentityError("reason does not match a reason-specific suppression")
    backup = _timestamped_backup(args.state_file)
    if tracker.pending_hash and tx is not None and int(tx["code"]) != 0:
        tracker.suppressed_plan = tracker.pending_plan
    elif tracker.pending_hash:
        tracker.suppressed_plan = None
    else:
        tracker.suppressed_plan = None
    tracker.pending_hash = None
    tracker.pending_plan = None
    tracker.pending_since = None
    tracker.broadcasting = False
    tracker.save()
    detail = {"reason": reason, "backup": backup, "account": account,
              "transaction": tx, "vault_should_rebalance": plan.get("should_rebalance")}
    _audit(args.state_file, "resolve", detail)
    return detail


def restore_backup(args, identity, address, number):
    if number < 1 or number > TxTracker.MAX_BACKUPS:
        raise StateIdentityError("backup number is outside the bounded recovery set")
    source = f"{args.state_file}.bak.{number}"
    backup_tracker = TxTracker(source, identity)
    account = query_account(args.lcd, address)
    plan = smart_query(args.lcd, args.vault, {"rebalance_plan": {}})
    if backup_tracker.broadcasting and not backup_tracker.pending_hash:
        raise StateIdentityError("backup contains an unknown broadcast and cannot be restored")
    transaction = None
    if backup_tracker.pending_hash:
        transaction = query_final_tx(args.lcd, args.rpc, backup_tracker.pending_hash)
        if transaction is None:
            raise StateIdentityError("backup transaction remains unresolved or ambiguous")
        if query_latest_height(args.rpc) < int(transaction["height"]) + args.confirmation_blocks:
            raise StateIdentityError("backup transaction is not yet final")
    preserved = _timestamped_backup(args.state_file) if os.path.exists(args.state_file) else None
    temporary = args.state_file + ".restore.tmp"
    shutil.copy2(source, temporary)
    os.chmod(temporary, 0o600)
    os.replace(temporary, args.state_file)
    detail = {"source": source, "preserved": preserved, "account": account,
              "transaction": transaction, "vault_should_rebalance": plan.get("should_rebalance")}
    _audit(args.state_file, "restore-backup", detail)
    return detail


def poll_pending(args, tracker, query_tx=query_final_tx, sleep=time.sleep,
                 latest_height=query_latest_height):
    deadline = time.monotonic() + args.tx_timeout_seconds
    while True:
        result = query_tx(args.lcd, args.rpc, tracker.pending_hash)
        if result is not None:
            try:
                if not all(key in result for key in ("code", "height", "hash")):
                    raise ValueError
                if result["hash"].lower() != tracker.pending_hash.lower() or int(result["height"]) <= 0:
                    raise ValueError
                code = int(result["code"])
            except (AttributeError, TypeError, ValueError):
                result = None
        if result is not None:
            confirmation_blocks = getattr(args, "confirmation_blocks", 0)
            tx_height = int(result["height"])
            if confirmation_blocks and latest_height(args.rpc) < tx_height + confirmation_blocks:
                if time.monotonic() >= deadline:
                    return False
                sleep(args.tx_poll_seconds)
                continue
            tx_hash = tracker.pending_hash
            plan = tracker.pending_plan
            tracker.pending_hash = None
            tracker.pending_plan = None
            tracker.pending_since = None
            tracker.broadcasting = False
            if code != 0:
                tracker.suppressed_plan = plan
                tracker.save()
                raise DeterministicTxError(
                    f"DeliverTx {tx_hash} code {result['code']}: {result['raw_log']}",
                    ErrorCode.DELIVER_REJECTED,
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
    fingerprint = plan_fingerprint(
        plan,
        message,
        vault=args.vault,
        chain_id=args.chain_id,
        config_version=args.config_version,
        deadline_seconds=args.deadline_seconds,
    )
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


def parse_args(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", nargs="?", choices=("run", "diagnose", "resolve"), default="run")
    parser.add_argument("--vault", default=os.getenv("KEEPER_VAULT_ADDRESS"))
    parser.add_argument("--lcd", default=os.getenv("KEEPER_LCD_URL", "http://127.0.0.1:1317"))
    parser.add_argument("--rpc", default=os.getenv("KEEPER_RPC_URL", "http://127.0.0.1:26657"))
    parser.add_argument("--chain-id", default=os.getenv("KEEPER_CHAIN_ID", "localterra"))
    parser.add_argument("--key", default=os.getenv("KEEPER_KEY_NAME", "test1"))
    parser.add_argument("--keyring-backend", default=os.getenv("KEEPER_KEYRING_BACKEND", "os"))
    parser.add_argument("--terrad", default=os.getenv("KEEPER_TERRAD", "terrad"))
    parser.add_argument("--terrad-home", default=os.getenv("KEEPER_TERRAD_HOME", os.path.expanduser("~/.terra")))
    parser.add_argument("--credential-file", default=os.getenv("KEEPER_KEYRING_PASSWORD_FILE"))
    parser.add_argument("--gas-prices", default=os.getenv("KEEPER_GAS_PRICES", "28.325uluna"))
    parser.add_argument("--gas-adjustment", default=os.getenv("KEEPER_GAS_ADJUSTMENT", "1.4"))
    parser.add_argument("--deadline-seconds", type=int, default=int(os.getenv("KEEPER_DEADLINE_SECONDS", "120")))
    parser.add_argument("--poll-seconds", type=int, default=int(os.getenv("KEEPER_POLL_SECONDS", "15")))
    parser.add_argument("--tx-poll-seconds", type=float, default=float(os.getenv("KEEPER_TX_POLL_SECONDS", "2")))
    parser.add_argument("--tx-timeout-seconds", type=float, default=float(os.getenv("KEEPER_TX_TIMEOUT_SECONDS", "60")))
    parser.add_argument("--confirmation-blocks", type=int, default=int(os.getenv("KEEPER_CONFIRMATION_BLOCKS", "2")))
    parser.add_argument("--config-version", default=os.getenv("KEEPER_CONFIG_VERSION", "1"))
    parser.add_argument("--state-file", default=os.getenv("KEEPER_STATE_FILE", ".keeper-state.json"))
    parser.add_argument("--once", action="store_true")
    parser.add_argument("--broadcast", action="store_true")
    parser.add_argument("--diagnose", action="store_true")
    parser.add_argument("--resolve", choices=("deterministic-failure", "pending-final"))
    parser.add_argument("--reason", choices=("deterministic-failure", "pending-final"))
    parser.add_argument("--restore-backup", type=int, choices=range(1, TxTracker.MAX_BACKUPS + 1))
    args = parser.parse_args(argv)
    if args.command == "diagnose":
        args.diagnose = True
    if args.command == "resolve":
        args.resolve = args.reason or args.resolve
        if not args.resolve:
            parser.error("resolve requires --reason")
    if not args.vault:
        parser.error("--vault or KEEPER_VAULT_ADDRESS is required")
    if args.deadline_seconds <= 0 or not math.isfinite(args.tx_poll_seconds) \
            or not math.isfinite(args.tx_timeout_seconds) or args.tx_poll_seconds <= 0 \
            or args.tx_timeout_seconds <= 0 \
            or args.poll_seconds <= 0 or args.confirmation_blocks < 0:
        parser.error("deadline and transaction polling values must be positive")
    return args


def main(argv=None):
    args = parse_args(argv)
    try:
        with ProcessLock(args.state_file):
            identity = {
                "chain_id": args.chain_id,
                "vault": args.vault.lower(),
                "signer": f"{args.keyring_backend}:{args.key}:{signer_address(args)}",
                "protocol_kind": "rebalancer",
            }
            address = identity["signer"].rsplit(":", 1)[-1]
            if args.restore_backup:
                print(json.dumps(restore_backup(args, identity, address, args.restore_backup),
                                 indent=2, sort_keys=True))
                return
            try:
                tracker = TxTracker(args.state_file, identity)
            except StateIdentityError as error:
                if not args.diagnose:
                    raise
                print(json.dumps(diagnose_corrupt_state(args, identity, address, error),
                                 indent=2, sort_keys=True))
                return
            if not os.path.exists(args.state_file):
                tracker.save()
            if args.diagnose:
                print(json.dumps(diagnose_state(args, tracker, address),
                                 indent=2, sort_keys=True))
                return
            if args.resolve:
                print(json.dumps(resolve_state(args, tracker, address,
                                               args.resolve), indent=2, sort_keys=True))
                return
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
    except (StateIdentityError, StateLockError) as error:
        raise SystemExit(f"keeper startup refused: {error}") from error


if __name__ == "__main__":
    main()

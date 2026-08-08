import json
import subprocess
import urllib.parse
import urllib.request
import base64


class RpcError(RuntimeError):
    pass


class TendermintRPC:
    def __init__(self, url: str, timeout: int = 20):
        self.url = url.rstrip("/")
        self.timeout = timeout

    def get(self, method: str, **params):
        url = f"{self.url}/{method}?{urllib.parse.urlencode(params)}"
        try:
            with urllib.request.urlopen(url, timeout=self.timeout) as response:
                body = json.load(response)
        except Exception as exc:
            raise RpcError(f"RPC {method} failed: {exc}") from exc
        if body.get("error"):
            raise RpcError(f"RPC {method}: {body['error']}")
        return body["result"]

    def latest_height(self) -> int:
        return int(self.get("status")["sync_info"]["latest_block_height"])

    def block(self, height: int):
        return self.get("block", height=height)

    def block_results(self, height: int):
        return self.get("block_results", height=height)


class Terrad:
    def __init__(self, binary: str, node: str, chain_id: str, key: str, keyring: str,
                  gas_adjustment: str = "1.4", fees: str = "", home: str = "",
                  signer_command: tuple[str, ...] = ()):
        self.binary, self.node, self.chain_id = binary, node, chain_id
        self.key, self.keyring = key, keyring
        self.gas_adjustment, self.fees = gas_adjustment, fees
        self.home, self.signer_command = home, signer_command

    def _run(self, args, stdin=None):
        if self.home:
            args = [*args, "--home", self.home]
        result = subprocess.run([self.binary, *args], input=stdin, capture_output=True, timeout=90)
        if result.returncode:
            raise RpcError(result.stderr.decode(errors="replace").strip() or "terrad failed")
        try:
            return json.loads(result.stdout)
        except json.JSONDecodeError as exc:
            raise RpcError("terrad returned invalid JSON") from exc

    def _signer(self, request: dict) -> dict:
        if not self.signer_command:
            raise RpcError("external signer is not configured")
        encoded = json.dumps(request, sort_keys=True, separators=(",", ":")).encode()
        try:
            result = subprocess.run(list(self.signer_command), input=encoded, capture_output=True,
                                    timeout=90, check=False)
        except (OSError, subprocess.TimeoutExpired) as exc:
            raise RpcError(f"external signer unavailable: {type(exc).__name__}") from exc
        if result.returncode:
            raise RpcError("external signer refused request")
        try:
            response = json.loads(result.stdout)
        except json.JSONDecodeError as exc:
            raise RpcError("external signer returned invalid JSON") from exc
        if not isinstance(response, dict) or set(response) - {"address", "signed_tx"}:
            raise RpcError("external signer response violates protocol")
        return response

    def smart_query(self, contract: str, message: dict):
        result = self._run(["query", "wasm", "contract-state", "smart", contract,
                            json.dumps(message, separators=(",", ":")), "--node", self.node,
                            "--output", "json"])
        return result.get("data", result)

    def key_address(self) -> str:
        if self.signer_command:
            result = self._signer({"version": 1, "action": "address", "chain_id": self.chain_id})
            address = result.get("address")
            if not isinstance(address, str) or not address.strip():
                raise RpcError("external signer returned no address")
            return address.strip().lower()
        result = self._run([
            "keys", "show", self.key, "--keyring-backend", self.keyring, "--output", "json"
        ])
        address = result.get("address") if isinstance(result, dict) else None
        if not isinstance(address, str) or not address.strip():
            raise RpcError("terrad key lookup returned no signer address")
        return address.strip().lower()

    def sign_execute(self, contract: str, message: dict) -> bytes:
        signer_address = self.key_address() if self.signer_command else self.key
        args = ["tx", "wasm", "execute", contract, json.dumps(message, separators=(",", ":")),
                "--from", signer_address, "--keyring-backend", self.keyring, "--chain-id", self.chain_id,
                "--node", self.node, "--gas", "auto", "--gas-adjustment", self.gas_adjustment,
                "--generate-only", "--output", "json"]
        if self.fees:
            args += ["--fees", self.fees]
        unsigned = json.dumps(self._run(args), separators=(",", ":")).encode()
        if self.signer_command:
            result = self._signer({"version": 1, "action": "sign", "chain_id": self.chain_id,
                                   "signer": signer_address,
                                   "unsigned_tx": base64.b64encode(unsigned).decode("ascii")})
            signed = result.get("signed_tx")
            if not isinstance(signed, str):
                raise RpcError("external signer returned no signed transaction")
            try:
                decoded = base64.b64decode(signed, validate=True)
                parsed = json.loads(decoded)
            except (ValueError, json.JSONDecodeError) as exc:
                raise RpcError("external signer returned invalid signed transaction") from exc
            return json.dumps(parsed, separators=(",", ":")).encode()
        # terrad accepts stdin as the conventional '-' transaction file.
        signed = self._run(["tx", "sign", "-", "--from", self.key, "--keyring-backend", self.keyring,
                            "--chain-id", self.chain_id, "--node", self.node, "--output", "json"], unsigned)
        return json.dumps(signed, separators=(",", ":")).encode()

    def preflight(self, contract: str, message: dict) -> dict:
        """Simulate the rebalance tx (--generate-only) without broadcasting."""
        args = ["tx", "wasm", "execute", contract, json.dumps(message, separators=(",", ":")),
                "--from", self.key, "--keyring-backend", self.keyring, "--chain-id", self.chain_id,
                "--node", self.node, "--gas", "auto", "--gas-adjustment", self.gas_adjustment,
                "--generate-only", "--output", "json"]
        if self.fees:
            args += ["--fees", self.fees]
        result = self._run(args)
        if not isinstance(result, dict):
            raise RpcError("preflight returned invalid transaction data")
        return result

    def latest_height(self) -> int:
        return int(self._run(["status", "--node", self.node])["sync_info"]["latest_block_height"])

    def broadcast(self, signed_tx: bytes):
        return self._run(["tx", "broadcast", "-", "--node", self.node,
                          "--broadcast-mode", "sync", "--output", "json"], signed_tx)

    def query_tx(self, tx_hash: str):
        return self._run(["query", "tx", tx_hash, "--node", self.node, "--output", "json"])

    def account_state(self) -> dict:
        address = self.key_address()
        result = self._run(["query", "auth", "account", address, "--node", self.node,
                            "--output", "json"])
        account = result.get("account", result)
        while isinstance(account, dict) and "base_account" in account:
            account = account["base_account"]
        if not isinstance(account, dict) or "account_number" not in account or "sequence" not in account:
            raise RpcError("terrad account query returned incomplete identity")
        return {"address": address, "account_number": str(account["account_number"]),
                "sequence": str(account["sequence"])}

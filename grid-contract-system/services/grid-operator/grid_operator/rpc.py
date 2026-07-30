import json
import subprocess
import urllib.parse
import urllib.request


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
                 gas_adjustment: str = "1.4", fees: str = ""):
        self.binary, self.node, self.chain_id = binary, node, chain_id
        self.key, self.keyring = key, keyring
        self.gas_adjustment, self.fees = gas_adjustment, fees

    def _run(self, args, stdin=None):
        result = subprocess.run([self.binary, *args], input=stdin, capture_output=True, timeout=90)
        if result.returncode:
            raise RpcError(result.stderr.decode(errors="replace").strip() or "terrad failed")
        try:
            return json.loads(result.stdout)
        except json.JSONDecodeError as exc:
            raise RpcError("terrad returned invalid JSON") from exc

    def smart_query(self, contract: str, message: dict):
        result = self._run(["query", "wasm", "contract-state", "smart", contract,
                            json.dumps(message, separators=(",", ":")), "--node", self.node,
                            "--output", "json"])
        return result.get("data", result)

    def sign_execute(self, contract: str, message: dict) -> bytes:
        args = ["tx", "wasm", "execute", contract, json.dumps(message, separators=(",", ":")),
                "--from", self.key, "--keyring-backend", self.keyring, "--chain-id", self.chain_id,
                "--node", self.node, "--gas", "auto", "--gas-adjustment", self.gas_adjustment,
                "--generate-only", "--output", "json"]
        if self.fees:
            args += ["--fees", self.fees]
        unsigned = json.dumps(self._run(args), separators=(",", ":")).encode()
        # terrad accepts stdin as the conventional '-' transaction file.
        signed = self._run(["tx", "sign", "-", "--from", self.key, "--keyring-backend", self.keyring,
                            "--chain-id", self.chain_id, "--node", self.node, "--output", "json"], unsigned)
        return json.dumps(signed, separators=(",", ":")).encode()

    def broadcast(self, signed_tx: bytes):
        return self._run(["tx", "broadcast", "-", "--node", self.node,
                          "--broadcast-mode", "sync", "--output", "json"], signed_tx)

    def query_tx(self, tx_hash: str):
        return self._run(["query", "tx", tx_hash, "--node", self.node, "--output", "json"])

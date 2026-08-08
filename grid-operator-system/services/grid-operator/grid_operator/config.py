import os
import json
from dataclasses import dataclass
from pathlib import Path


def _integer(name: str, default: int) -> int:
    value = int(os.environ.get(name, default))
    if value < 0:
        raise ValueError(f"{name} must not be negative")
    return value


def _positive_integer(name: str, default: int) -> int:
    value = _integer(name, default)
    if value == 0:
        raise ValueError(f"{name} must be positive")
    return value


@dataclass(frozen=True)
class Config:
    db_path: Path
    rpc_url: str
    chain_id: str
    deployment_height: int
    vaults: tuple[str, ...]
    finality_depth: int = 10
    max_orders_per_batch: int = 20
    poll_seconds: int = 6
    tx_timeout_seconds: int = 180
    loop_seconds: int = 5
    terrad: str = "terrad"
    key_name: str = "grid-keeper"
    keyring_backend: str = "os"
    gas_adjustment: str = "1.4"
    fees: str = ""
    terrad_home: str = ""
    signer_command: tuple[str, ...] = ()

    @classmethod
    def from_env(cls) -> "Config":
        vaults = tuple(x.strip() for x in os.environ.get("GRID_VAULTS", "").split(",") if x.strip())
        required = {
            "GRID_RPC_URL": os.environ.get("GRID_RPC_URL", ""),
            "GRID_CHAIN_ID": os.environ.get("GRID_CHAIN_ID", ""),
        }
        missing = [name for name, value in required.items() if not value]
        if missing:
            raise ValueError("missing environment: " + ", ".join(missing))
        signer_command_json = os.environ.get("GRID_SIGNER_COMMAND_JSON", "[]")
        signer_command = json.loads(signer_command_json)
        if not isinstance(signer_command, list) or not all(isinstance(x, str) and x for x in signer_command):
            raise ValueError("GRID_SIGNER_COMMAND_JSON must be a JSON array of nonempty argv strings")
        return cls(
            db_path=Path(os.environ.get("GRID_DB_PATH", "./grid-operator.sqlite3")),
            rpc_url=required["GRID_RPC_URL"].rstrip("/"),
            chain_id=required["GRID_CHAIN_ID"],
            deployment_height=_positive_integer("GRID_DEPLOYMENT_HEIGHT", 1),
            vaults=vaults,
            finality_depth=_integer("GRID_FINALITY_DEPTH", 10),
            max_orders_per_batch=_positive_integer("GRID_MAX_ORDERS_PER_BATCH", 20),
            poll_seconds=_positive_integer("GRID_TX_POLL_SECONDS", 6),
            tx_timeout_seconds=_positive_integer("GRID_TX_TIMEOUT_SECONDS", 180),
            loop_seconds=_positive_integer("GRID_LOOP_SECONDS", 5),
            terrad=os.environ.get("GRID_TERRAD", "terrad"),
            key_name=os.environ.get("GRID_KEY_NAME", "grid-keeper"),
            keyring_backend=os.environ.get("GRID_KEYRING_BACKEND", "os"),
            gas_adjustment=os.environ.get("GRID_GAS_ADJUSTMENT", "1.4"),
            fees=os.environ.get("GRID_FEES", ""),
            terrad_home=os.environ.get("GRID_TERRAD_HOME", ""),
            signer_command=tuple(signer_command),
        )

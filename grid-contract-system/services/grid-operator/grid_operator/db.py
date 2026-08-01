import sqlite3
import time
from contextlib import contextmanager
from pathlib import Path


SCHEMA = """
CREATE TABLE IF NOT EXISTS blocks (
  chain_id TEXT NOT NULL, height INTEGER NOT NULL, hash TEXT NOT NULL,
  parent_hash TEXT, time TEXT, scanned_at INTEGER NOT NULL,
  PRIMARY KEY(chain_id, height)
);
CREATE TABLE IF NOT EXISTS vaults (
  address TEXT PRIMARY KEY, bot_id INTEGER NOT NULL DEFAULT 1 CHECK(bot_id = 1),
  pair_address TEXT, enabled INTEGER NOT NULL DEFAULT 1,
  last_order_refresh_height INTEGER
);
CREATE TABLE IF NOT EXISTS orders (
  pair_address TEXT NOT NULL, order_id INTEGER NOT NULL, vault_address TEXT NOT NULL,
  bot_id INTEGER NOT NULL DEFAULT 1, side TEXT, rung_index INTEGER, price TEXT,
  remaining TEXT, active INTEGER NOT NULL DEFAULT 1, updated_height INTEGER,
  PRIMARY KEY(pair_address, order_id), FOREIGN KEY(vault_address) REFERENCES vaults(address)
);
CREATE TABLE IF NOT EXISTS raw_events (
  id INTEGER PRIMARY KEY, chain_id TEXT NOT NULL, height INTEGER NOT NULL,
  block_hash TEXT NOT NULL, tx_hash TEXT NOT NULL, tx_index INTEGER NOT NULL,
  event_index INTEGER NOT NULL, pair_address TEXT NOT NULL, order_id INTEGER NOT NULL,
  vault_address TEXT NOT NULL, bot_id INTEGER NOT NULL, side TEXT NOT NULL,
  input_amount TEXT NOT NULL, output_amount TEXT NOT NULL, raw_json TEXT NOT NULL,
  reconciled_batch_id INTEGER,
  UNIQUE(chain_id, pair_address, tx_hash, event_index, order_id)
);
CREATE INDEX IF NOT EXISTS raw_events_pending ON raw_events(vault_address, reconciled_batch_id, height);
CREATE TABLE IF NOT EXISTS aggregates (
  pair_address TEXT NOT NULL, order_id INTEGER NOT NULL, vault_address TEXT NOT NULL,
  bot_id INTEGER NOT NULL, input_amount TEXT NOT NULL, output_amount TEXT NOT NULL,
  fill_count INTEGER NOT NULL, first_height INTEGER NOT NULL, through_height INTEGER NOT NULL,
  PRIMARY KEY(pair_address, order_id)
);
CREATE TABLE IF NOT EXISTS batches (
  id INTEGER PRIMARY KEY, vault_address TEXT NOT NULL, bot_id INTEGER NOT NULL,
  through_height INTEGER NOT NULL, state TEXT NOT NULL,
  created_at INTEGER NOT NULL, confirmed_at INTEGER, tx_hash TEXT, error TEXT,
  failure_count INTEGER NOT NULL DEFAULT 0, next_retry_at INTEGER
);
CREATE INDEX IF NOT EXISTS batches_state ON batches(state, vault_address);
CREATE TABLE IF NOT EXISTS batch_items (
  batch_id INTEGER NOT NULL, pair_address TEXT NOT NULL, order_id INTEGER NOT NULL,
  input_amount TEXT NOT NULL, output_amount TEXT NOT NULL, fill_count INTEGER NOT NULL,
  PRIMARY KEY(batch_id, pair_address, order_id), FOREIGN KEY(batch_id) REFERENCES batches(id)
);
CREATE TABLE IF NOT EXISTS batch_events (
  batch_id INTEGER NOT NULL, event_id INTEGER NOT NULL UNIQUE,
  PRIMARY KEY(batch_id, event_id), FOREIGN KEY(batch_id) REFERENCES batches(id)
);
CREATE TABLE IF NOT EXISTS tx_attempts (
  id INTEGER PRIMARY KEY, batch_id INTEGER NOT NULL, state TEXT NOT NULL,
  signed_tx BLOB NOT NULL, tx_hash TEXT, check_code INTEGER, deliver_code INTEGER,
  created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, response_json TEXT, error TEXT,
  FOREIGN KEY(batch_id) REFERENCES batches(id)
);
CREATE TABLE IF NOT EXISTS cursors (
  name TEXT PRIMARY KEY, height INTEGER NOT NULL, value TEXT, updated_at INTEGER NOT NULL
);
PRAGMA user_version = 2;
"""


class Database:
    def __init__(self, path: Path | str):
        self.path = str(path)
        self.conn = sqlite3.connect(self.path, timeout=30, isolation_level=None)
        self.conn.row_factory = sqlite3.Row
        self.conn.execute("PRAGMA journal_mode=WAL")
        self.conn.execute("PRAGMA synchronous=FULL")
        self.conn.execute("PRAGMA foreign_keys=ON")

    def migrate(self) -> None:
        version = self.conn.execute("PRAGMA user_version").fetchone()[0]
        if version > 2:
            raise RuntimeError(f"database schema {version} is newer than this operator")
        if version == 0:
            self.conn.executescript(SCHEMA)
        elif version == 1:
            with self.transaction(immediate=True) as conn:
                conn.execute("ALTER TABLE batches ADD COLUMN failure_count INTEGER NOT NULL DEFAULT 0")
                conn.execute("ALTER TABLE batches ADD COLUMN next_retry_at INTEGER")
                conn.execute("PRAGMA user_version = 2")

    @contextmanager
    def transaction(self, immediate: bool = False):
        self.conn.execute("BEGIN IMMEDIATE" if immediate else "BEGIN")
        try:
            yield self.conn
        except BaseException:
            self.conn.rollback()
            raise
        else:
            self.conn.commit()

    def cursor(self, name: str, default: int = 0) -> int:
        row = self.conn.execute("SELECT height FROM cursors WHERE name=?", (name,)).fetchone()
        return int(row[0]) if row else default

    def set_cursor(self, name: str, height: int, value: str | None = None, conn=None) -> None:
        (conn or self.conn).execute(
            "INSERT INTO cursors(name,height,value,updated_at) VALUES(?,?,?,?) "
            "ON CONFLICT(name) DO UPDATE SET height=excluded.height,value=excluded.value,updated_at=excluded.updated_at",
            (name, height, value, int(time.time())),
        )

    def rebuild_aggregates(self, conn=None) -> None:
        c = conn or self.conn
        c.execute("DELETE FROM aggregates")
        rows = c.execute(
            "SELECT pair_address,order_id,vault_address,bot_id,input_amount,output_amount,height "
            "FROM raw_events WHERE reconciled_batch_id IS NULL ORDER BY id"
        )
        totals = {}
        for row in rows:
            key = (row["pair_address"], row["order_id"])
            item = totals.setdefault(key, [row["vault_address"], row["bot_id"], 0, 0, 0, row["height"], row["height"]])
            item[2] += int(row["input_amount"])
            item[3] += int(row["output_amount"])
            item[4] += 1
            item[6] = row["height"]
        c.executemany(
            "INSERT INTO aggregates VALUES(?,?,?,?,?,?,?,?,?)",
            [(pair, oid, *values[:2], str(values[2]), str(values[3]), *values[4:]) for (pair, oid), values in totals.items()],
        )

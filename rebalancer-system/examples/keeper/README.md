# Keeper Setup And Operation

This directory contains everything needed to run a keeper for one bot vault.
The keeper monitors the vault's on-chain plan and signs the deadline-only
`rebalance` transaction with `terrad`. The vault derives every economic value.

## Files

- `keeper.py`: plan polling, preflight, transaction signing, and finality tracking.
- `config.example.env`: all runtime settings without key material.
- `setup-key.sh`: creates or displays the keeper key.
- `register-keeper.sh`: lets the vault admin assign that key as keeper.
- `run.sh`: loads `config.env` and starts the keeper.
- `cl8y-keeper.service.example`: continuous systemd service example.
- `test_keeper.py`: calculation tests.

## Requirements

- Python 3.9 or newer.
- Terra Classic `terrad` with the `tx wasm execute` command.
- Network access to a Terra Classic LCD and RPC endpoint.
- A dedicated keeper wallet funded with LUNC for transaction gas.
- The vault admin available once to register the keeper address.

The keeper wallet needs no user tokens, bot LP, CL8Y, or vault withdrawal
permission. Its only protocol role is submitting constrained rebalances.

## 1. Configure

Run the relative commands below from the `rebalancer-system` directory.

```sh
cp examples/keeper/config.example.env examples/keeper/config.env
```

Edit `config.env` and set at least:

```text
KEEPER_VAULT_ADDRESS=<BOT_VAULT_ADDRESS>
KEEPER_LCD_URL=<LCD_URL>
KEEPER_RPC_URL=<RPC_URL>
KEEPER_CHAIN_ID=columbus-5
KEEPER_KEY_NAME=cl8y-bot-keeper
KEEPER_KEYRING_BACKEND=os
```

`config.env` is ignored by Git. Do not place a mnemonic or private key in it.

## 2. Create The Keeper Key

```sh
./examples/keeper/setup-key.sh
```

The script calls:

```sh
terrad keys add cl8y-bot-keeper --keyring-backend os
```

Store the recovery phrase offline. The keeper reads the key from the operating
system keyring when it signs. For an existing wallet, recover it manually with
`terrad keys add <name> --recover --keyring-backend os` and enter the recovery
phrase only in the interactive prompt.

## 3. Fund Gas

Display the keeper address:

```sh
terrad keys show cl8y-bot-keeper --keyring-backend os --address
```

Send LUNC to this address and verify its balance:

```sh
terrad query bank balances <KEEPER_ADDRESS> \
  --node <RPC_URL> --output json
```

The keeper should be monitored and refilled before its LUNC balance reaches the
minimum required for another rebalance transaction.

## 4. Register The Keeper On The Vault

The vault admin must send this message to the vault:

```json
{
  "update_keeper": {
    "keeper": "<KEEPER_ADDRESS>"
  }
}
```

To use the included script, temporarily add these local values to `config.env`:

```text
VAULT_ADMIN_KEY_NAME=<ADMIN_KEYRING_NAME>
VAULT_ADMIN_KEYRING_BACKEND=os
```

Then run:

```sh
./examples/keeper/register-keeper.sh
```

Remove the admin key settings from the keeper host afterward when administration
is performed from another machine or multisig workflow.

Verify the vault's `config` query returns the keeper address.

## 5. Dry Run

Dry-run mode queries live contracts and prints the transaction without signing:

```sh
KEEPER_ONCE=1 ./examples/keeper/run.sh
```

The same operation can be run directly:

```sh
python3 examples/keeper/keeper.py \
  --vault terra1... \
  --lcd https://terra-classic-lcd.publicnode.com \
  --rpc https://terra-classic-rpc.publicnode.com:443 \
  --chain-id columbus-5 \
  --key cl8y-bot-keeper \
  --keyring-backend os \
  --once
```

## 6. Broadcast

After checking the dry-run output:

```sh
KEEPER_BROADCAST=1 KEEPER_ONCE=1 ./examples/keeper/run.sh
```

For continuous operation, set `KEEPER_BROADCAST=1` in the service environment
and run without `KEEPER_ONCE`.

## What The Keeper Sends

The keeper sends this message to `KEEPER_VAULT_ADDRESS`:

```json
{
  "rebalance": {
    "deadline": 1800000000
  }
}
```

The keeper first queries `rebalance_plan`. The vault recaptures TWAP during
execution and derives offer side, amount, trade cap, minimum return, and maximum
spread from on-chain configuration. If movement has triggered but allocation is
already within tolerance, the keeper sends `sync_reference` instead.

Before broadcast the keeper runs a `terrad --dry-run` preflight. It parses the
sync CheckTx code and hash, then polls LCD with RPC fallback until DeliverTx is
final and requires code zero. Only one hash can be in flight. Pending hashes and
deterministic-failure suppression are persisted and fsynced in
`KEEPER_STATE_FILE`. A `broadcasting` marker is committed before network access;
if the process restarts before receiving a hash, automatic submission stops for
operator intervention rather than risking a duplicate. Commands have a 60-second
timeout, and transient connection, sequence, timeout, availability, and mempool
errors do not suppress the plan. An unchanged plan rejected deterministically by
CheckTx or DeliverTx remains suppressed until on-chain plan inputs change.
Code-zero inclusion is re-queried until `KEEPER_CONFIRMATION_BLOCKS` later
blocks exist (default 2), so a transaction that disappears in a shallow reorg
remains pending rather than advancing keeper state.

## 7. Run As A Service

Create a dedicated operating-system user and install the repository, keeper
config, and keyring for that user. Adapt the paths in
`cl8y-keeper.service.example`, then install it:

```sh
sudo cp examples/keeper/cl8y-keeper.service.example \
  /etc/systemd/system/cl8y-keeper.service
sudo systemctl daemon-reload
sudo systemctl enable --now cl8y-keeper
sudo journalctl -u cl8y-keeper -f
```

The service restarts after RPC failures or process exits. Its systemd
`StateDirectory` supplies the writable persistent transaction-state path.
Monitor final transaction results, RPC availability, vault allocation, and
keeper LUNC balance.

Before enabling continuous broadcast, run one signed transaction as the
service user and confirm its keyring can unlock noninteractively. If the chosen
keyring requires an interactive password or desktop session, use a reviewed
signer wrapper as `KEEPER_TERRAD` or an operator key-management setup that can
sign unattended without placing private-key material in `config.env`.

## Key Rotation

1. Create and fund a new keeper key.
2. Dry-run the new keeper configuration.
3. Have the vault admin execute `update_keeper` with the new address.
4. Start the new keeper and stop the old service.
5. Remove the old key after the operational rollback period.

## Tests

```sh
make test-keeper
```

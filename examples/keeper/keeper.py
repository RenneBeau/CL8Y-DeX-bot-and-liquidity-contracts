#!/usr/bin/env python3
"""Minimal CL8Y bot-vault keeper.

Dry-run is the default. Pass --broadcast to sign with a terrad keyring entry.
"""

import argparse
import base64
import json
import subprocess
import time
import urllib.parse
import urllib.request


def smart_query(lcd, contract, message):
    encoded = base64.b64encode(
        json.dumps(message, separators=(",", ":")).encode()
    ).decode()
    url = (
        f"{lcd.rstrip('/')}/cosmwasm/wasm/v1/contract/"
        f"{urllib.parse.quote(contract, safe='')}/smart/{encoded}"
    )
    with urllib.request.urlopen(url, timeout=15) as response:
        return json.load(response)["data"]


def calculate_offer(balances, reserves, max_trade_bps):
    """Return (token_index, amount) that moves holdings toward pool ratio."""
    balance0, balance1 = map(int, balances)
    reserve0, reserve1 = map(int, reserves)
    if min(reserve0, reserve1) <= 0 or balance0 + balance1 == 0:
        return None

    cross = balance1 * reserve0 - balance0 * reserve1
    if cross > 0:
        amount = cross // (2 * reserve0)
        index = 1
        balance = balance1
    elif cross < 0:
        amount = (-cross) // (2 * reserve1)
        index = 0
        balance = balance0
    else:
        return None

    cap = balance * max_trade_bps // 10_000
    amount = min(amount, cap)
    return (index, amount) if amount > 0 else None


def quote_swap(lcd, pair, proxy, offer_token, amount):
    return smart_query(
        lcd,
        pair,
        {
            "hybrid_simulation": {
                "offer_asset": {
                    "info": {"token": {"contract_addr": offer_token}},
                    "amount": str(amount),
                },
                "hybrid": {
                    "pool_input": str(amount),
                    "book_input": "0",
                    "max_maker_fills": 1,
                    "book_start_hint": None,
                },
                "trader": proxy,
                "sender": None,
                "belief_price": None,
            }
        },
    )


def build_rebalance(config, status, balances, pool, args):
    if not status["should_rebalance"]:
        return None

    reserves = [asset["amount"] for asset in pool["assets"]]
    offer = calculate_offer(balances, reserves, args.max_trade_bps)
    if offer is None:
        return None
    token_index, amount = offer
    offer_token = config["asset_tokens"][token_index]
    quote = quote_swap(
        args.lcd, config["pair"], config["proxy"], offer_token, amount
    )
    quoted_return = int(quote["return_amount"])
    min_return = quoted_return * (10_000 - args.slippage_bps) // 10_000
    if min_return <= 0:
        raise RuntimeError("quote produced zero minimum return")

    deadline = int(time.time()) + args.deadline_seconds
    return {
        "rebalance": {
            "params": {
                "offer_token": offer_token,
                "amount": str(amount),
                "min_return": str(min_return),
                "max_spread": args.max_spread,
                "deadline": deadline,
            }
        }
    }


def broadcast(vault, message, args):
    command = [
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
        "--broadcast-mode",
        "sync",
        "--yes",
        "--output",
        "json",
    ]
    result = subprocess.run(command, check=True, text=True, capture_output=True)
    tx = json.loads(result.stdout)
    print(f"broadcast tx: {tx.get('txhash', 'unknown')}")


def run_once(args):
    config = smart_query(args.lcd, args.vault, {"config": {}})
    status = smart_query(args.lcd, args.vault, {"rebalance_status": {}})
    if not status["should_rebalance"]:
        print(
            "no rebalance: "
            f"price deviation {status['price_deviation_bps']} bps"
        )
        return

    balances = smart_query(args.lcd, args.vault, {"balances": {}})["balances"]
    pool = smart_query(args.lcd, config["pair"], {"pool": {}})
    message = build_rebalance(config, status, balances, pool, args)
    if message is None:
        print("no rebalance: vault allocation already matches the pool")
        return

    print(json.dumps(message, indent=2))
    if args.broadcast:
        broadcast(args.vault, message, args)
    else:
        print("dry-run only; pass --broadcast to sign and submit")


def parse_args():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--vault", required=True, help="Bot vault address")
    parser.add_argument("--lcd", default="http://127.0.0.1:1317")
    parser.add_argument("--rpc", default="http://127.0.0.1:26657")
    parser.add_argument("--chain-id", default="localterra")
    parser.add_argument("--key", default="test1", help="terrad keyring name")
    parser.add_argument("--keyring-backend", default="test")
    parser.add_argument("--terrad", default="terrad")
    parser.add_argument("--gas-prices", default="28.325uluna")
    parser.add_argument("--gas-adjustment", default="1.4")
    parser.add_argument("--slippage-bps", type=int, default=200)
    parser.add_argument("--max-spread", default="0.05")
    parser.add_argument("--max-trade-bps", type=int, default=2500)
    parser.add_argument("--deadline-seconds", type=int, default=120)
    parser.add_argument("--poll-seconds", type=int, default=15)
    parser.add_argument("--once", action="store_true")
    parser.add_argument("--broadcast", action="store_true")
    args = parser.parse_args()
    if not 0 <= args.slippage_bps < 10_000:
        parser.error("--slippage-bps must be between 0 and 9999")
    if not 1 <= args.max_trade_bps <= 10_000:
        parser.error("--max-trade-bps must be between 1 and 10000")
    return args


def main():
    args = parse_args()
    while True:
        try:
            run_once(args)
        except Exception as error:  # Keep a long-running keeper alive on RPC errors.
            print(f"keeper error: {error}")
            if args.once:
                raise
        if args.once:
            return
        time.sleep(args.poll_seconds)


if __name__ == "__main__":
    main()

"""Vault protocol abstraction for the discovery keeper.

Each kind of vault has its own query, decision and execute-message shape:

- ``grid`` (``grid-vault-swap``): permissionless rebalance driven by
  ``{"grid_status": {}}``.
- ``rebalance`` (``bot-vault``): keeper-restricted rebalance driven by
  ``{"rebalance_plan": {}}``; a pure reference-price drift sends
  ``{"sync_reference": {}}`` instead of a swap.

The keeper key must be the address authorized by each ``rebalance`` vault's
``config.keeper``; grid rebalances are open to any signer.
"""

import decimal
import hashlib
import json


def fingerprint_v1(plan, message, vault, deadline_seconds):
    identity = {
        "version": 1,
        "vault": vault.strip().lower(),
        "deadline_seconds": deadline_seconds,
        "action": next(iter(message)),
        "plan": plan,
    }
    canonical = json.dumps(identity, sort_keys=True, separators=(",", ":")).encode()
    return "v1:" + hashlib.sha256(canonical).hexdigest()


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


def fingerprint_v2(plan, message, vault, chain_id, config_version, deadline_seconds):
    identity = {
        "version": 2,
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
    return "v2:" + hashlib.sha256(canonical).hexdigest()


class VaultProtocol:
    kind = ""
    query_msg = {}
    query_label = ""

    def plan(self, terrad, vault):
        return terrad.smart_query(vault, self.query_msg)

    def build_message(self, plan, deadline):
        raise NotImplementedError

    def noop_message(self, plan):
        return "no rebalance required"

    def fingerprint(self, plan, message, vault, args):
        raise NotImplementedError


class GridSwapProtocol(VaultProtocol):
    kind = "grid"
    query_msg = {"grid_status": {}}
    query_label = "grid_status"

    def build_message(self, plan, deadline):
        if not plan.get("should_rebalance"):
            return None
        if plan.get("pending_swap"):
            return None
        return {"rebalance": {"deadline": deadline}}

    def noop_message(self, plan):
        return (
            f"no rebalance: should_rebalance={plan.get('should_rebalance')} "
            f"pending_swap={plan.get('pending_swap')} "
            f"cell={plan.get('current_cell')} "
            f"deviation_bps={plan.get('allocation_deviation_bps')}"
        )

    def fingerprint(self, plan, message, vault, args):
        return fingerprint_v1(plan, message, vault, args.deadline_seconds)


class RebalanceProtocol(VaultProtocol):
    kind = "rebalance"
    query_msg = {"rebalance_plan": {}}
    query_label = "rebalance_plan"

    def build_message(self, plan, deadline):
        if not plan.get("should_rebalance"):
            return None
        if plan.get("offer_token") is None:
            return {"sync_reference": {}}
        return {"rebalance": {"deadline": deadline}}

    def noop_message(self, plan):
        return (
            f"no rebalance: price_deviation_bps={plan.get('price_deviation_bps')} bps "
            f"allocation_deviation_bps={plan.get('allocation_deviation_bps')} bps"
        )

    def fingerprint(self, plan, message, vault, args):
        return fingerprint_v2(
            plan, message, vault,
            getattr(args, "chain_id", ""),
            getattr(args, "config_version", "1"),
            args.deadline_seconds,
        )


grid_protocol = GridSwapProtocol()
rebalance_protocol = RebalanceProtocol()
PROTOCOLS = {"grid": grid_protocol, "rebalance": rebalance_protocol}

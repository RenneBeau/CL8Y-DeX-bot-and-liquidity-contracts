# CL8Y Settlement Extension

`limit-order-settlement.patch` applies to the LocalTerra harness's pinned CL8Y
revision. It adds a persistent `limit_order_settlement` query containing:

- The order owner, side, and price
- Initial and current maker escrow
- Exact cumulative maker output recorded during each fill
- Open, filled, or parked status
- Any parked input refund that can be claimed

The multi-user grid manager requires this query. A shared maker address receives
fills from many bots, and per-fill integer rounding makes exact output impossible
to reconstruct from the decrease in escrow alone. The manager credits a bot only
from the authoritative cumulative-output delta and claims parked refunds before
removing the order from that bot's ledger.

The LocalTerra helper in `test-area/common.sh` applies the patch idempotently.
Production deployment requires the equivalent CL8Y pair release and a migration
policy for orders created before settlement records existed.

pub const MAX_GRID_COUNT: u32 = 100;
pub const MAX_ORDERS_PER_RECONCILE: u32 = 100;
pub const MAX_ACTIVE_ORDERS_PER_VAULT: u32 = 500;

pub fn valid_vault_limits(
    max_grid_count: u32,
    max_orders_per_reconcile: u32,
    max_active_orders: u32,
) -> bool {
    (2..=MAX_GRID_COUNT).contains(&max_grid_count)
        && (1..=MAX_ORDERS_PER_RECONCILE).contains(&max_orders_per_reconcile)
        && (max_grid_count..=MAX_ACTIVE_ORDERS_PER_VAULT).contains(&max_active_orders)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_exact_vault_boundaries() {
        assert!(valid_vault_limits(2, 1, 2));
        assert!(valid_vault_limits(
            MAX_GRID_COUNT,
            MAX_ORDERS_PER_RECONCILE,
            MAX_ACTIVE_ORDERS_PER_VAULT,
        ));
    }

    #[test]
    fn rejects_every_out_of_range_manager_limit() {
        assert!(!valid_vault_limits(1, 1, 2));
        assert!(!valid_vault_limits(MAX_GRID_COUNT + 1, 1, 101));
        assert!(!valid_vault_limits(2, 0, 2));
        assert!(!valid_vault_limits(2, MAX_ORDERS_PER_RECONCILE + 1, 2));
        assert!(!valid_vault_limits(3, 1, 2));
        assert!(!valid_vault_limits(2, 1, MAX_ACTIVE_ORDERS_PER_VAULT + 1,));
    }
}

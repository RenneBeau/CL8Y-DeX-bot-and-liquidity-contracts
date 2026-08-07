use bot_types::{SwapParams, WithdrawalType};
use cosmwasm_schema::{cw_serde, QueryResponses};
use cosmwasm_std::{Binary, Uint128};
use cw20::Logo;
use cw20_base::msg::InstantiateMarketingInfo;

#[cw_serde]
pub struct InstantiateMsg {
    pub admin: String,
    pub vault: String,
    pub name: String,
    pub symbol: String,
    pub decimals: u8,
    pub minimum_initial_deposit: Uint128,
    pub marketing: Option<InstantiateMarketingInfo>,
}

#[cw_serde]
pub struct MigrateMsg {}

#[cw_serde]
pub enum ExecuteMsg {
    UpdateConfig {
        minimum_initial_deposit: Option<Uint128>,
    },
    TransferAdmin {
        admin: String,
    },
    AcceptAdmin {},
    CancelAdminTransfer {},
    Deposit {
        amounts: [Uint128; 2],
        min_shares: Uint128,
        deadline: u64,
        swap: Option<SwapParams>,
    },
    Withdraw {
        shares: Uint128,
        recipient: Option<String>,
        deadline: u64,
        output: WithdrawalType,
    },
    MintTo {
        recipient: String,
        amount: Uint128,
    },
    Transfer {
        recipient: String,
        amount: Uint128,
    },
    Burn {
        amount: Uint128,
    },
    Send {
        contract: String,
        amount: Uint128,
        msg: Binary,
    },
    IncreaseAllowance {
        spender: String,
        amount: Uint128,
        expires: Option<cw20::Expiration>,
    },
    DecreaseAllowance {
        spender: String,
        amount: Uint128,
        expires: Option<cw20::Expiration>,
    },
    TransferFrom {
        owner: String,
        recipient: String,
        amount: Uint128,
    },
    BurnFrom {
        owner: String,
        amount: Uint128,
    },
    SendFrom {
        owner: String,
        contract: String,
        amount: Uint128,
        msg: Binary,
    },
    UpdateMarketing {
        project: Option<String>,
        description: Option<String>,
        marketing: Option<String>,
    },
    UploadLogo(Logo),
}

#[cw_serde]
#[derive(QueryResponses)]
pub enum QueryMsg {
    #[returns(ConfigResponse)]
    Config {},
    #[returns(bot_types::LiquidityAuthorizationResponse)]
    Authorization {},
    #[returns(cw20::BalanceResponse)]
    Balance { address: String },
    #[returns(cw20::TokenInfoResponse)]
    TokenInfo {},
    #[returns(Option<cw20::MinterResponse>)]
    Minter {},
    #[returns(cw20::AllowanceResponse)]
    Allowance { owner: String, spender: String },
    #[returns(cw20::AllAllowancesResponse)]
    AllAllowances {
        owner: String,
        start_after: Option<String>,
        limit: Option<u32>,
    },
    #[returns(cw20::AllSpenderAllowancesResponse)]
    AllSpenderAllowances {
        spender: String,
        start_after: Option<String>,
        limit: Option<u32>,
    },
    #[returns(cw20::AllAccountsResponse)]
    AllAccounts {
        start_after: Option<String>,
        limit: Option<u32>,
    },
    #[returns(cw20::MarketingInfoResponse)]
    MarketingInfo {},
    #[returns(cw20::DownloadLogoResponse)]
    DownloadLogo {},
}

#[cw_serde]
pub struct ConfigResponse {
    pub admin: String,
    pub pending_admin: Option<String>,
    pub vault: String,
    pub asset_tokens: [String; 2],
    pub minimum_initial_deposit: Uint128,
}

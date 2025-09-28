//! StellarLend Soroban Smart Contract
//
//! This contract provides the foundation for the StellarLend DeFi Lending & Borrowing Protocol.
//! Core features will be implemented incrementally in separate modules.

#![no_std]
extern crate alloc;

use alloc::format;
use alloc::string::ToString;
use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, vec, Address, Bytes, Env,
    IntoVal, Map, String, Symbol, Vec,
};
use soroban_sdk::token::TokenClient;
mod oracle;
use oracle::{Oracle, OracleSource, OracleStorage};
mod governance;
use governance::{Governance, GovStorage, Proposal};
mod flash_loan;
use flash_loan::FlashLoan;

// Global allocator for Soroban contracts
#[global_allocator]
static ALLOC: wee_alloc::WeeAlloc = wee_alloc::WeeAlloc::INIT;

#[cfg(test)]
mod test;

// Core protocol modules
mod deposit;
mod borrow;
mod repay;
mod withdraw;
mod liquidate;
mod analytics;

/// Reentrancy guard for security
pub struct ReentrancyGuard;

impl ReentrancyGuard {
    fn key(env: &Env) -> Symbol { Symbol::new(env, "reentrancy") }
    pub fn enter(env: &Env) -> Result<(), ProtocolError> {
        let entered = env.storage().instance().get::<Symbol, bool>(&Self::key(env)).unwrap_or(false);
        if entered {
            let error = ProtocolError::ReentrancyDetected;
            return Err(error);
        }
        env.storage().instance().set(&Self::key(env), &true);
        Ok(())
    }
    pub fn exit(env: &Env) {
        env.storage().instance().set(&Self::key(env), &false);
    }
}

/// The main contract struct for StellarLend
#[contract]
pub struct Contract;

/// Represents a user's position in the protocol
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct Position {
    /// The address of the user
    pub user: Address,
    /// The amount of collateral deposited
    pub collateral: i128,
    /// The amount borrowed
    pub debt: i128,
    /// Accrued borrow interest (scaled by 1e8)
    pub borrow_interest: i128,
    /// Accrued supply interest (scaled by 1e8)
    pub supply_interest: i128,
    /// Last time interest was accrued for this position
    pub last_accrual_time: u64,
}

impl Position {
    /// Create a new position
    pub fn new(user: Address, collateral: i128, debt: i128) -> Self {
        Self {
            user,
            collateral,
            debt,
            borrow_interest: 0,
            supply_interest: 0,
            last_accrual_time: 0,
        }
    }
}

/// Interest rate configuration parameters
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct InterestRateConfig {
    /// Base interest rate (scaled by 1e8, e.g., 2% = 2000000)
    pub base_rate: i128,
    /// Utilization point where rate increases (scaled by 1e8, e.g., 80% = 80000000)
    pub kink_utilization: i128,
    /// Rate multiplier above kink (scaled by 1e8, e.g., 10x = 10000000)
    pub multiplier: i128,
    /// Protocol fee percentage (scaled by 1e8, e.g., 10% = 10000000)
    pub reserve_factor: i128,
    /// Maximum allowed rate (scaled by 1e8, e.g., 50% = 50000000)
    pub rate_ceiling: i128,
    /// Minimum allowed rate (scaled by 1e8, e.g., 0.1% = 100000)
    pub rate_floor: i128,
    /// Last time config was updated
    pub last_update: u64,
    /// Smoothing factor in bps for rate changes (0..=10000)
    pub smoothing_bps: i128,
    /// Volatility sensitivity in bps (impact of utilization change)
    pub util_sensitivity_bps: i128,
}

impl InterestRateConfig {
    /// Create default interest rate configuration
    pub fn default() -> Self {
        Self {
            base_rate: 2000000,         // 2%
            kink_utilization: 80000000, // 80%
            multiplier: 10000000,       // 10x
            reserve_factor: 10000000,   // 10%
            rate_ceiling: 50000000,     // 50%
            rate_floor: 100000,         // 0.1%
            last_update: 0,
            smoothing_bps: 2000,        // 20% smoothing by default
            util_sensitivity_bps: 100,  // 1% per 1% util change
        }
    }
}

/// Current interest rate state
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct InterestRateState {
    /// Current borrow rate (scaled by 1e8)
    pub current_borrow_rate: i128,
    /// Current supply rate (scaled by 1e8)
    pub current_supply_rate: i128,
    /// Current utilization rate (scaled by 1e8)
    pub utilization_rate: i128,
    /// Total borrowed amount
    pub total_borrowed: i128,
    /// Total supplied amount
    pub total_supplied: i128,
    /// Last time interest was accrued
    pub last_accrual_time: u64,
    /// Smoothed borrow rate
    pub smoothed_borrow_rate: i128,
}

impl InterestRateState {
    /// Create initial interest rate state
    pub fn initial() -> Self {
        Self {
            current_borrow_rate: 0,
            current_supply_rate: 0,
            utilization_rate: 0,
            total_borrowed: 0,
            total_supplied: 0,
            last_accrual_time: 0,
            smoothed_borrow_rate: 0,
        }
    }
}

/// Risk management configuration
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct RiskConfig {
    /// Max % of debt that can be repaid in a single liquidation (scaled by 1e8)
    pub close_factor: i128,
    /// % bonus collateral given to liquidators (scaled by 1e8)
    pub liquidation_incentive: i128,
    /// Pause switches for protocol actions
    pub pause_borrow: bool,
    pub pause_deposit: bool,
    pub pause_withdraw: bool,
    pub pause_liquidate: bool,
    /// Last time config was updated
    pub last_update: u64,
}

impl RiskConfig {
    pub fn default() -> Self {
        Self {
            close_factor: 50000000,          // 50%
            liquidation_incentive: 10000000, // 10%
            pause_borrow: false,
            pause_deposit: false,
            pause_withdraw: false,
            pause_liquidate: false,
            last_update: 0,
        }
    }
}

/// Storage helper for risk config
pub struct RiskConfigStorage;

impl RiskConfigStorage {
    fn key(env: &Env) -> Symbol {
        Symbol::new(env, "risk_config")
    }

    pub fn save(env: &Env, config: &RiskConfig) {
        env.storage().instance().set(&Self::key(env), config);
    }

    pub fn get(env: &Env) -> RiskConfig {
        env.storage().instance().get(&Self::key(env)).unwrap_or_else(RiskConfig::default)
    }
}

/// Interest rate storage helper
pub struct InterestRateStorage;

impl InterestRateStorage {
    fn config_key(env: &Env) -> Symbol {
        Symbol::new(env, "interest_config")
    }

    fn state_key(env: &Env) -> Symbol {
        Symbol::new(env, "interest_state")
    }

    pub fn save_config(env: &Env, config: &InterestRateConfig) {
        env.storage().instance().set(&Self::config_key(env), config);
    }

    pub fn get_config(env: &Env) -> InterestRateConfig {
        env.storage().instance().get(&Self::config_key(env)).unwrap_or_else(InterestRateConfig::default)
    }

    pub fn save_state(env: &Env, state: &InterestRateState) {
        env.storage().instance().set(&Self::state_key(env), state);
    }

    pub fn get_state(env: &Env) -> InterestRateState {
        env.storage().instance().get(&Self::state_key(env)).unwrap_or_else(InterestRateState::initial)
    }

    pub fn update_state(env: &Env) -> InterestRateState {
        let mut state = Self::get_state(env);
        let config = Self::get_config(env);
        
        // Simple interest rate calculation based on utilization
        if state.total_supplied > 0 {
            state.utilization_rate = (state.total_borrowed * 100000000) / state.total_supplied;
        } else {
            state.utilization_rate = 0;
        }

        // Calculate borrow rate based on utilization
        if state.utilization_rate <= config.kink_utilization {
            state.current_borrow_rate = config.base_rate + 
                (state.utilization_rate * config.multiplier) / 100000000;
        } else {
            let kink_rate = config.base_rate + 
                (config.kink_utilization * config.multiplier) / 100000000;
            let excess_utilization = state.utilization_rate - config.kink_utilization;
            state.current_borrow_rate = kink_rate + 
                (excess_utilization * config.multiplier * 2) / 100000000;
        }

        // Apply rate limits
        if state.current_borrow_rate > config.rate_ceiling {
            state.current_borrow_rate = config.rate_ceiling;
        }
        if state.current_borrow_rate < config.rate_floor {
            state.current_borrow_rate = config.rate_floor;
        }

        // Smoothing for borrow rate: new = old*(s) + current*(1-s)
        let s_bps = config.smoothing_bps;
        let old = state.smoothed_borrow_rate;
        let cur = state.current_borrow_rate;
        state.smoothed_borrow_rate = (old * s_bps + cur * (10000 - s_bps)) / 10000;

        // Calculate supply rate from smoothed borrow rate
        state.current_supply_rate = state.smoothed_borrow_rate * (100000000 - config.reserve_factor) / 100000000;

        state.last_accrual_time = env.ledger().timestamp();
        Self::save_state(env, &state);
        state
    }
}

/// Interest rate manager
pub struct InterestRateManager;

impl InterestRateManager {
    pub fn accrue_interest_for_position(
        env: &Env,
        position: &mut Position,
        borrow_rate: i128,
        supply_rate: i128,
    ) {
        let current_time = env.ledger().timestamp();
        if position.last_accrual_time == 0 {
            position.last_accrual_time = current_time;
            return;
        }

        let time_delta = current_time - position.last_accrual_time;
        if time_delta == 0 {
            return;
        }

        // Accrue borrow interest
        if position.debt > 0 {
            let interest = (position.debt * borrow_rate * time_delta as i128) / (365 * 24 * 60 * 60 * 100000000);
            position.borrow_interest += interest;
        }

        // Accrue supply interest
        if position.collateral > 0 {
            let interest = (position.collateral * supply_rate * time_delta as i128) / (365 * 24 * 60 * 60 * 100000000);
            position.supply_interest += interest;
        }

        position.last_accrual_time = current_time;
    }
}

/// State helper for managing user positions
pub struct StateHelper;

impl StateHelper {
    fn position_key(env: &Env, _user: &Address) -> Symbol {
        Symbol::new(env, &format!("position_{}", "user"))
    }

    pub fn save_position(env: &Env, position: &Position) {
        let key = Self::position_key(env, &position.user);
        env.storage().instance().set(&key, position);
    }

    pub fn get_position(env: &Env, user: &Address) -> Option<Position> {
        let key = Self::position_key(env, user);
        env.storage().instance().get::<Symbol, Position>(&key)
    }
}

/// Protocol configuration
pub struct ProtocolConfig;

impl ProtocolConfig {
    fn admin_key(env: &Env) -> Symbol {
        Symbol::new(env, "admin")
    }

    fn oracle_key(env: &Env) -> Symbol {
        Symbol::new(env, "oracle")
    }

    fn min_collateral_ratio_key(env: &Env) -> Symbol {
        Symbol::new(env, "min_ratio")
    }

    fn flash_fee_bps_key(env: &Env) -> Symbol {
        Symbol::new(env, "flash_fee_bps")
    }

    pub fn set_admin(env: &Env, admin: &Address) {
        env.storage().instance().set(&Self::admin_key(env), admin);
    }

    pub fn get_admin(env: &Env) -> Option<Address> {
        env.storage().instance().get::<Symbol, Address>(&Self::admin_key(env))
    }

    pub fn require_admin(env: &Env, caller: &Address) -> Result<(), ProtocolError> {
        let admin = Self::get_admin(env).ok_or(ProtocolError::Unauthorized)?;
        if admin != *caller {
            return Err(ProtocolError::Unauthorized);
        }
        Ok(())
    }

    pub fn set_oracle(env: &Env, caller: &Address, oracle: &Address) -> Result<(), ProtocolError> {
        Self::require_admin(env, caller)?;
        env.storage().instance().set(&Self::oracle_key(env), oracle);
        Ok(())
    }

    pub fn set_min_collateral_ratio(env: &Env, caller: &Address, ratio: i128) -> Result<(), ProtocolError> {
        Self::require_admin(env, caller)?;
        if ratio <= 0 {
            return Err(ProtocolError::InvalidInput);
        }
        env.storage().instance().set(&Self::min_collateral_ratio_key(env), &ratio);
        Ok(())
    }

    pub fn get_min_collateral_ratio(env: &Env) -> i128 {
        env.storage().instance().get::<Symbol, i128>(&Self::min_collateral_ratio_key(env)).unwrap_or(150)
    }

    pub fn set_flash_loan_fee_bps(env: &Env, caller: &Address, bps: i128) -> Result<(), ProtocolError> {
        Self::require_admin(env, caller)?;
        if bps < 0 || bps > 10000 { return Err(ProtocolError::InvalidInput); }
        env.storage().instance().set(&Self::flash_fee_bps_key(env), &bps);
        Ok(())
    }

    pub fn get_flash_loan_fee_bps(env: &Env) -> i128 {
        env.storage().instance().get::<Symbol, i128>(&Self::flash_fee_bps_key(env)).unwrap_or(5) // 0.05%
    }
}

/// Protocol errors
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum ProtocolError {
    Unauthorized = 1,
    InsufficientCollateral = 2,
    InsufficientCollateralRatio = 3,
    InvalidAmount = 4,
    InvalidAddress = 5,
    PositionNotFound = 6,
    AlreadyInitialized = 7,
    NotInitialized = 8,
    InvalidInput = 9,
    NotEligibleForLiquidation = 10,
    ProtocolPaused = 11,
    AssetNotSupported = 12,
    OracleFailure = 13,
    ReentrancyDetected = 14,
    StorageError = 15,
    ConfigurationError = 16,
    NotFound = 17,
    AlreadyExists = 18,
    InvalidOperation = 19,
    RecoveryFailed = 20,
    InvalidParameters = 21,
    StorageLimitExceeded = 22,
}

/// Protocol events
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub enum ProtocolEvent {
    PositionUpdated(Address, i128, i128, i128), // user, collateral, debt, collateral_ratio
    InterestAccrued(Address, i128, i128), // user, borrow_interest, supply_interest
    LiquidationExecuted(Address, Address, i128, i128), // liquidator, user, collateral_seized, debt_repaid
    RiskParamsUpdated(i128, i128), // close_factor, liquidation_incentive
    PauseSwitchesUpdated(bool, bool, bool, bool), // pause_borrow, pause_deposit, pause_withdraw, pause_liquidate
    // Cross-asset events
    CrossDeposit(Address, Address, i128), // user, asset, amount
    CrossBorrow(Address, Address, i128), // user, asset, amount
    CrossRepay(Address, Address, i128), // user, asset, amount
    CrossWithdraw(Address, Address, i128), // user, asset, amount
    // Flash loan events
    FlashLoanInitiated(Address, Address, i128, i128), // initiator, asset, amount, fee
    FlashLoanCompleted(Address, Address, i128, i128), // initiator, asset, amount, fee
    // Dynamic collateral factor
    DynamicCFUpdated(Address, i128), // asset, new_collateral_factor
    // AMM
    AMMSwap(Address, Address, Address, i128, i128), // user, asset_in, asset_out, amount_in, amount_out
    AMMLiquidityAdded(Address, Address, Address, i128, i128), // user, asset_a, asset_b, amt_a, amt_b
    AMMLiquidityRemoved(Address, Address, i128), // user, pool, lp_amount
    // Risk scoring
    RiskParamsSet(i128, i128, i128, i128), // base_limit, factor, min_rate_bps, max_rate_bps
    UserRiskUpdated(Address, i128, i128), // user, score, credit_limit_value
    // Liquidation advanced
    AuctionStarted(Address, Address, i128), // user, asset, debt_portion
    AuctionBidPlaced(Address, Address, i128), // bidder, user, bid_amount
    AuctionSettled(Address, Address, i128, i128), // winner, user, seized_collateral, repaid_debt
    // Risk monitoring
    RiskAlert(Address, i128), // user, risk_score
    // Performance & Ops
    PerfMetric(Symbol, i128), // metric_name, value
    CacheUpdated(Symbol, Symbol), // cache_key, op (set/evict)
    // Compliance
    ComplianceKycUpdated(Address, bool),
    ComplianceAlert(Address, Symbol),
    // Market making
    MMParamsUpdated(i128, i128), // spread_bps, inventory_cap
    MMIncentiveAccrued(Address, i128), // user, amount
    // Integration/API
    WebhookRegistered(Address, Symbol), // target, topic
    // Security
    BugReportLogged(Address, Symbol), // reporter, code
    AuditTrail(Symbol, Symbol), // action, ref
    // Fees
    FeesUpdated(i128, i128), // base_bps, tier1_bps
    // Insurance
    InsuranceParamsUpdated(i128, i128), // premium_bps, coverage_cap
    CircuitBreaker(bool),
    ClaimFiled(Address, i128, Symbol), // user, amount, reason
    // Bridge
    BridgeRegistered(String, Address, i128), // network_id, bridge, fee_bps
    BridgeFeeUpdated(String, i128),          // network_id, fee_bps
    AssetBridgedIn(Address, String, Address, i128, i128),  // user, network_id, asset, amount, fee
    AssetBridgedOut(Address, String, Address, i128, i128), // user, network_id, asset, amount, fee
    // Monitoring
    HealthReported(String),
    PerformanceReported(i128),
    SecurityIncident(String),
    IntegrationRegistered(String, Address),
    IntegrationCalled(String, Symbol),
    // Analytics
    AnalyticsUpdated(Address, String, i128, u64), // user, activity_type, amount, timestamp
}

impl ProtocolEvent {
    pub fn emit(&self, env: &Env) {
        match self {
            ProtocolEvent::PositionUpdated(user, collateral, debt, collateral_ratio) => {
                env.events().publish(
                    (Symbol::new(env, "position_updated"), Symbol::new(env, "user")),
                    (
                        Symbol::new(env, "user"), user.clone(),
                        Symbol::new(env, "collateral"), *collateral,
                        Symbol::new(env, "debt"), *debt,
                        Symbol::new(env, "collateral_ratio"), *collateral_ratio,
                    )
                );
            }
            ProtocolEvent::InterestAccrued(user, borrow_interest, supply_interest) => {
                env.events().publish(
                    (Symbol::new(env, "interest_accrued"), Symbol::new(env, "user")),
                    (
                        Symbol::new(env, "user"), user.clone(),
                        Symbol::new(env, "borrow_interest"), *borrow_interest,
                        Symbol::new(env, "supply_interest"), *supply_interest,
                    )
                );
            }
            ProtocolEvent::LiquidationExecuted(liquidator, user, collateral_seized, debt_repaid) => {
                env.events().publish(
                    (Symbol::new(env, "liquidation_executed"), Symbol::new(env, "liquidator")),
                    (
                        Symbol::new(env, "liquidator"), liquidator.clone(),
                        Symbol::new(env, "user"), user.clone(),
                        Symbol::new(env, "collateral_seized"), *collateral_seized,
                        Symbol::new(env, "debt_repaid"), *debt_repaid,
                    )
                );
            }
            ProtocolEvent::RiskParamsUpdated(close_factor, liquidation_incentive) => {
                env.events().publish(
                    (Symbol::new(env, "risk_params_updated"), Symbol::new(env, "close_factor")),
                    (
                        Symbol::new(env, "close_factor"), *close_factor,
                        Symbol::new(env, "liquidation_incentive"), *liquidation_incentive,
                    )
                );
            }
            ProtocolEvent::PauseSwitchesUpdated(pause_borrow, pause_deposit, pause_withdraw, pause_liquidate) => {
                env.events().publish(
                    (Symbol::new(env, "pause_switches_updated"), Symbol::new(env, "pause_borrow")),
                    (
                        Symbol::new(env, "pause_borrow"), *pause_borrow,
                        Symbol::new(env, "pause_deposit"), *pause_deposit,
                        Symbol::new(env, "pause_withdraw"), *pause_withdraw,
                        Symbol::new(env, "pause_liquidate"), *pause_liquidate,
                    )
                );
            }
            ProtocolEvent::CrossDeposit(user, asset, amount) => {
                env.events().publish(
                    (Symbol::new(env, "cross_deposit"), Symbol::new(env, "user")),
                    (
                        Symbol::new(env, "user"), user.clone(),
                        Symbol::new(env, "asset"), asset.clone(),
                        Symbol::new(env, "amount"), *amount,
                    )
                );
            }
            ProtocolEvent::CrossBorrow(user, asset, amount) => {
                env.events().publish(
                    (Symbol::new(env, "cross_borrow"), Symbol::new(env, "user")),
                    (
                        Symbol::new(env, "user"), user.clone(),
                        Symbol::new(env, "asset"), asset.clone(),
                        Symbol::new(env, "amount"), *amount,
                    )
                );
            }
            ProtocolEvent::CrossRepay(user, asset, amount) => {
                env.events().publish(
                    (Symbol::new(env, "cross_repay"), Symbol::new(env, "user")),
                    (
                        Symbol::new(env, "user"), user.clone(),
                        Symbol::new(env, "asset"), asset.clone(),
                        Symbol::new(env, "amount"), *amount,
                    )
                );
            }
            ProtocolEvent::CrossWithdraw(user, asset, amount) => {
                env.events().publish(
                    (Symbol::new(env, "cross_withdraw"), Symbol::new(env, "user")),
                    (
                        Symbol::new(env, "user"), user.clone(),
                        Symbol::new(env, "asset"), asset.clone(),
                        Symbol::new(env, "amount"), *amount,
                    )
                );
            }
            ProtocolEvent::FlashLoanInitiated(initiator, asset, amount, fee) => {
                env.events().publish(
                    (Symbol::new(env, "flash_loan_initiated"), Symbol::new(env, "user")),
                    (
                        Symbol::new(env, "user"), initiator.clone(),
                        Symbol::new(env, "asset"), asset.clone(),
                        Symbol::new(env, "amount"), *amount,
                        Symbol::new(env, "fee"), *fee,
                    )
                );
            }
            ProtocolEvent::FlashLoanCompleted(initiator, asset, amount, fee) => {
                env.events().publish(
                    (Symbol::new(env, "flash_loan_completed"), Symbol::new(env, "user")),
                    (
                        Symbol::new(env, "user"), initiator.clone(),
                        Symbol::new(env, "asset"), asset.clone(),
                        Symbol::new(env, "amount"), *amount,
                        Symbol::new(env, "fee"), *fee,
                    )
                );
            }
            ProtocolEvent::DynamicCFUpdated(asset, new_cf) => {
                env.events().publish(
                    (Symbol::new(env, "dynamic_cf_updated"), Symbol::new(env, "asset")),
                    (
                        Symbol::new(env, "asset"), asset.clone(),
                        Symbol::new(env, "new_cf"), *new_cf,
                    )
                );
            }
            ProtocolEvent::AMMSwap(user, asset_in, asset_out, amount_in, amount_out) => {
                env.events().publish(
                    (Symbol::new(env, "amm_swap"), Symbol::new(env, "user")),
                    (
                        Symbol::new(env, "user"), user.clone(),
                        Symbol::new(env, "asset_in"), asset_in.clone(),
                        Symbol::new(env, "asset_out"), asset_out.clone(),
                        Symbol::new(env, "amount_in"), *amount_in,
                        Symbol::new(env, "amount_out"), *amount_out,
                    )
                );
            }
            ProtocolEvent::AMMLiquidityAdded(user, asset_a, asset_b, amt_a, amt_b) => {
                env.events().publish(
                    (Symbol::new(env, "amm_liquidity_added"), Symbol::new(env, "user")),
                    (
                        Symbol::new(env, "user"), user.clone(),
                        Symbol::new(env, "asset_a"), asset_a.clone(),
                        Symbol::new(env, "asset_b"), asset_b.clone(),
                        Symbol::new(env, "amount_a"), *amt_a,
                        Symbol::new(env, "amount_b"), *amt_b,
                    )
                );
            }
            ProtocolEvent::AMMLiquidityRemoved(user, pool, lp_amount) => {
                env.events().publish(
                    (Symbol::new(env, "amm_liquidity_removed"), Symbol::new(env, "user")),
                    (
                        Symbol::new(env, "user"), user.clone(),
                        Symbol::new(env, "pool"), pool.clone(),
                        Symbol::new(env, "lp_amount"), *lp_amount,
                    )
                );
            }
            ProtocolEvent::RiskParamsSet(base_limit, factor, min_rate_bps, max_rate_bps) => {
                env.events().publish(
                    (Symbol::new(env, "risk_params_set"), Symbol::new(env, "base_limit")),
                    (
                        Symbol::new(env, "base_limit"), *base_limit,
                        Symbol::new(env, "factor"), *factor,
                        Symbol::new(env, "min_rate_bps"), *min_rate_bps,
                        Symbol::new(env, "max_rate_bps"), *max_rate_bps,
                    )
                );
            }
            ProtocolEvent::UserRiskUpdated(user, score, limit) => {
                env.events().publish(
                    (Symbol::new(env, "user_risk_updated"), Symbol::new(env, "user")),
                    (
                        Symbol::new(env, "user"), user.clone(),
                        Symbol::new(env, "score"), *score,
                        Symbol::new(env, "credit_limit"), *limit,
                    )
                );
            }
            ProtocolEvent::AuctionStarted(user, asset, debt_portion) => {
                env.events().publish(
                    (Symbol::new(env, "auction_started"), Symbol::new(env, "user")),
                    (
                        Symbol::new(env, "user"), user.clone(),
                        Symbol::new(env, "asset"), asset.clone(),
                        Symbol::new(env, "debt_portion"), *debt_portion,
                    )
                );
            }
            ProtocolEvent::AuctionBidPlaced(bidder, user, bid_amount) => {
                env.events().publish(
                    (Symbol::new(env, "auction_bid"), Symbol::new(env, "bidder")),
                    (
                        Symbol::new(env, "bidder"), bidder.clone(),
                        Symbol::new(env, "user"), user.clone(),
                        Symbol::new(env, "bid_amount"), *bid_amount,
                    )
                );
            }
            ProtocolEvent::AuctionSettled(winner, user, seized, repaid) => {
                env.events().publish(
                    (Symbol::new(env, "auction_settled"), Symbol::new(env, "winner")),
                    (
                        Symbol::new(env, "winner"), winner.clone(),
                        Symbol::new(env, "user"), user.clone(),
                        Symbol::new(env, "seized_collateral"), *seized,
                        Symbol::new(env, "repaid_debt"), *repaid,
                    )
                );
            }
            ProtocolEvent::RiskAlert(user, score) => {
                env.events().publish(
                    (Symbol::new(env, "risk_alert"), Symbol::new(env, "user")),
                    (
                        Symbol::new(env, "user"), user.clone(),
                        Symbol::new(env, "score"), *score,
                    )
                );
            }
            ProtocolEvent::BridgeRegistered(network_id, bridge, fee_bps) => {
                env.events().publish(
                    (Symbol::new(env, "bridge_registered"), Symbol::new(env, "network")),
                    (
                        Symbol::new(env, "network"), network_id.clone(),
                        Symbol::new(env, "bridge"), bridge.clone(),
                        Symbol::new(env, "fee_bps"), *fee_bps,
                    )
                );
            }
            ProtocolEvent::BridgeFeeUpdated(network_id, fee_bps) => {
                env.events().publish(
                    (Symbol::new(env, "bridge_fee_updated"), Symbol::new(env, "network")),
                    (
                        Symbol::new(env, "network"), network_id.clone(),
                        Symbol::new(env, "fee_bps"), *fee_bps,
                    )
                );
            }
            ProtocolEvent::AssetBridgedIn(user, network_id, asset, amount, fee) => {
                env.events().publish(
                    (Symbol::new(env, "asset_bridged_in"), Symbol::new(env, "user")),
                    (
                        Symbol::new(env, "user"), user.clone(),
                        Symbol::new(env, "network"), network_id.clone(),
                        Symbol::new(env, "asset"), asset.clone(),
                        Symbol::new(env, "amount"), *amount,
                        Symbol::new(env, "fee"), *fee,
                    )
                );
            }
            ProtocolEvent::AssetBridgedOut(user, network_id, asset, amount, fee) => {
                env.events().publish(
                    (Symbol::new(env, "asset_bridged_out"), Symbol::new(env, "user")),
                    (
                        Symbol::new(env, "user"), user.clone(),
                        Symbol::new(env, "network"), network_id.clone(),
                        Symbol::new(env, "asset"), asset.clone(),
                        Symbol::new(env, "amount"), *amount,
                        Symbol::new(env, "fee"), *fee,
                    )
                );
            }
            ProtocolEvent::HealthReported(msg) => {
                env.events().publish(
                    (Symbol::new(env, "health_report"), Symbol::new(env, "msg")),
                    (Symbol::new(env, "msg"), msg.clone())
                );
            }
            ProtocolEvent::PerformanceReported(gas) => {
                env.events().publish(
                    (Symbol::new(env, "performance_report"), Symbol::new(env, "gas")),
                    (Symbol::new(env, "gas"), *gas)
                );
            }
            ProtocolEvent::SecurityIncident(msg) => {
                env.events().publish(
                    (Symbol::new(env, "security_incident"), Symbol::new(env, "msg")),
                    (Symbol::new(env, "msg"), msg.clone())
                );
            }
            ProtocolEvent::IntegrationRegistered(name, addr) => {
                env.events().publish(
                    (Symbol::new(env, "integration_registered"), Symbol::new(env, "name")),
                    (Symbol::new(env, "name"), name.clone(), Symbol::new(env, "address"), addr.clone())
                );
            }
            ProtocolEvent::IntegrationCalled(name, method) => {
                env.events().publish(
                    (Symbol::new(env, "integration_called"), Symbol::new(env, "name")),
                    (Symbol::new(env, "name"), name.clone(), Symbol::new(env, "method"), method.clone())
                );
            }
            ProtocolEvent::AnalyticsUpdated(user, activity_type, amount, timestamp) => {
                env.events().publish(
                    (Symbol::new(env, "analytics_updated"), Symbol::new(env, "user")),
                    (
                        Symbol::new(env, "user"), user.clone(),
                        Symbol::new(env, "activity_type"), activity_type.clone(),
                        Symbol::new(env, "amount"), *amount,
                        Symbol::new(env, "timestamp"), *timestamp,
                    )
                );
            }
            // Add placeholder implementations for missing event variants
            _ => {
                // For now, we'll skip emitting these events to avoid compilation errors
                // In a full implementation, these would have proper event emission logic
            }
        }
    }
}

/// Analytics helper function
pub fn analytics_record_action(env: &Env, user: &Address, action: &str, amount: i128) {
    // Simple analytics recording - can be enhanced later
    let timestamp = env.ledger().timestamp();
    // For now, just emit a simple event
    ProtocolEvent::InterestAccrued(user.clone(), amount, timestamp as i128).emit(env);
}

/// Helper function to ensure amount is positive
fn ensure_amount_positive(amount: i128) -> Result<(), ProtocolError> {
    if amount <= 0 { return Err(ProtocolError::InvalidAmount); }
    Ok(())
}

/// Core protocol functions
pub fn deposit_collateral(env: Env, depositor: String, amount: i128) -> Result<(), ProtocolError> {
    deposit::DepositModule::deposit_collateral(&env, &depositor, amount)
}

pub fn borrow(env: Env, borrower: String, amount: i128) -> Result<(), ProtocolError> {
    borrow::BorrowModule::borrow(&env, &borrower, amount)
}

pub fn repay(env: Env, repayer: String, amount: i128) -> Result<(), ProtocolError> {
    repay::RepayModule::repay(&env, &repayer, amount)
}

pub fn withdraw(env: Env, withdrawer: String, amount: i128) -> Result<(), ProtocolError> {
    withdraw::WithdrawModule::withdraw(&env, &withdrawer, amount)
}

pub fn liquidate(env: Env, liquidator: String, user: String, amount: i128) -> Result<(), ProtocolError> {
    liquidate::LiquidationModule::liquidate(&env, &liquidator, &user, amount)?;
    Ok(())
}

pub fn get_position(env: Env, user: String) -> Result<(i128, i128, i128), ProtocolError> {
    let user_addr = Address::from_string(&user);
    match StateHelper::get_position(&env, &user_addr) {
        Some(position) => {
            let collateral_ratio = if position.debt > 0 {
                (position.collateral * 100) / position.debt
            } else {
                0
            };
            Ok((position.collateral, position.debt, collateral_ratio))
        }
        None => Err(ProtocolError::PositionNotFound)
    }
}

pub fn set_risk_params(env: Env, caller: String, close_factor: i128, liquidation_incentive: i128) -> Result<(), ProtocolError> {
    let caller_addr = Address::from_string(&caller);
    ProtocolConfig::require_admin(&env, &caller_addr)?;
    
    let mut config = RiskConfigStorage::get(&env);
    config.close_factor = close_factor;
    config.liquidation_incentive = liquidation_incentive;
    config.last_update = env.ledger().timestamp();
    RiskConfigStorage::save(&env, &config);
    
    ProtocolEvent::RiskParamsUpdated(close_factor, liquidation_incentive).emit(&env);
    Ok(())
}

pub fn set_pause_switches(env: Env, caller: String, pause_borrow: bool, pause_deposit: bool, pause_withdraw: bool, pause_liquidate: bool) -> Result<(), ProtocolError> {
    let caller_addr = Address::from_string(&caller);
    ProtocolConfig::require_admin(&env, &caller_addr)?;
    
    let mut config = RiskConfigStorage::get(&env);
    config.pause_borrow = pause_borrow;
    config.pause_deposit = pause_deposit;
    config.pause_withdraw = pause_withdraw;
    config.pause_liquidate = pause_liquidate;
    config.last_update = env.ledger().timestamp();
    RiskConfigStorage::save(&env, &config);
    
    ProtocolEvent::PauseSwitchesUpdated(pause_borrow, pause_deposit, pause_withdraw, pause_liquidate).emit(&env);
    Ok(())
}

pub fn get_protocol_params(env: Env) -> Result<(i128, i128, i128, i128, i128, i128), ProtocolError> {
    let config = InterestRateStorage::get_config(&env);
    let risk_config = RiskConfigStorage::get(&env);
    
    Ok((
        config.base_rate,         // 2000000 (2%)
        config.kink_utilization,  // 80000000 (80%)
        config.multiplier,        // 10000000 (10x)
        config.reserve_factor,    // 10000000 (10%)
        risk_config.close_factor, // 50000000 (50%)
        risk_config.liquidation_incentive // 10000000 (10%)
    ))
}

pub fn get_risk_config(env: Env) -> Result<(i128, i128, bool, bool, bool, bool), ProtocolError> {
    let config = RiskConfigStorage::get(&env);
    Ok((
        config.close_factor,
        config.liquidation_incentive,
        config.pause_borrow,
        config.pause_deposit,
        config.pause_withdraw,
        config.pause_liquidate
    ))
}

pub fn get_system_stats(env: Env) -> Result<(i128, i128, i128, i128), ProtocolError> {
    let state = InterestRateStorage::get_state(&env);
    
    Ok((
        state.total_supplied,
        state.total_borrowed,
        state.utilization_rate,
        0 // active_users - simplified for now
    ))
}

#[contractimpl]
impl Contract {
    /// Initializes the contract and sets the admin address
    pub fn initialize(env: Env, admin: String) -> Result<(), ProtocolError> {
        let admin_addr = Address::from_string(&admin);
        if env.storage().instance().has(&ProtocolConfig::admin_key(&env)) {
            return Err(ProtocolError::AlreadyInitialized);
        }
        ProtocolConfig::set_admin(&env, &admin_addr);

        // Initialize interest rate system with default configuration
        let config = InterestRateConfig::default();
        InterestRateStorage::save_config(&env, &config);

        let state = InterestRateState::initial();
        InterestRateStorage::save_state(&env, &state);

        // Initialize risk management system with default configuration
        let risk_config = RiskConfig::default();
        RiskConfigStorage::save(&env, &risk_config);

        Ok(())
    }

    /// Set the minimum collateral ratio (admin only)
    pub fn set_min_collateral_ratio(
        env: Env,
        caller: String,
        ratio: i128,
    ) -> Result<(), ProtocolError> {
        let caller_addr = Address::from_string(&caller);
        ProtocolConfig::set_min_collateral_ratio(&env, &caller_addr, ratio)?;
        Ok(())
    }

    /// Deposit collateral into the protocol
    pub fn deposit_collateral(env: Env, depositor: String, amount: i128) -> Result<(), ProtocolError> {
        deposit_collateral(env, depositor, amount)
    }

    /// Borrow assets from the protocol
    pub fn borrow(env: Env, borrower: String, amount: i128) -> Result<(), ProtocolError> {
        borrow(env, borrower, amount)
    }

    /// Repay borrowed assets
    pub fn repay(env: Env, repayer: String, amount: i128) -> Result<(), ProtocolError> {
        repay(env, repayer, amount)
    }

    /// Withdraw collateral from the protocol
    pub fn withdraw(env: Env, withdrawer: String, amount: i128) -> Result<(), ProtocolError> {
        withdraw(env, withdrawer, amount)
    }

    /// Liquidate an undercollateralized position
    pub fn liquidate(env: Env, liquidator: String, user: String, amount: i128) -> Result<(), ProtocolError> {
        liquidate(env, liquidator, user, amount)
    }

    /// Get user position
    pub fn get_position(env: Env, user: String) -> Result<(i128, i128, i128), ProtocolError> {
        get_position(env, user)
    }

    /// Set risk parameters (admin only)
    pub fn set_risk_params(env: Env, caller: String, close_factor: i128, liquidation_incentive: i128) -> Result<(), ProtocolError> {
        set_risk_params(env, caller, close_factor, liquidation_incentive)
    }

    /// Set pause switches (admin only)
    pub fn set_pause_switches(env: Env, caller: String, pause_borrow: bool, pause_deposit: bool, pause_withdraw: bool, pause_liquidate: bool) -> Result<(), ProtocolError> {
        set_pause_switches(env, caller, pause_borrow, pause_deposit, pause_withdraw, pause_liquidate)
    }

    /// Get protocol parameters
    pub fn get_protocol_params(env: Env) -> Result<(i128, i128, i128, i128, i128, i128), ProtocolError> {
        get_protocol_params(env)
    }

    /// Get risk configuration
    pub fn get_risk_config(env: Env) -> Result<(i128, i128, bool, bool, bool, bool), ProtocolError> {
        get_risk_config(env)
    }

    /// Get system stats
    pub fn get_system_stats(env: Env) -> Result<(i128, i128, i128, i128), ProtocolError> {
        get_system_stats(env)
    }

    // Analytics and Reporting Functions
    pub fn get_protocol_report(env: Env) -> Result<analytics::ProtocolReport, ProtocolError> {
        analytics::AnalyticsModule::get_protocol_report(&env)
    }

    pub fn get_user_report(env: Env, user: String) -> Result<analytics::UserReport, ProtocolError> {
        let user_addr = Address::from_string(&user);
        analytics::AnalyticsModule::get_user_report(&env, &user_addr)
    }

    pub fn get_asset_report(env: Env, asset: Address) -> Result<analytics::AssetReport, ProtocolError> {
        analytics::AnalyticsModule::get_asset_report(&env, &asset)
    }

    pub fn calculate_risk_analytics(env: Env) -> Result<analytics::RiskAnalytics, ProtocolError> {
        analytics::AnalyticsModule::calculate_risk_analytics(&env)
    }

    pub fn update_performance_metrics(env: Env, processing_time: i128, success: bool) -> Result<(), ProtocolError> {
        analytics::AnalyticsModule::update_performance_metrics(&env, processing_time, success)
    }

    pub fn record_activity(env: Env, user: String, activity_type: String, amount: i128, asset: Option<Address>) -> Result<(), ProtocolError> {
        let user_addr = Address::from_string(&user);
        // Convert String to &str for the analytics module
        let activity_type_str = activity_type.to_string();
        analytics::AnalyticsModule::record_activity(&env, &user_addr, &activity_type_str, amount, asset)
    }
}

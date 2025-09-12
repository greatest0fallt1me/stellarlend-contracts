//! StellarLend Soroban Smart Contract
//
//! This contract provides the foundation for the StellarLend DeFi Lending & Borrowing Protocol.
//! Core features will be implemented incrementally in separate modules.

#![no_std]
extern crate alloc;

use alloc::format;
use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, vec, Address, Env,
    IntoVal, Map, String, Symbol, Vec,
};
mod oracle;
use oracle::{Oracle, OracleSource, OracleStorage};

// Global allocator for Soroban contracts
#[global_allocator]
static ALLOC: wee_alloc::WeeAlloc = wee_alloc::WeeAlloc::INIT;

#[cfg(test)]
mod test;

// Module placeholders for future expansion
// mod deposit;
// mod borrow;
// mod repay;
// mod withdraw;
// mod liquidate;

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

/// Parameters for a specific asset supported by cross-asset functionality
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct AssetParams {
    /// Collateral factor (scaled by 1e8). 75% => 75000000
    pub collateral_factor: i128,
    /// Borrowing enabled for this asset
    pub borrow_enabled: bool,
    /// Deposits enabled for this asset
    pub deposit_enabled: bool,
    /// Cross-asset features enabled for this asset
    pub cross_enabled: bool,
}

impl AssetParams {
    pub fn default() -> Self {
        Self {
            collateral_factor: 75000000, // 75%
            borrow_enabled: true,
            deposit_enabled: true,
            cross_enabled: true,
        }
    }
}

/// Dynamic CF parameters per asset
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct DynamicCFParams {
    pub min_cf: i128,         // 0..=1e8
    pub max_cf: i128,         // 0..=1e8
    pub sensitivity_bps: i128, // how much to reduce per 1% vol (bps)
    pub max_step_bps: i128,    // max change per update (bps)
}

impl DynamicCFParams {
    pub fn default() -> Self {
        Self { min_cf: 50000000, max_cf: 90000000, sensitivity_bps: 100, max_step_bps: 200 } // defaults
    }
}

/// Market state tracking for dynamic CF
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct MarketState {
    pub last_price: i128,    // 1e8
    pub vol_index_bps: i128, // simple volatility index in bps
}

impl MarketState {
    pub fn initial() -> Self { Self { last_price: 0, vol_index_bps: 0 } }
}

/// Global risk parameters
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct RiskParamsGlobal {
    pub base_limit_value: i128,    // in 1e8 value units
    pub score_to_limit_factor: i128, // multiplier per score unit to increase limit
    pub min_rate_adj_bps: i128,
    pub max_rate_adj_bps: i128,
}

impl RiskParamsGlobal {
    pub fn default() -> Self {
        Self { base_limit_value: 0, score_to_limit_factor: 0, min_rate_adj_bps: 0, max_rate_adj_bps: 0 }
    }
}

/// Per-user risk state
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct UserRiskState {
    pub user: Address,
    pub score: i128,              // 0..=1000
    pub credit_limit_value: i128, // 1e8 value units
    pub tx_count: i128,
    pub last_update: u64,
}

impl UserRiskState {
    pub fn new(user: Address) -> Self {
        Self { user, score: 0, credit_limit_value: 0, tx_count: 0, last_update: 0 }
    }
}

/// Cross-asset position with per-asset collateral and debt
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct CrossPosition {
    pub user: Address,
    /// Collateral balances by asset
    pub collateral: Map<Address, i128>,
    /// Debt balances by asset
    pub debt: Map<Address, i128>,
    /// Last accrual time for interest-like accounting (placeholder)
    pub last_accrual_time: u64,
}

impl CrossPosition {
    pub fn new(env: &Env, user: Address) -> Self {
        Self {
            user,
            collateral: Map::new(env),
            debt: Map::new(env),
            last_accrual_time: 0,
        }
    }
}

/// Pair key for AMM registry
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct PairKey {
    pub a: Address,
    pub b: Address,
}

impl PairKey {
    pub fn ordered(a: Address, b: Address) -> Self {
        // Simple ordering by address to ensure deterministic mapping
        if a.to_string() <= b.to_string() { Self { a, b } } else { Self { a: b, b: a } }
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

        // Calculate supply rate (borrow rate minus reserve factor)
        state.current_supply_rate = state.current_borrow_rate * 
            (100000000 - config.reserve_factor) / 100000000;

        state.last_accrual_time = env.ledger().timestamp();
        Self::save_state(env, &state);
        state
    }
}

/// Storage for asset registry and oracle prices (cross-asset)
pub struct AssetRegistryStorage;

impl AssetRegistryStorage {
    fn params_key(env: &Env) -> Symbol { Symbol::new(env, "asset_params_map") }
    fn prices_key(env: &Env) -> Symbol { Symbol::new(env, "asset_prices_map") }
    fn cross_positions_key(env: &Env) -> Symbol { Symbol::new(env, "cross_positions_map") }
    fn dyn_params_key(env: &Env) -> Symbol { Symbol::new(env, "dyn_cf_params_map") }
    fn market_state_key(env: &Env) -> Symbol { Symbol::new(env, "market_state_map") }
    fn amm_registry_key(env: &Env) -> Symbol { Symbol::new(env, "amm_registry_map") }
    fn user_risk_key(env: &Env) -> Symbol { Symbol::new(env, "user_risk_map") }
    fn risk_params_key(env: &Env) -> Symbol { Symbol::new(env, "risk_params") }

    pub fn get_params_map(env: &Env) -> Map<Address, AssetParams> {
        env.storage().instance().get(&Self::params_key(env)).unwrap_or_else(|| Map::new(env))
    }

    pub fn put_params_map(env: &Env, map: &Map<Address, AssetParams>) {
        env.storage().instance().set(&Self::params_key(env), map);
    }

    pub fn get_prices_map(env: &Env) -> Map<Address, i128> {
        env.storage().instance().get(&Self::prices_key(env)).unwrap_or_else(|| Map::new(env))
    }

    pub fn put_prices_map(env: &Env, map: &Map<Address, i128>) {
        env.storage().instance().set(&Self::prices_key(env), map);
    }

    pub fn get_cross_positions(env: &Env) -> Map<Address, CrossPosition> {
        env.storage().instance().get(&Self::cross_positions_key(env)).unwrap_or_else(|| Map::new(env))
    }

    pub fn put_cross_positions(env: &Env, map: &Map<Address, CrossPosition>) {
        env.storage().instance().set(&Self::cross_positions_key(env), map);
    }

    pub fn get_dyn_params(env: &Env) -> Map<Address, DynamicCFParams> {
        env.storage().instance().get(&Self::dyn_params_key(env)).unwrap_or_else(|| Map::new(env))
    }

    pub fn put_dyn_params(env: &Env, map: &Map<Address, DynamicCFParams>) {
        env.storage().instance().set(&Self::dyn_params_key(env), map);
    }

    pub fn get_market_state(env: &Env) -> Map<Address, MarketState> {
        env.storage().instance().get(&Self::market_state_key(env)).unwrap_or_else(|| Map::new(env))
    }

    pub fn put_market_state(env: &Env, map: &Map<Address, MarketState>) {
        env.storage().instance().set(&Self::market_state_key(env), map);
    }

    pub fn get_amm_registry(env: &Env) -> Map<PairKey, Address> {
        env.storage().instance().get(&Self::amm_registry_key(env)).unwrap_or_else(|| Map::new(env))
    }

    pub fn put_amm_registry(env: &Env, map: &Map<PairKey, Address>) {
        env.storage().instance().set(&Self::amm_registry_key(env), map);
    }

    pub fn get_user_risk(env: &Env) -> Map<Address, UserRiskState> {
        env.storage().instance().get(&Self::user_risk_key(env)).unwrap_or_else(|| Map::new(env))
    }

    pub fn put_user_risk(env: &Env, map: &Map<Address, UserRiskState>) {
        env.storage().instance().set(&Self::user_risk_key(env), map);
    }

    pub fn save_risk_params(env: &Env, params: &RiskParamsGlobal) {
        env.storage().instance().set(&Self::risk_params_key(env), params);
    }

    pub fn get_risk_params(env: &Env) -> RiskParamsGlobal {
        env.storage().instance().get(&Self::risk_params_key(env)).unwrap_or_else(RiskParamsGlobal::default)
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

/// Helper for cross-asset positions
pub struct CrossStateHelper;

impl CrossStateHelper {
    pub fn get_or_init_position(env: &Env, user: &Address) -> CrossPosition {
        let mut positions = AssetRegistryStorage::get_cross_positions(env);
        if let Some(pos) = positions.get(user.clone()) { return pos; }
        let pos = CrossPosition::new(env, user.clone());
        positions.set(user.clone(), pos.clone());
        AssetRegistryStorage::put_cross_positions(env, &positions);
        pos
    }

    pub fn save_position(env: &Env, position: &CrossPosition) {
        let mut positions = AssetRegistryStorage::get_cross_positions(env);
        positions.set(position.user.clone(), position.clone());
        AssetRegistryStorage::put_cross_positions(env, &positions);
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
    GuardianNotFound = 31,
    GuardianAlreadyExists = 32,
    RecoveryRequestNotFound = 33,
    RecoveryRequestAlreadyExists = 34,
    RecoveryNotReady = 35,
    InvalidGuardianAddress = 36,
    InvalidRecoveryAddress = 37,
    MultiSigProposalNotFound = 38,
    MultiSigNotReady = 39,
    // Cross-asset specific
    CrossAssetDisabled = 40,
    PriceNotAvailable = 41,
    CollateralFactorInvalid = 42,
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
    // Cross-asset
    CrossDeposit(Address, Address, i128), // user, asset, amount
    CrossBorrow(Address, Address, i128), // user, asset, amount
    CrossRepay(Address, Address, i128), // user, asset, amount
    CrossWithdraw(Address, Address, i128), // user, asset, amount
    // Flash loan
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
        }
    }
}

/// Minimum collateral ratio required (e.g., 150%)
const MIN_COLLATERAL_RATIO: i128 = 150;

// --- Core Protocol Function Placeholders ---
/// Deposit collateral into the protocol
pub fn deposit_collateral(env: Env, depositor: String, amount: i128) -> Result<(), ProtocolError> {
    ReentrancyGuard::enter(&env)?;
    let result = (|| {
        // Input validation
        if depositor.is_empty() {
            return Err(ProtocolError::InvalidAddress);
        }
        if amount <= 0 {
            return Err(ProtocolError::InvalidAmount);
        }

        // Check if deposit is paused
        let risk_config = RiskConfigStorage::get(&env);
        if risk_config.pause_deposit {
            return Err(ProtocolError::ProtocolPaused);
        }

        let depositor_addr = Address::from_string(&depositor);
        
        // Load user position with error handling
        let mut position = match StateHelper::get_position(&env, &depositor_addr) {
            Some(pos) => pos,
            None => Position::new(depositor_addr.clone(), 0, 0),
        };

        // Accrue interest before updating position
        let state = InterestRateStorage::update_state(&env);
        InterestRateManager::accrue_interest_for_position(
            &env,
            &mut position,
            state.current_borrow_rate,
            state.current_supply_rate,
        );

        // Update position
        position.collateral += amount;
        
        // Save position
        StateHelper::save_position(&env, &position);

        // Emit event
        let collateral_ratio = if position.debt > 0 {
            (position.collateral * 100) / position.debt
        } else {
            0
        };

        ProtocolEvent::PositionUpdated(
            depositor_addr,
            position.collateral,
            position.debt,
            collateral_ratio,
        ).emit(&env);

        Ok(())
    })();
    
    ReentrancyGuard::exit(&env);
    result
}

/// Borrow assets from the protocol
pub fn borrow(env: Env, borrower: String, amount: i128) -> Result<(), ProtocolError> {
    ReentrancyGuard::enter(&env)?;
    let result = (|| {
        // Input validation
        if borrower.is_empty() {
            return Err(ProtocolError::InvalidAddress);
        }
        if amount <= 0 {
            return Err(ProtocolError::InvalidAmount);
        }

        // Check if borrow is paused
        let risk_config = RiskConfigStorage::get(&env);
        if risk_config.pause_borrow {
            return Err(ProtocolError::ProtocolPaused);
        }

        let borrower_addr = Address::from_string(&borrower);
        
        // Load user position
        let mut position = match StateHelper::get_position(&env, &borrower_addr) {
            Some(pos) => pos,
            None => return Err(ProtocolError::PositionNotFound),
        };

        // Accrue interest
        let state = InterestRateStorage::update_state(&env);
        InterestRateManager::accrue_interest_for_position(
            &env,
            &mut position,
            state.current_borrow_rate,
            state.current_supply_rate,
        );

        // Check collateral ratio
        let min_ratio = ProtocolConfig::get_min_collateral_ratio(&env);
        let new_debt = position.debt + amount;
        let collateral_ratio = if new_debt > 0 {
            (position.collateral * 100) / new_debt
        } else {
            0
        };

        if collateral_ratio < min_ratio {
            return Err(ProtocolError::InsufficientCollateralRatio);
        }

        // Update position
        position.debt = new_debt;
        StateHelper::save_position(&env, &position);

        // Emit event
        ProtocolEvent::PositionUpdated(
            borrower_addr,
            position.collateral,
            position.debt,
            collateral_ratio,
        ).emit(&env);

        Ok(())
    })();
    
    ReentrancyGuard::exit(&env);
    result
}

/// Repay borrowed assets
pub fn repay(env: Env, repayer: String, amount: i128) -> Result<(), ProtocolError> {
    ReentrancyGuard::enter(&env)?;
    let result = (|| {
        // Input validation
        if repayer.is_empty() {
            return Err(ProtocolError::InvalidAddress);
        }
        if amount <= 0 {
            return Err(ProtocolError::InvalidAmount);
        }

        let repayer_addr = Address::from_string(&repayer);
        
        // Load user position
        let mut position = match StateHelper::get_position(&env, &repayer_addr) {
            Some(pos) => pos,
            None => return Err(ProtocolError::PositionNotFound),
        };

        // Accrue interest
        let state = InterestRateStorage::update_state(&env);
        InterestRateManager::accrue_interest_for_position(
            &env,
            &mut position,
            state.current_borrow_rate,
            state.current_supply_rate,
        );

        // Check if user has debt to repay
        if position.debt == 0 {
            return Err(ProtocolError::InvalidOperation);
        }

        // Update position
        let repay_amount = if amount > position.debt {
            position.debt
        } else {
            amount
        };
        position.debt -= repay_amount;
        StateHelper::save_position(&env, &position);

        // Emit event
        let collateral_ratio = if position.debt > 0 {
            (position.collateral * 100) / position.debt
        } else {
            0
        };

        ProtocolEvent::PositionUpdated(
            repayer_addr,
            position.collateral,
            position.debt,
            collateral_ratio,
        ).emit(&env);

        Ok(())
    })();
    
    ReentrancyGuard::exit(&env);
    result
}

/// Withdraw collateral from the protocol
pub fn withdraw(env: Env, withdrawer: String, amount: i128) -> Result<(), ProtocolError> {
    ReentrancyGuard::enter(&env)?;
    let result = (|| {
        // Input validation
        if withdrawer.is_empty() {
            return Err(ProtocolError::InvalidAddress);
        }
        if amount <= 0 {
            return Err(ProtocolError::InvalidAmount);
        }

        // Check if withdraw is paused
        let risk_config = RiskConfigStorage::get(&env);
        if risk_config.pause_withdraw {
            return Err(ProtocolError::ProtocolPaused);
        }

        let withdrawer_addr = Address::from_string(&withdrawer);
        
        // Load user position
        let mut position = match StateHelper::get_position(&env, &withdrawer_addr) {
            Some(pos) => pos,
            None => return Err(ProtocolError::PositionNotFound),
        };

        // Check if user has enough collateral
        if position.collateral < amount {
            return Err(ProtocolError::InsufficientCollateral);
        }

        // Accrue interest
        let state = InterestRateStorage::update_state(&env);
        InterestRateManager::accrue_interest_for_position(
            &env,
            &mut position,
            state.current_borrow_rate,
            state.current_supply_rate,
        );

        // Check collateral ratio after withdrawal (only if there's debt)
        let new_collateral = position.collateral - amount;
        let collateral_ratio = if position.debt > 0 {
            let min_ratio = ProtocolConfig::get_min_collateral_ratio(&env);
            let ratio = (new_collateral * 100) / position.debt;
            if ratio < min_ratio {
                return Err(ProtocolError::InsufficientCollateralRatio);
            }
            ratio
        } else {
            0
        };

        // Update position
        position.collateral = new_collateral;
        StateHelper::save_position(&env, &position);

        // Emit event
        ProtocolEvent::PositionUpdated(
            withdrawer_addr,
            position.collateral,
            position.debt,
            collateral_ratio,
        ).emit(&env);

        Ok(())
    })();
    
    ReentrancyGuard::exit(&env);
    result
}

/// Liquidate an undercollateralized position
pub fn liquidate(env: Env, liquidator: String, user: String, amount: i128) -> Result<(), ProtocolError> {
    ReentrancyGuard::enter(&env)?;
    let result = (|| {
        // Input validation
        if liquidator.is_empty() || user.is_empty() {
            return Err(ProtocolError::InvalidAddress);
        }
        if amount <= 0 {
            return Err(ProtocolError::InvalidAmount);
        }

        // Check if liquidation is paused
        let risk_config = RiskConfigStorage::get(&env);
        if risk_config.pause_liquidate {
            return Err(ProtocolError::ProtocolPaused);
        }

        let liquidator_addr = Address::from_string(&liquidator);
        let user_addr = Address::from_string(&user);
        
        // Load user position
        let mut position = match StateHelper::get_position(&env, &user_addr) {
            Some(pos) => pos,
            None => return Err(ProtocolError::PositionNotFound),
        };

        // Check if position is eligible for liquidation
        let min_ratio = ProtocolConfig::get_min_collateral_ratio(&env);
        let collateral_ratio = if position.debt > 0 {
            (position.collateral * 100) / position.debt
        } else {
            0
        };

        if collateral_ratio >= min_ratio {
            return Err(ProtocolError::NotEligibleForLiquidation);
        }

        // Calculate liquidation amount
        let max_liquidation = (position.debt * risk_config.close_factor) / 100000000;
        let liquidation_amount = if amount > max_liquidation {
            max_liquidation
        } else {
            amount
        };

        // Calculate collateral to seize
        let collateral_seized = (liquidation_amount * (100000000 + risk_config.liquidation_incentive)) / 100000000;

        // Update position
        position.debt -= liquidation_amount;
        position.collateral -= collateral_seized;
        StateHelper::save_position(&env, &position);

        // Emit liquidation event
        ProtocolEvent::LiquidationExecuted(
            liquidator_addr,
            user_addr,
            collateral_seized,
            liquidation_amount,
        ).emit(&env);

        Ok(())
    })();
    
    ReentrancyGuard::exit(&env);
    result
}

/// Get user position
pub fn get_position(env: Env, user: String) -> Result<(i128, i128, i128), ProtocolError> {
    if user.is_empty() {
        return Err(ProtocolError::InvalidAddress);
    }

    let user_addr = Address::from_string(&user);
    let position = match StateHelper::get_position(&env, &user_addr) {
        Some(pos) => pos,
        None => return Err(ProtocolError::PositionNotFound),
    };

    let collateral_ratio = if position.debt > 0 {
        (position.collateral * 100) / position.debt
    } else {
        0
    };

    Ok((position.collateral, position.debt, collateral_ratio))
}

/// Set risk parameters (admin only)
pub fn set_risk_params(env: Env, caller: String, close_factor: i128, liquidation_incentive: i128) -> Result<(), ProtocolError> {
    let caller_addr = Address::from_string(&caller);
    ProtocolConfig::require_admin(&env, &caller_addr)?;

    if close_factor <= 0 || liquidation_incentive <= 0 {
        return Err(ProtocolError::InvalidInput);
    }

    let mut config = RiskConfigStorage::get(&env);
    config.close_factor = close_factor;
    config.liquidation_incentive = liquidation_incentive;
    config.last_update = env.ledger().timestamp();
    RiskConfigStorage::save(&env, &config);

            ProtocolEvent::RiskParamsUpdated(
            close_factor,
            liquidation_incentive,
        ).emit(&env);

    Ok(())
}

/// Set pause switches (admin only)
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

            ProtocolEvent::PauseSwitchesUpdated(
            pause_borrow,
            pause_deposit,
            pause_withdraw,
            pause_liquidate,
        ).emit(&env);

    Ok(())
}

/// Get protocol parameters
pub fn get_protocol_params(env: Env) -> Result<(i128, i128, i128, i128, i128, i128), ProtocolError> {
    let config = InterestRateStorage::get_config(&env);
    let risk_config = RiskConfigStorage::get(&env);
    
    Ok((
        config.base_rate,
        config.kink_utilization,
        config.multiplier,
        config.reserve_factor,
        risk_config.close_factor,
        risk_config.liquidation_incentive,
    ))
}

/// Get risk configuration
pub fn get_risk_config(env: Env) -> Result<(i128, i128, bool, bool, bool, bool), ProtocolError> {
    let config = RiskConfigStorage::get(&env);
    
    Ok((
        config.close_factor,
        config.liquidation_incentive,
        config.pause_borrow,
        config.pause_deposit,
        config.pause_withdraw,
        config.pause_liquidate,
    ))
}

/// Get system stats
pub fn get_system_stats(env: Env) -> Result<(i128, i128, i128, i128), ProtocolError> {
    let state = InterestRateStorage::get_state(&env);
    
    Ok((
        state.total_supplied,
        state.total_borrowed,
        state.current_borrow_rate,
        state.current_supply_rate,
    ))
}

// --------------- Cross-Asset Core ---------------

fn get_asset_price(env: &Env, asset: &Address) -> Result<i128, ProtocolError> {
    if let Some(p) = Oracle::aggregate_price(env, asset) {
        return Ok(p);
    }
    let prices = AssetRegistryStorage::get_prices_map(env);
    let price = prices.get(asset.clone()).ok_or(ProtocolError::PriceNotAvailable)?;
    if price <= 0 { return Err(ProtocolError::PriceNotAvailable); }
    Ok(price)
}

fn get_asset_params(env: &Env, asset: &Address) -> Result<AssetParams, ProtocolError> {
    let params_map = AssetRegistryStorage::get_params_map(env);
    let params = params_map.get(asset.clone()).ok_or(ProtocolError::AssetNotSupported)?;
    if params.collateral_factor < 0 || params.collateral_factor > 100000000 {
        return Err(ProtocolError::CollateralFactorInvalid);
    }
    Ok(params)
}

fn calc_cross_totals(env: &Env, pos: &CrossPosition) -> Result<(i128, i128), ProtocolError> {
    // Returns (weighted_collateral_value, total_debt_value) both scaled by 1e8
    let mut total_collateral_value: i128 = 0;
    let mut total_debt_value: i128 = 0;

    let mut keys: Vec<Address> = Vec::new(env);
    for (asset, _bal) in pos.collateral.iter() { keys.push_back(asset); }
    for (asset, _bal) in pos.debt.iter() { keys.push_back(asset); }

    // Deduplicate keys (simple O(n^2))
    let mut uniq: Vec<Address> = Vec::new(env);
    'outer: for a in keys.iter() {
        for b in uniq.iter() { if a == b { continue 'outer; } }
        uniq.push_back(a);
    }

    for asset in uniq.iter() {
        let price = get_asset_price(env, &asset)?; // 1e8 scaled
        let params = get_asset_params(env, &asset)?;
        let c = pos.collateral.get(asset.clone()).unwrap_or(0);
        let d = pos.debt.get(asset.clone()).unwrap_or(0);
        // Weighted collateral value: c * price * cf / 1e16
        total_collateral_value += (c * price * params.collateral_factor) / 100000000 / 100000000;
        // Debt value: d * price / 1e8
        total_debt_value += (d * price) / 100000000;
    }

    Ok((total_collateral_value, total_debt_value))
}

/// Deposit collateral for a specific asset (cross-asset)
pub fn deposit_collateral_asset(env: Env, user: String, asset: Address, amount: i128) -> Result<(), ProtocolError> {
    ReentrancyGuard::enter(&env)?;
    let result = (|| {
        if user.is_empty() { return Err(ProtocolError::InvalidAddress); }
        if amount <= 0 { return Err(ProtocolError::InvalidAmount); }
        let params = get_asset_params(&env, &asset)?;
        if !params.cross_enabled || !params.deposit_enabled { return Err(ProtocolError::CrossAssetDisabled); }

        let user_addr = Address::from_string(&user);
        let mut x = CrossStateHelper::get_or_init_position(&env, &user_addr);
        let bal = x.collateral.get(asset.clone()).unwrap_or(0) + amount;
        x.collateral.set(asset.clone(), bal);
        CrossStateHelper::save_position(&env, &x);
        ProtocolEvent::CrossDeposit(user_addr, asset, amount).emit(&env);
        Ok(())
    })();
    ReentrancyGuard::exit(&env);
    result
}

/// Borrow a specific asset against total cross-asset collateral
pub fn borrow_asset(env: Env, user: String, asset: Address, amount: i128) -> Result<(), ProtocolError> {
    ReentrancyGuard::enter(&env)?;
    let result = (|| {
        if user.is_empty() { return Err(ProtocolError::InvalidAddress); }
        if amount <= 0 { return Err(ProtocolError::InvalidAmount); }
        let params = get_asset_params(&env, &asset)?;
        if !params.cross_enabled || !params.borrow_enabled { return Err(ProtocolError::CrossAssetDisabled); }

        let user_addr = Address::from_string(&user);
        let mut x = CrossStateHelper::get_or_init_position(&env, &user_addr);

        if x.last_accrual_time == 0 { x.last_accrual_time = env.ledger().timestamp(); }

        let prev = x.debt.get(asset.clone()).unwrap_or(0);
        x.debt.set(asset.clone(), prev + amount);
        let (total_collateral, total_debt) = calc_cross_totals(&env, &x)?;
        let min_ratio = ProtocolConfig::get_min_collateral_ratio(&env); // percent
        let ratio = if total_debt > 0 { (total_collateral * 100) / total_debt } else { 0 };
        if ratio < min_ratio { return Err(ProtocolError::InsufficientCollateralRatio); }

        CrossStateHelper::save_position(&env, &x);
        ProtocolEvent::CrossBorrow(user_addr, asset, amount).emit(&env);
        Ok(())
    })();
    ReentrancyGuard::exit(&env);
    result
}

/// Repay debt for a specific asset
pub fn repay_asset(env: Env, user: String, asset: Address, amount: i128) -> Result<(), ProtocolError> {
    ReentrancyGuard::enter(&env)?;
    let result = (|| {
        if user.is_empty() { return Err(ProtocolError::InvalidAddress); }
        if amount <= 0 { return Err(ProtocolError::InvalidAmount); }
        let _ = get_asset_params(&env, &asset)?; // ensure asset exists
        let user_addr = Address::from_string(&user);
        let mut x = CrossStateHelper::get_or_init_position(&env, &user_addr);
        let prev = x.debt.get(asset.clone()).unwrap_or(0);
        if prev == 0 { return Err(ProtocolError::InvalidOperation); }
        let new_debt = if amount > prev { 0 } else { prev - amount };
        x.debt.set(asset.clone(), new_debt);
        CrossStateHelper::save_position(&env, &x);
        ProtocolEvent::CrossRepay(user_addr, asset, amount).emit(&env);
        Ok(())
    })();
    ReentrancyGuard::exit(&env);
    result
}

/// Withdraw collateral for a specific asset (checks cross-asset ratio)
pub fn withdraw_asset(env: Env, user: String, asset: Address, amount: i128) -> Result<(), ProtocolError> {
    ReentrancyGuard::enter(&env)?;
    let result = (|| {
        if user.is_empty() { return Err(ProtocolError::InvalidAddress); }
        if amount <= 0 { return Err(ProtocolError::InvalidAmount); }
        let params = get_asset_params(&env, &asset)?;
        if !params.cross_enabled || !params.deposit_enabled { return Err(ProtocolError::CrossAssetDisabled); }
        let user_addr = Address::from_string(&user);
        let mut x = CrossStateHelper::get_or_init_position(&env, &user_addr);
        let prev = x.collateral.get(asset.clone()).unwrap_or(0);
        if amount > prev { return Err(ProtocolError::InsufficientCollateral); }
        x.collateral.set(asset.clone(), prev - amount);

        // Check ratio after withdrawal
        let (tc, td) = calc_cross_totals(&env, &x)?;
        let min_ratio = ProtocolConfig::get_min_collateral_ratio(&env);
        let ratio = if td > 0 { (tc * 100) / td } else { 0 };
        if td > 0 && ratio < min_ratio { return Err(ProtocolError::InsufficientCollateralRatio); }

        CrossStateHelper::save_position(&env, &x);
        ProtocolEvent::CrossWithdraw(user_addr, asset, amount).emit(&env);
        Ok(())
    })();
    ReentrancyGuard::exit(&env);
    result
}

/// Get cross-asset position summary (total weighted collateral, total debt, ratio)
pub fn get_cross_position_summary(env: Env, user: String) -> Result<(i128, i128, i128), ProtocolError> {
    if user.is_empty() { return Err(ProtocolError::InvalidAddress); }
    let user_addr = Address::from_string(&user);
    let x = CrossStateHelper::get_or_init_position(&env, &user_addr);
    let (tc, td) = calc_cross_totals(&env, &x)?;
    let ratio = if td > 0 { (tc * 100) / td } else { 0 };
    Ok((tc, td, ratio))
}

// ---- Admin helpers for cross-asset ----

/// Add or update supported asset params
pub fn set_asset_params(
    env: Env,
    caller: String,
    asset: Address,
    collateral_factor: i128,
    borrow_enabled: bool,
    deposit_enabled: bool,
    cross_enabled: bool,
) -> Result<(), ProtocolError> {
    let caller_addr = Address::from_string(&caller);
    ProtocolConfig::require_admin(&env, &caller_addr)?;
    if collateral_factor < 0 || collateral_factor > 100000000 {
        return Err(ProtocolError::CollateralFactorInvalid);
    }
    let mut map = AssetRegistryStorage::get_params_map(&env);
    let params = AssetParams { collateral_factor, borrow_enabled, deposit_enabled, cross_enabled };
    map.set(asset, params);
    AssetRegistryStorage::put_params_map(&env, &map);
    Ok(())
}

/// Set price for an asset in 1e8 scale (oracle/admin)
pub fn set_asset_price(env: Env, caller: String, asset: Address, price: i128) -> Result<(), ProtocolError> {
    let caller_addr = Address::from_string(&caller);
    // For now, admin-only setter. Later can gate by oracle address.
    ProtocolConfig::require_admin(&env, &caller_addr)?;
    if price <= 0 { return Err(ProtocolError::InvalidInput); }
    let mut map = AssetRegistryStorage::get_prices_map(&env);
    map.set(asset, price);
    AssetRegistryStorage::put_prices_map(&env, &map);
    Ok(())
}

/// Admin: set dynamic CF parameters for an asset
pub fn set_dynamic_cf_params(
    env: Env,
    caller: String,
    asset: Address,
    min_cf: i128,
    max_cf: i128,
    sensitivity_bps: i128,
    max_step_bps: i128,
) -> Result<(), ProtocolError> {
    let caller_addr = Address::from_string(&caller);
    ProtocolConfig::require_admin(&env, &caller_addr)?;
    if !(0 <= min_cf && min_cf <= 100000000 && 0 <= max_cf && max_cf <= 100000000 && min_cf <= max_cf) {
        return Err(ProtocolError::InvalidInput);
    }
    let mut dyn_map = AssetRegistryStorage::get_dyn_params(&env);
    dyn_map.set(
        asset.clone(),
        DynamicCFParams { min_cf, max_cf, sensitivity_bps, max_step_bps },
    );
    AssetRegistryStorage::put_dyn_params(&env, &dyn_map);
    Ok(())
}

/// Admin/Oracle: push a new price and update collateral factor dynamically
pub fn push_price_and_update_cf(env: Env, caller: String, asset: Address, price: i128) -> Result<i128, ProtocolError> {
    // Set the price first (admin rights required)
    set_asset_price(env.clone(), caller, asset.clone(), price)?;

    // Update market state
    let mut ms_map = AssetRegistryStorage::get_market_state(&env);
    let mut ms = ms_map.get(asset.clone()).unwrap_or_else(MarketState::initial);
    if ms.last_price > 0 {
        // simple absolute return in bps = |p/p0 - 1| * 10000
        let num = (price - ms.last_price).abs() * 10000;
        let den = if ms.last_price == 0 { 1 } else { ms.last_price };
        let ret_bps = num / den;
        // EWMA-like update: vol = (vol*4 + ret)/5
        ms.vol_index_bps = (ms.vol_index_bps * 4 + ret_bps) / 5;
    }
    ms.last_price = price;
    ms_map.set(asset.clone(), ms.clone());
    AssetRegistryStorage::put_market_state(&env, &ms_map);

    // Apply dynamic CF change
    let mut params_map = AssetRegistryStorage::get_params_map(&env);
    let mut asset_params = params_map.get(asset.clone()).unwrap_or_else(AssetParams::default);
    let dyn_map = AssetRegistryStorage::get_dyn_params(&env);
    let dcf = dyn_map.get(asset.clone()).unwrap_or_else(DynamicCFParams::default);

    // Reduce CF proportional to vol: delta_cf_bps = sensitivity_bps * (vol_index_bps / 100)
    let delta_cf_bps = dcf.sensitivity_bps * (ms.vol_index_bps / 100);
    let base_cf_bps = asset_params.collateral_factor / 1000; // convert 1e8 -> bps approx
    let mut target_cf_bps = base_cf_bps - delta_cf_bps;
    // clamp to bounds
    let min_cf_bps = dcf.min_cf / 1000;
    let max_cf_bps = dcf.max_cf / 1000;
    if target_cf_bps < min_cf_bps { target_cf_bps = min_cf_bps; }
    if target_cf_bps > max_cf_bps { target_cf_bps = max_cf_bps; }
    // apply max step
    let current_bps = asset_params.collateral_factor / 1000;
    let diff = target_cf_bps - current_bps;
    let step = if diff.abs() > dcf.max_step_bps { dcf.max_step_bps * diff.signum() } else { diff };
    let new_cf_bps = current_bps + step;
    let new_cf_1e8 = new_cf_bps * 1000;
    asset_params.collateral_factor = new_cf_1e8;
    params_map.set(asset.clone(), asset_params.clone());
    AssetRegistryStorage::put_params_map(&env, &params_map);

    ProtocolEvent::DynamicCFUpdated(asset, new_cf_1e8).emit(&env);
    Ok(new_cf_1e8)
}

// ---- AMM Integration ----

/// Admin: register AMM pool for asset pair
pub fn set_amm_pool(env: Env, caller: String, asset_a: Address, asset_b: Address, pool: Address) -> Result<(), ProtocolError> {
    let caller_addr = Address::from_string(&caller);
    ProtocolConfig::require_admin(&env, &caller_addr)?;
    let key = PairKey::ordered(asset_a, asset_b);
    let mut reg = AssetRegistryStorage::get_amm_registry(&env);
    reg.set(key, pool);
    AssetRegistryStorage::put_amm_registry(&env, &reg);
    Ok(())
}

fn get_pool_for(env: &Env, a: &Address, b: &Address) -> Option<Address> {
    let key = PairKey::ordered(a.clone(), b.clone());
    let reg = AssetRegistryStorage::get_amm_registry(env);
    reg.get(key)
}

/// Swap via registered AMM pool
pub fn amm_swap(
    env: Env,
    user: String,
    asset_in: Address,
    amount_in: i128,
    asset_out: Address,
    min_out: i128,
) -> Result<i128, ProtocolError> {
    ReentrancyGuard::enter(&env)?;
    let result = (|| {
        if user.is_empty() { return Err(ProtocolError::InvalidAddress); }
        if amount_in <= 0 || min_out < 0 { return Err(ProtocolError::InvalidAmount); }
        let pool = get_pool_for(&env, &asset_in, &asset_out).ok_or(ProtocolError::NotFound)?;
        let user_addr = Address::from_string(&user);

        // Call pool.swap(asset_in, amount_in, asset_out, min_out, user)
        let args = vec![
            &env,
            asset_in.clone().into_val(&env),
            amount_in.into_val(&env),
            asset_out.clone().into_val(&env),
            min_out.into_val(&env),
            user_addr.clone().into_val(&env),
        ];
        let amount_out: i128 = env.invoke_contract(&pool, &Symbol::new(&env, "swap"), args);
        if amount_out < min_out { return Err(ProtocolError::InvalidOperation); }
        ProtocolEvent::AMMSwap(user_addr, asset_in, asset_out, amount_in, amount_out).emit(&env);
        Ok(amount_out)
    })();
    ReentrancyGuard::exit(&env);
    result
}

/// Provide liquidity to a pool
pub fn amm_add_liquidity(env: Env, user: String, asset_a: Address, amt_a: i128, asset_b: Address, amt_b: i128) -> Result<(), ProtocolError> {
    ReentrancyGuard::enter(&env)?;
    let result = (|| {
        if user.is_empty() { return Err(ProtocolError::InvalidAddress); }
        if amt_a <= 0 || amt_b <= 0 { return Err(ProtocolError::InvalidAmount); }
        let pool = get_pool_for(&env, &asset_a, &asset_b).ok_or(ProtocolError::NotFound)?;
        let user_addr = Address::from_string(&user);
        let args = vec![
            &env,
            asset_a.clone().into_val(&env),
            amt_a.into_val(&env),
            asset_b.clone().into_val(&env),
            amt_b.into_val(&env),
            user_addr.clone().into_val(&env),
        ];
        let _: () = env.invoke_contract(&pool, &Symbol::new(&env, "add_liquidity"), args);
        ProtocolEvent::AMMLiquidityAdded(user_addr, asset_a, asset_b, amt_a, amt_b).emit(&env);
        Ok(())
    })();
    ReentrancyGuard::exit(&env);
    result
}

/// Remove liquidity from a pool
pub fn amm_remove_liquidity(env: Env, user: String, pool: Address, lp_amount: i128) -> Result<(), ProtocolError> {
    ReentrancyGuard::enter(&env)?;
    let result = (|| {
        if user.is_empty() { return Err(ProtocolError::InvalidAddress); }
        if lp_amount <= 0 { return Err(ProtocolError::InvalidAmount); }
        let user_addr = Address::from_string(&user);
        let args = vec![&env, lp_amount.into_val(&env), user_addr.clone().into_val(&env)];
        let _: () = env.invoke_contract(&pool, &Symbol::new(&env, "remove_liquidity"), args);
        ProtocolEvent::AMMLiquidityRemoved(user_addr, pool, lp_amount).emit(&env);
        Ok(())
    })();
    ReentrancyGuard::exit(&env);
    result
}

// ---- Risk Scoring ----

/// Admin: set global risk parameters
pub fn set_risk_scoring_params(env: Env, caller: String, base_limit_value: i128, score_to_limit_factor: i128, min_rate_adj_bps: i128, max_rate_adj_bps: i128) -> Result<(), ProtocolError> {
    let caller_addr = Address::from_string(&caller);
    ProtocolConfig::require_admin(&env, &caller_addr)?;
    let params = RiskParamsGlobal { base_limit_value, score_to_limit_factor, min_rate_adj_bps, max_rate_adj_bps };
    AssetRegistryStorage::save_risk_params(&env, &params);
    ProtocolEvent::RiskParamsSet(base_limit_value, score_to_limit_factor, min_rate_adj_bps, max_rate_adj_bps).emit(&env);
    Ok(())
}

/// Record a user action and update their risk score
pub fn record_user_action(env: Env, user: String, _action: Symbol) -> Result<(i128, i128), ProtocolError> {
    if user.is_empty() { return Err(ProtocolError::InvalidAddress); }
    let user_addr = Address::from_string(&user);
    let mut risk_map = AssetRegistryStorage::get_user_risk(&env);
    let mut st = risk_map.get(user_addr.clone()).unwrap_or_else(|| UserRiskState::new(user_addr.clone()));
    st.tx_count += 1;
    st.last_update = env.ledger().timestamp();

    // Very simple scoring: score capped by activity count up to 1000
    let score = if st.tx_count > 1000 { 1000 } else { st.tx_count };
    st.score = score;
    let params = AssetRegistryStorage::get_risk_params(&env);
    st.credit_limit_value = params.base_limit_value + params.score_to_limit_factor * st.score;
    risk_map.set(user_addr.clone(), st.clone());
    AssetRegistryStorage::put_user_risk(&env, &risk_map);
    ProtocolEvent::UserRiskUpdated(user_addr, st.score, st.credit_limit_value).emit(&env);
    Ok((st.score, st.credit_limit_value))
}

/// Get user risk state
pub fn get_user_risk(env: Env, user: String) -> Result<(i128, i128, i128, u64), ProtocolError> {
    if user.is_empty() { return Err(ProtocolError::InvalidAddress); }
    let user_addr = Address::from_string(&user);
    let risk_map = AssetRegistryStorage::get_user_risk(&env);
    let st = risk_map.get(user_addr).unwrap_or_else(|| UserRiskState::new(Address::from_string(&String::from_str(&env, ""))));
    Ok((st.score, st.credit_limit_value, st.tx_count, st.last_update))
}

// --------------- Flash Loan ---------------

/// Execute a flash loan by calling `on_flash_loan(asset, amount, fee, initiator)` on receiver.
pub fn flash_loan(
    env: Env,
    initiator: String,
    asset: Address,
    amount: i128,
    receiver_contract: Address,
) -> Result<(), ProtocolError> {
    ReentrancyGuard::enter(&env)?;
    let result = (|| {
        if initiator.is_empty() { return Err(ProtocolError::InvalidAddress); }
        if amount <= 0 { return Err(ProtocolError::InvalidAmount); }
        // Ensure asset is registered (price must exist)
        let _ = get_asset_price(&env, &asset)?;
        let initiator_addr = Address::from_string(&initiator);

        // Calculate fee
        let bps = ProtocolConfig::get_flash_loan_fee_bps(&env); // e.g., 5 bps
        let fee = (amount * bps) / 10000;

        // Emit initiation event
        ProtocolEvent::FlashLoanInitiated(initiator_addr.clone(), asset.clone(), amount, fee).emit(&env);

        // Invoke receiver callback: on_flash_loan(env, asset, amount, fee, initiator)
        // The callee must ensure repayment within the same transaction.
        let args = vec![
            &env,
            asset.clone().into_val(&env),
            amount.into_val(&env),
            fee.into_val(&env),
            initiator_addr.clone().into_val(&env),
        ];
        let _: () = env.invoke_contract(
            &receiver_contract,
            &Symbol::new(&env, "on_flash_loan"),
            args,
        );

        // Basic validation placeholder: in a real implementation, we'd verify the asset amount + fee
        // returned to the protocol treasury. Here, we just assume the callee reverts on failure.

        ProtocolEvent::FlashLoanCompleted(initiator_addr, asset, amount, fee).emit(&env);
        Ok(())
    })();
    ReentrancyGuard::exit(&env);
    result
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

    // Cross-asset entrypoints
    pub fn set_asset_params(
        env: Env,
        caller: String,
        asset: Address,
        collateral_factor: i128,
        borrow_enabled: bool,
        deposit_enabled: bool,
        cross_enabled: bool,
    ) -> Result<(), ProtocolError> {
        set_asset_params(env, caller, asset, collateral_factor, borrow_enabled, deposit_enabled, cross_enabled)
    }

    pub fn set_asset_price(env: Env, caller: String, asset: Address, price: i128) -> Result<(), ProtocolError> {
        set_asset_price(env, caller, asset, price)
    }

    pub fn deposit_collateral_asset(env: Env, user: String, asset: Address, amount: i128) -> Result<(), ProtocolError> {
        deposit_collateral_asset(env, user, asset, amount)
    }

    pub fn borrow_asset(env: Env, user: String, asset: Address, amount: i128) -> Result<(), ProtocolError> {
        borrow_asset(env, user, asset, amount)
    }

    pub fn repay_asset(env: Env, user: String, asset: Address, amount: i128) -> Result<(), ProtocolError> {
        repay_asset(env, user, asset, amount)
    }

    pub fn withdraw_asset(env: Env, user: String, asset: Address, amount: i128) -> Result<(), ProtocolError> {
        withdraw_asset(env, user, asset, amount)
    }

    pub fn get_cross_position_summary(env: Env, user: String) -> Result<(i128, i128, i128), ProtocolError> {
        get_cross_position_summary(env, user)
    }

    // Flash loan entrypoint
    pub fn flash_loan(
        env: Env,
        initiator: String,
        asset: Address,
        amount: i128,
        receiver_contract: Address,
    ) -> Result<(), ProtocolError> {
        flash_loan(env, initiator, asset, amount, receiver_contract)
    }
    /// Admin: set flash loan fee in bps
    pub fn set_flash_loan_fee_bps(env: Env, caller: String, bps: i128) -> Result<(), ProtocolError> {
        let caller_addr = Address::from_string(&caller);
        ProtocolConfig::set_flash_loan_fee_bps(&env, &caller_addr, bps)
    }

    // Dynamic CF admin entrypoints
    pub fn set_dynamic_cf_params(
        env: Env,
        caller: String,
        asset: Address,
        min_cf: i128,
        max_cf: i128,
        sensitivity_bps: i128,
        max_step_bps: i128,
    ) -> Result<(), ProtocolError> {
        set_dynamic_cf_params(env, caller, asset, min_cf, max_cf, sensitivity_bps, max_step_bps)
    }

    pub fn push_price_and_update_cf(env: Env, caller: String, asset: Address, price: i128) -> Result<i128, ProtocolError> {
        push_price_and_update_cf(env, caller, asset, price)
    }

    // AMM integration
    pub fn set_amm_pool(env: Env, caller: String, asset_a: Address, asset_b: Address, pool: Address) -> Result<(), ProtocolError> {
        set_amm_pool(env, caller, asset_a, asset_b, pool)
    }

    pub fn amm_swap(env: Env, user: String, asset_in: Address, amount_in: i128, asset_out: Address, min_out: i128) -> Result<i128, ProtocolError> {
        amm_swap(env, user, asset_in, amount_in, asset_out, min_out)
    }

    pub fn amm_add_liquidity(env: Env, user: String, asset_a: Address, amt_a: i128, asset_b: Address, amt_b: i128) -> Result<(), ProtocolError> {
        amm_add_liquidity(env, user, asset_a, amt_a, asset_b, amt_b)
    }

    pub fn amm_remove_liquidity(env: Env, user: String, pool: Address, lp_amount: i128) -> Result<(), ProtocolError> {
        amm_remove_liquidity(env, user, pool, lp_amount)
    }

    // Risk scoring entrypoints
    pub fn set_risk_scoring_params(env: Env, caller: String, base_limit_value: i128, score_to_limit_factor: i128, min_rate_adj_bps: i128, max_rate_adj_bps: i128) -> Result<(), ProtocolError> {
        set_risk_scoring_params(env, caller, base_limit_value, score_to_limit_factor, min_rate_adj_bps, max_rate_adj_bps)
    }

    pub fn record_user_action(env: Env, user: String, action: Symbol) -> Result<(i128, i128), ProtocolError> {
        record_user_action(env, user, action)
    }

    pub fn get_user_risk(env: Env, user: String) -> Result<(i128, i128, i128, u64), ProtocolError> {
        get_user_risk(env, user)
    }

    // Oracle admin controls
    pub fn oracle_set_source(env: Env, caller: String, asset: Address, oracle_addr: Address, weight: i128, last_heartbeat: u64) -> Result<(), ProtocolError> {
        let caller_addr = Address::from_string(&caller);
        ProtocolConfig::require_admin(&env, &caller_addr)?;
        let src = OracleSource::new(oracle_addr, weight, last_heartbeat);
        Oracle::set_source(&env, &caller_addr, &asset, src);
        Ok(())
    }
    pub fn oracle_remove_source(env: Env, caller: String, asset: Address, oracle_addr: Address) -> Result<(), ProtocolError> {
        let caller_addr = Address::from_string(&caller);
        ProtocolConfig::require_admin(&env, &caller_addr)?;
        Oracle::remove_source(&env, &caller_addr, &asset, &oracle_addr);
        Ok(())
    }
    pub fn oracle_set_heartbeat_ttl(env: Env, caller: String, ttl: u64) -> Result<(), ProtocolError> {
        let caller_addr = Address::from_string(&caller);
        ProtocolConfig::require_admin(&env, &caller_addr)?;
        OracleStorage::set_heartbeat_ttl(&env, ttl);
        Ok(())
    }
}

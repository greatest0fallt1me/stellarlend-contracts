//! StellarLend Soroban Smart Contract
//
//! This contract provides the foundation for the StellarLend DeFi Lending & Borrowing Protocol.
//! Core features will be implemented incrementally in separate modules.

#![no_std]
extern crate alloc;

use alloc::format;
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

/// Auction state for advanced liquidation
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct LiquidationAuction {
    pub user: Address,
    pub asset: Address,
    pub debt_portion: i128,
    pub highest_bid: i128,
    pub highest_bidder: Option<Address>,
    pub start_time: u64,
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
    fn token_registry_key(env: &Env) -> Symbol { Symbol::new(env, "token_registry") }
    fn enforce_transfers_key(env: &Env) -> Symbol { Symbol::new(env, "enforce_transfers") }
    fn base_token_key(env: &Env) -> Symbol { Symbol::new(env, "base_token") }
    fn auction_book_key(env: &Env) -> Symbol { Symbol::new(env, "auction_book") }
    fn corr_key(env: &Env) -> Symbol { Symbol::new(env, "asset_correlations") }
    fn liq_queue_key(env: &Env) -> Symbol { Symbol::new(env, "liquidation_queue") }
    fn kyc_map_key(env: &Env) -> Symbol { Symbol::new(env, "kyc_map") }
    fn mm_params_key(env: &Env) -> Symbol { Symbol::new(env, "mm_params") }
    fn webhook_registry_key(env: &Env) -> Symbol { Symbol::new(env, "webhook_registry") }
    fn fees_key(env: &Env) -> Symbol { Symbol::new(env, "fees_config") }
    fn insurance_key(env: &Env) -> Symbol { Symbol::new(env, "insurance_params") }
    fn breaker_key(env: &Env) -> Symbol { Symbol::new(env, "circuit_breaker") }

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

    pub fn get_token_registry(env: &Env) -> Map<Address, Address> {
        env.storage().instance().get(&Self::token_registry_key(env)).unwrap_or_else(|| Map::new(env))
    }

    pub fn put_token_registry(env: &Env, map: &Map<Address, Address>) {
        env.storage().instance().set(&Self::token_registry_key(env), map);
    }

    pub fn get_enforce_transfers(env: &Env) -> bool {
        env.storage().instance().get(&Self::enforce_transfers_key(env)).unwrap_or(false)
    }

    pub fn set_enforce_transfers(env: &Env, flag: bool) {
        env.storage().instance().set(&Self::enforce_transfers_key(env), &flag);
    }

    pub fn set_base_token(env: &Env, token: &Address) {
        env.storage().instance().set(&Self::base_token_key(env), token);
    }

    pub fn get_base_token(env: &Env) -> Option<Address> {
        env.storage().instance().get(&Self::base_token_key(env))
    }

    pub fn get_auction_book(env: &Env) -> Map<Address, LiquidationAuction> {
        env.storage().instance().get(&Self::auction_book_key(env)).unwrap_or_else(|| Map::new(env))
    }

    pub fn put_auction_book(env: &Env, map: &Map<Address, LiquidationAuction>) {
        env.storage().instance().set(&Self::auction_book_key(env), map);
    }

    

    pub fn get_kyc_map(env: &Env) -> Map<Address, bool> {
        env.storage().instance().get(&Self::kyc_map_key(env)).unwrap_or_else(|| Map::new(env))
    }

    pub fn put_kyc_map(env: &Env, map: &Map<Address, bool>) {
        env.storage().instance().set(&Self::kyc_map_key(env), map);
    }

    pub fn get_liq_queue(env: &Env) -> Vec<Address> {
        env.storage().instance().get(&Self::liq_queue_key(env)).unwrap_or_else(|| Vec::new(env))
    }
    pub fn put_liq_queue(env: &Env, q: &Vec<Address>) { env.storage().instance().set(&Self::liq_queue_key(env), q); }

    pub fn save_mm_params(env: &Env, spread_bps: i128, inventory_cap: i128) {
        let key = Self::mm_params_key(env);
        env.storage().instance().set(&key, &(spread_bps, inventory_cap));
    }

    pub fn get_mm_params(env: &Env) -> (i128, i128) {
        env.storage().instance().get(&Self::mm_params_key(env)).unwrap_or((50, 1_000_000))
    }

    pub fn get_webhooks(env: &Env) -> Map<Symbol, Address> {
        env.storage().instance().get(&Self::webhook_registry_key(env)).unwrap_or_else(|| Map::new(env))
    }

    pub fn put_webhooks(env: &Env, map: &Map<Symbol, Address>) {
        env.storage().instance().set(&Self::webhook_registry_key(env), map);
    }

    pub fn save_fees(env: &Env, base_bps: i128, tier1_bps: i128) {
        env.storage().instance().set(&Self::fees_key(env), &(base_bps, tier1_bps));
    }

    pub fn get_fees(env: &Env) -> (i128, i128) {
        env.storage().instance().get(&Self::fees_key(env)).unwrap_or((5, 3))
    }

    pub fn save_insurance(env: &Env, premium_bps: i128, coverage_cap: i128) {
        env.storage().instance().set(&Self::insurance_key(env), &(premium_bps, coverage_cap));
    }
    pub fn get_insurance(env: &Env) -> (i128, i128) {
        env.storage().instance().get(&Self::insurance_key(env)).unwrap_or((10, 1_000_000))
    }
    pub fn set_breaker(env: &Env, flag: bool) { env.storage().instance().set(&Self::breaker_key(env), &flag); }
    pub fn get_breaker(env: &Env) -> bool { env.storage().instance().get(&Self::breaker_key(env)).unwrap_or(false) }

    pub fn get_correlations(env: &Env) -> Map<PairKey, i128> {
        env.storage().instance().get(&Self::corr_key(env)).unwrap_or_else(|| Map::new(env))
    }

    pub fn put_correlations(env: &Env, map: &Map<PairKey, i128>) {
        env.storage().instance().set(&Self::corr_key(env), map);
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

/// Simple performance counters and cache storage
pub struct PerfStorage;

impl PerfStorage {
    fn counters_key(env: &Env) -> Symbol { Symbol::new(env, "perf_counters") }
    fn cache_key(env: &Env) -> Symbol { Symbol::new(env, "perf_cache") }

    pub fn inc_counter(env: &Env, name: &Symbol, by: i128) -> i128 {
        let mut map: Map<Symbol, i128> = env.storage().instance().get(&Self::counters_key(env)).unwrap_or_else(|| Map::new(env));
        let cur = map.get(name.clone()).unwrap_or(0) + by;
        map.set(name.clone(), cur);
        env.storage().instance().set(&Self::counters_key(env), &map);
        cur
    }

    pub fn get_counter(env: &Env, name: &Symbol) -> i128 {
        let map: Map<Symbol, i128> = env.storage().instance().get(&Self::counters_key(env)).unwrap_or_else(|| Map::new(env));
        map.get(name.clone()).unwrap_or(0)
    }

    pub fn cache_set(env: &Env, key: &Symbol, val: &Symbol) {
        let mut map: Map<Symbol, Symbol> = env.storage().instance().get(&Self::cache_key(env)).unwrap_or_else(|| Map::new(env));
        map.set(key.clone(), val.clone());
        env.storage().instance().set(&Self::cache_key(env), &map);
        ProtocolEvent::CacheUpdated(key.clone(), Symbol::new(env, "set")).emit(env);
    }

    pub fn cache_get(env: &Env, key: &Symbol) -> Option<Symbol> {
        let map: Map<Symbol, Symbol> = env.storage().instance().get(&Self::cache_key(env)).unwrap_or_else(|| Map::new(env));
        map.get(key.clone())
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
            ProtocolEvent::PerfMetric(name, value) => {
                env.events().publish(
                    (Symbol::new(env, "perf_metric"), name.clone()),
                    (
                        Symbol::new(env, "metric"), name.clone(),
                        Symbol::new(env, "value"), *value,
                    )
                );
            }
            ProtocolEvent::CacheUpdated(key, op) => {
                env.events().publish(
                    (Symbol::new(env, "cache_updated"), key.clone()),
                    (
                        Symbol::new(env, "key"), key.clone(),
                        Symbol::new(env, "op"), op.clone(),
                    )
                );
            }
            ProtocolEvent::ComplianceKycUpdated(user, status) => {
                env.events().publish(
                    (Symbol::new(env, "kyc_updated"), Symbol::new(env, "user")),
                    (
                        Symbol::new(env, "user"), user.clone(),
                        Symbol::new(env, "status"), *status,
                    )
                );
            }
            ProtocolEvent::ComplianceAlert(user, code) => {
                env.events().publish(
                    (Symbol::new(env, "compliance_alert"), Symbol::new(env, "user")),
                    (
                        Symbol::new(env, "user"), user.clone(),
                        Symbol::new(env, "code"), code.clone(),
                    )
                );
            }
            ProtocolEvent::MMParamsUpdated(spread_bps, cap) => {
                env.events().publish(
                    (Symbol::new(env, "mm_params_updated"), Symbol::new(env, "spread_bps")),
                    (
                        Symbol::new(env, "spread_bps"), *spread_bps,
                        Symbol::new(env, "inventory_cap"), *cap,
                    )
                );
            }
            ProtocolEvent::MMIncentiveAccrued(user, amount) => {
                env.events().publish(
                    (Symbol::new(env, "mm_incentive"), Symbol::new(env, "user")),
                    (
                        Symbol::new(env, "user"), user.clone(),
                        Symbol::new(env, "amount"), *amount,
                    )
                );
            }
            ProtocolEvent::WebhookRegistered(target, topic) => {
                env.events().publish(
                    (Symbol::new(env, "webhook_registered"), Symbol::new(env, "target")),
                    (
                        Symbol::new(env, "target"), target.clone(),
                        Symbol::new(env, "topic"), topic.clone(),
                    )
                );
            }
            ProtocolEvent::BugReportLogged(reporter, code) => {
                env.events().publish(
                    (Symbol::new(env, "bug_report"), Symbol::new(env, "reporter")),
                    (
                        Symbol::new(env, "reporter"), reporter.clone(),
                        Symbol::new(env, "code"), code.clone(),
                    )
                );
            }
            ProtocolEvent::AuditTrail(action, reference) => {
                env.events().publish(
                    (Symbol::new(env, "audit_trail"), action.clone()),
                    (
                        Symbol::new(env, "action"), action.clone(),
                        Symbol::new(env, "ref"), reference.clone(),
                    )
                );
            }
            ProtocolEvent::FeesUpdated(base, tier1) => {
                env.events().publish(
                    (Symbol::new(env, "fees_updated"), Symbol::new(env, "base")),
                    (
                        Symbol::new(env, "base"), *base,
                        Symbol::new(env, "tier1"), *tier1,
                    )
                );
            }
            ProtocolEvent::InsuranceParamsUpdated(premium, cap) => {
                env.events().publish(
                    (Symbol::new(env, "insurance_params"), Symbol::new(env, "premium")),
                    (
                        Symbol::new(env, "premium"), *premium,
                        Symbol::new(env, "cap"), *cap,
                    )
                );
            }
            ProtocolEvent::CircuitBreaker(flag) => {
                env.events().publish(
                    (Symbol::new(env, "circuit_breaker"), Symbol::new(env, "flag")),
                    (
                        Symbol::new(env, "flag"), *flag,
                    )
                );
            }
            ProtocolEvent::ClaimFiled(user, amount, reason) => {
                env.events().publish(
                    (Symbol::new(env, "claim_filed"), Symbol::new(env, "user")),
                    (
                        Symbol::new(env, "user"), user.clone(),
                        Symbol::new(env, "amount"), *amount,
                        Symbol::new(env, "reason"), reason.clone(),
                    )
                );
            }
            ProtocolEvent::FeesUpdated(base, tier1) => {
                env.events().publish(
                    (Symbol::new(env, "fees_updated"), Symbol::new(env, "base")),
                    (
                        Symbol::new(env, "base"), *base,
                        Symbol::new(env, "tier1"), *tier1,
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
        }
    }
}

/// Analytics structures
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct Metrics {
    pub total_deposited: i128,
    pub total_borrowed: i128,
    pub total_withdrawn: i128,
    pub total_repaid: i128,
    pub active_users: i128,
    pub last_update: u64,
}

impl Metrics { pub fn zero() -> Self { Self { total_deposited:0, total_borrowed:0, total_withdrawn:0, total_repaid:0, active_users:0, last_update:0 } } }

#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct UserMetrics {
    pub deposits: i128,
    pub borrows: i128,
    pub withdrawals: i128,
    pub repayments: i128,
    pub last_active: u64,
}

impl UserMetrics { pub fn zero() -> Self { Self { deposits:0, borrows:0, withdrawals:0, repayments:0, last_active:0 } } }

pub struct AnalyticsStorage;

impl AnalyticsStorage {
    fn metrics_key(env: &Env) -> Symbol { Symbol::new(env, "metrics") }
    fn user_metrics_key(env: &Env) -> Symbol { Symbol::new(env, "user_metrics") }
    fn history_key(env: &Env) -> Symbol { Symbol::new(env, "metrics_history") }

    pub fn get_metrics(env: &Env) -> Metrics {
        env.storage().instance().get(&Self::metrics_key(env)).unwrap_or_else(Metrics::zero)
    }
    pub fn put_metrics(env: &Env, m: &Metrics) {
        env.storage().instance().set(&Self::metrics_key(env), m);
    }
    pub fn get_user_map(env: &Env) -> Map<Address, UserMetrics> {
        env.storage().instance().get(&Self::user_metrics_key(env)).unwrap_or_else(|| Map::new(env))
    }
    pub fn put_user_map(env: &Env, m: &Map<Address, UserMetrics>) {
        env.storage().instance().set(&Self::user_metrics_key(env), m);
    }
    pub fn get_history(env: &Env) -> Map<u64, Metrics> {
        env.storage().instance().get(&Self::history_key(env)).unwrap_or_else(|| Map::new(env))
    }
    pub fn put_history(env: &Env, m: &Map<u64, Metrics>) {
        env.storage().instance().set(&Self::history_key(env), m);
    }
}

fn analytics_record_action(env: &Env, user: &Address, action: &str, amount: i128) {
    // Update global metrics
    let mut m = AnalyticsStorage::get_metrics(env);
    match action {
        "deposit" => m.total_deposited += amount,
        "borrow" => m.total_borrowed += amount,
        "withdraw" => m.total_withdrawn += amount,
        "repay" => m.total_repaid += amount,
        _ => {}
    }
    m.last_update = env.ledger().timestamp();
    AnalyticsStorage::put_metrics(env, &m);

    // Update per-user metrics
    let mut umap = AnalyticsStorage::get_user_map(env);
    let mut um = umap.get(user.clone()).unwrap_or_else(UserMetrics::zero);
    match action {
        "deposit" => um.deposits += amount,
        "borrow" => um.borrows += amount,
        "withdraw" => um.withdrawals += amount,
        "repay" => um.repayments += amount,
        _ => {}
    }
    let was_inactive = um.last_active == 0;
    um.last_active = m.last_update;
    umap.set(user.clone(), um);
    AnalyticsStorage::put_user_map(env, &umap);

    if was_inactive {
        let mut m2 = AnalyticsStorage::get_metrics(env);
        m2.active_users += 1;
        AnalyticsStorage::put_metrics(env, &m2);
    }

    // Append simple daily snapshot
    let bucket = m.last_update / 86400;
    let mut hist = AnalyticsStorage::get_history(env);
    hist.set(bucket, m);
    AnalyticsStorage::put_history(env, &hist);
}

/// Bridge configuration per external network
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct BridgeConfig {
    pub network_id: String,
    pub bridge: Address,
    pub fee_bps: i128,
    pub enabled: bool,
}

impl BridgeConfig {
    pub fn new(network_id: String, bridge: Address, fee_bps: i128) -> Self {
        Self { network_id, bridge, fee_bps, enabled: true }
    }
}

/// Storage for bridge registry and helpers
pub struct BridgeStorage;

impl BridgeStorage {
    fn bridges_key(env: &Env) -> Symbol { Symbol::new(env, "bridges_registry") }

    pub fn get_registry(env: &Env) -> Map<String, BridgeConfig> {
        env.storage().instance().get(&Self::bridges_key(env)).unwrap_or_else(|| Map::new(env))
    }

    pub fn put_registry(env: &Env, m: &Map<String, BridgeConfig>) {
        env.storage().instance().set(&Self::bridges_key(env), m);
    }

    pub fn get(env: &Env, id: &String) -> Option<BridgeConfig> {
        let reg = Self::get_registry(env);
        reg.get(id.clone())
    }
}

fn ensure_amount_positive(amount: i128) -> Result<(), ProtocolError> {
    if amount <= 0 { return Err(ProtocolError::InvalidAmount); }
    Ok(())
}

// --- Social Recovery & MultiSig ---
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct RecoveryRequest {
    pub user: Address,
    pub new_address: Address,
    pub approvals: Map<Address, bool>,
    pub created_at: u64,
    pub delay_secs: u64,
    pub executed: bool,
}

impl RecoveryRequest {
    pub fn new(env: &Env, user: Address, new_address: Address, delay_secs: u64) -> Self {
        Self { user, new_address, approvals: Map::new(env), created_at: env.ledger().timestamp(), delay_secs, executed: false }
    }
}

pub struct RecoveryStorage;

impl RecoveryStorage {
    fn guardians_key(env: &Env) -> Symbol { Symbol::new(env, "guardians") }
    fn requests_key(env: &Env) -> Symbol { Symbol::new(env, "recovery_requests") }
    fn mapping_key(env: &Env) -> Symbol { Symbol::new(env, "recovered_mapping") }

    pub fn get_guardians(env: &Env) -> Map<Address, Vec<Address>> {
        env.storage().instance().get(&Self::guardians_key(env)).unwrap_or_else(|| Map::new(env))
    }
    pub fn put_guardians(env: &Env, m: &Map<Address, Vec<Address>>) { env.storage().instance().set(&Self::guardians_key(env), m); }

    pub fn get_requests(env: &Env) -> Map<Address, RecoveryRequest> {
        env.storage().instance().get(&Self::requests_key(env)).unwrap_or_else(|| Map::new(env))
    }
    pub fn put_requests(env: &Env, m: &Map<Address, RecoveryRequest>) { env.storage().instance().set(&Self::requests_key(env), m); }

    pub fn get_mapping(env: &Env) -> Map<Address, Address> {
        env.storage().instance().get(&Self::mapping_key(env)).unwrap_or_else(|| Map::new(env))
    }
    pub fn put_mapping(env: &Env, m: &Map<Address, Address>) { env.storage().instance().set(&Self::mapping_key(env), m); }
}

pub fn set_guardians(env: Env, user: String, guardians: Vec<Address>) -> Result<(), ProtocolError> {
    if user.is_empty() { return Err(ProtocolError::InvalidAddress); }
    let user_addr = Address::from_string(&user);
    let mut gmap = RecoveryStorage::get_guardians(&env);
    gmap.set(user_addr, guardians);
    RecoveryStorage::put_guardians(&env, &gmap);
    Ok(())
}

pub fn start_recovery(env: Env, guardian: String, user: String, new_address: Address, delay_secs: u64) -> Result<(), ProtocolError> {
    if guardian.is_empty() || user.is_empty() { return Err(ProtocolError::InvalidAddress); }
    let gaddr = Address::from_string(&guardian);
    let uaddr = Address::from_string(&user);
    let gmap = RecoveryStorage::get_guardians(&env);
    let guardians = gmap.get(uaddr.clone()).ok_or(ProtocolError::GuardianNotFound)?;
    let mut authorized = false;
    for ga in guardians.iter() { if ga == gaddr { authorized = true; break; } }
    if !authorized { return Err(ProtocolError::Unauthorized); }

    let mut reqs = RecoveryStorage::get_requests(&env);
    if reqs.contains_key(uaddr.clone()) { return Err(ProtocolError::RecoveryRequestAlreadyExists); }
    let mut req = RecoveryRequest::new(&env, uaddr.clone(), new_address, delay_secs);
    req.approvals.set(gaddr, true);
    reqs.set(uaddr, req);
    RecoveryStorage::put_requests(&env, &reqs);
    Ok(())
}

pub fn approve_recovery(env: Env, guardian: String, user: String) -> Result<(), ProtocolError> {
    if guardian.is_empty() || user.is_empty() { return Err(ProtocolError::InvalidAddress); }
    let gaddr = Address::from_string(&guardian);
    let uaddr = Address::from_string(&user);
    let gmap = RecoveryStorage::get_guardians(&env);
    let guardians = gmap.get(uaddr.clone()).ok_or(ProtocolError::GuardianNotFound)?;
    let mut authorized = false;
    for ga in guardians.iter() { if ga == gaddr { authorized = true; break; } }
    if !authorized { return Err(ProtocolError::Unauthorized); }

    let mut reqs = RecoveryStorage::get_requests(&env);
    let mut req = reqs.get(uaddr.clone()).ok_or(ProtocolError::RecoveryRequestNotFound)?;
    req.approvals.set(gaddr, true);
    reqs.set(uaddr, req);
    RecoveryStorage::put_requests(&env, &reqs);
    Ok(())
}

pub fn execute_recovery(env: Env, user: String, min_approvals: i128) -> Result<Address, ProtocolError> {
    if user.is_empty() { return Err(ProtocolError::InvalidAddress); }
    let uaddr = Address::from_string(&user);
    let mut reqs = RecoveryStorage::get_requests(&env);
    let mut req = reqs.get(uaddr.clone()).ok_or(ProtocolError::RecoveryRequestNotFound)?;
    if req.executed { return Err(ProtocolError::RecoveryFailed); }
    // check approvals
    let mut count: i128 = 0;
    for (_, v) in req.approvals.iter() { if v { count += 1; } }
    if count < min_approvals { return Err(ProtocolError::MultiSigNotReady); }
    // timelock
    if env.ledger().timestamp() < req.created_at + req.delay_secs { return Err(ProtocolError::RecoveryNotReady); }
    req.executed = true;
    reqs.set(uaddr.clone(), req);
    RecoveryStorage::put_requests(&env, &reqs);

    let mut map = RecoveryStorage::get_mapping(&env);
    map.set(uaddr.clone(), reqs.get(uaddr.clone()).unwrap().new_address);
    RecoveryStorage::put_mapping(&env, &map);
    Ok(map.get(uaddr).unwrap())
}

// Simple MultiSig for admin operations
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub enum MsActionKind { SetMinCR(i128) }

#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct MsProposal {
    pub id: u64,
    pub action: MsActionKind,
    pub approvals: Map<Address, bool>,
    pub executed: bool,
}

pub struct MsStorage;

impl MsStorage {
    fn admins_key(env: &Env) -> Symbol { Symbol::new(env, "ms_admins") }
    fn threshold_key(env: &Env) -> Symbol { Symbol::new(env, "ms_threshold") }
    fn counter_key(env: &Env) -> Symbol { Symbol::new(env, "ms_counter") }
    fn props_key(env: &Env) -> Symbol { Symbol::new(env, "ms_props") }

    pub fn get_admins(env: &Env) -> Vec<Address> { env.storage().instance().get(&Self::admins_key(env)).unwrap_or_else(|| Vec::new(env)) }
    pub fn set_admins(env: &Env, v: &Vec<Address>) { env.storage().instance().set(&Self::admins_key(env), v); }
    pub fn get_threshold(env: &Env) -> i128 { env.storage().instance().get(&Self::threshold_key(env)).unwrap_or(2) }
    pub fn set_threshold(env: &Env, t: i128) { env.storage().instance().set(&Self::threshold_key(env), &t); }
    pub fn next_id(env: &Env) -> u64 { let mut c: u64 = env.storage().instance().get(&Self::counter_key(env)).unwrap_or(0u64); c+=1; env.storage().instance().set(&Self::counter_key(env), &c); c }
    pub fn get_props(env: &Env) -> Map<u64, MsProposal> { env.storage().instance().get(&Self::props_key(env)).unwrap_or_else(|| Map::new(env)) }
    pub fn put_props(env: &Env, m: &Map<u64, MsProposal>) { env.storage().instance().set(&Self::props_key(env), m); }
}

fn ms_require_admin(env: &Env, caller: &Address) -> Result<(), ProtocolError> {
    let admins = MsStorage::get_admins(env);
    for a in admins.iter() { if a == *caller { return Ok(()); } }
    Err(ProtocolError::Unauthorized)
}

pub fn ms_set_admins(env: Env, caller: String, admins: Vec<Address>, threshold: i128) -> Result<(), ProtocolError> {
    let caller_addr = Address::from_string(&caller);
    ProtocolConfig::require_admin(&env, &caller_addr)?;
    if threshold <= 0 { return Err(ProtocolError::InvalidInput); }
    MsStorage::set_admins(&env, &admins);
    MsStorage::set_threshold(&env, threshold);
    Ok(())
}

pub fn ms_propose_set_min_cr(env: Env, caller: String, ratio: i128) -> Result<u64, ProtocolError> {
    let caller_addr = Address::from_string(&caller);
    ms_require_admin(&env, &caller_addr)?;
    if ratio <= 0 { return Err(ProtocolError::InvalidInput); }
    let id = MsStorage::next_id(&env);
    let mut props = MsStorage::get_props(&env);
    let mut approvals = Map::new(&env);
    approvals.set(caller_addr, true);
    let prop = MsProposal { id, action: MsActionKind::SetMinCR(ratio), approvals, executed: false };
    props.set(id, prop);
    MsStorage::put_props(&env, &props);
    Ok(id)
}

pub fn ms_approve(env: Env, caller: String, id: u64) -> Result<(), ProtocolError> {
    let caller_addr = Address::from_string(&caller);
    ms_require_admin(&env, &caller_addr)?;
    let mut props = MsStorage::get_props(&env);
    let mut p = props.get(id).ok_or(ProtocolError::MultiSigProposalNotFound)?;
    p.approvals.set(caller_addr, true);
    props.set(id, p);
    MsStorage::put_props(&env, &props);
    Ok(())
}

pub fn ms_execute(env: Env, id: u64) -> Result<(), ProtocolError> {
    let mut props = MsStorage::get_props(&env);
    let mut p = props.get(id).ok_or(ProtocolError::MultiSigProposalNotFound)?;
    if p.executed { return Err(ProtocolError::InvalidOperation); }
    // count approvals
    let mut cnt: i128 = 0;
    for (_, v) in p.approvals.iter() { if v { cnt += 1; } }
    if cnt < MsStorage::get_threshold(&env) { return Err(ProtocolError::MultiSigNotReady); }
    // execute action
    match p.action.clone() {
        MsActionKind::SetMinCR(ratio) => {
            let admin = ProtocolConfig::get_admin(&env).ok_or(ProtocolError::Unauthorized)?;
            ProtocolConfig::set_min_collateral_ratio(&env, &admin, ratio)?;
        }
    }
    p.executed = true;
    props.set(id, p);
    MsStorage::put_props(&env, &props);
    Ok(())
}
/// Bridge configuration per external network
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct BridgeConfig {
    pub network_id: String,
    pub bridge: Address,
    pub fee_bps: i128,
    pub enabled: bool,
}

impl BridgeConfig {
    pub fn new(network_id: String, bridge: Address, fee_bps: i128) -> Self {
        Self { network_id, bridge, fee_bps, enabled: true }
    }
}

/// Storage for bridge registry and helpers
pub struct BridgeStorage;

impl BridgeStorage {
    fn bridges_key(env: &Env) -> Symbol { Symbol::new(env, "bridges_registry") }

    pub fn get_registry(env: &Env) -> Map<String, BridgeConfig> {
        env.storage().instance().get(&Self::bridges_key(env)).unwrap_or_else(|| Map::new(env))
    }

    pub fn put_registry(env: &Env, m: &Map<String, BridgeConfig>) {
        env.storage().instance().set(&Self::bridges_key(env), m);
    }

    pub fn get(env: &Env, id: &String) -> Option<BridgeConfig> {
        let reg = Self::get_registry(env);
        reg.get(id.clone())
    }
}

fn ensure_amount_positive(amount: i128) -> Result<(), ProtocolError> {
    if amount <= 0 { return Err(ProtocolError::InvalidAmount); }
    Ok(())
}

// --- Social Recovery & MultiSig ---
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct RecoveryRequest {
    pub user: Address,
    pub new_address: Address,
    pub approvals: Map<Address, bool>,
    pub created_at: u64,
    pub delay_secs: u64,
    pub executed: bool,
}

impl RecoveryRequest {
    pub fn new(env: &Env, user: Address, new_address: Address, delay_secs: u64) -> Self {
        Self { user, new_address, approvals: Map::new(env), created_at: env.ledger().timestamp(), delay_secs, executed: false }
    }
}

pub struct RecoveryStorage;

impl RecoveryStorage {
    fn guardians_key(env: &Env) -> Symbol { Symbol::new(env, "guardians") }
    fn requests_key(env: &Env) -> Symbol { Symbol::new(env, "recovery_requests") }
    fn mapping_key(env: &Env) -> Symbol { Symbol::new(env, "recovered_mapping") }

    pub fn get_guardians(env: &Env) -> Map<Address, Vec<Address>> {
        env.storage().instance().get(&Self::guardians_key(env)).unwrap_or_else(|| Map::new(env))
    }
    pub fn put_guardians(env: &Env, m: &Map<Address, Vec<Address>>) { env.storage().instance().set(&Self::guardians_key(env), m); }

    pub fn get_requests(env: &Env) -> Map<Address, RecoveryRequest> {
        env.storage().instance().get(&Self::requests_key(env)).unwrap_or_else(|| Map::new(env))
    }
    pub fn put_requests(env: &Env, m: &Map<Address, RecoveryRequest>) { env.storage().instance().set(&Self::requests_key(env), m); }

    pub fn get_mapping(env: &Env) -> Map<Address, Address> {
        env.storage().instance().get(&Self::mapping_key(env)).unwrap_or_else(|| Map::new(env))
    }
    pub fn put_mapping(env: &Env, m: &Map<Address, Address>) { env.storage().instance().set(&Self::mapping_key(env), m); }
}

pub fn set_guardians(env: Env, user: String, guardians: Vec<Address>) -> Result<(), ProtocolError> {
    if user.is_empty() { return Err(ProtocolError::InvalidAddress); }
    let user_addr = Address::from_string(&user);
    let mut gmap = RecoveryStorage::get_guardians(&env);
    gmap.set(user_addr, guardians);
    RecoveryStorage::put_guardians(&env, &gmap);
    Ok(())
}

pub fn start_recovery(env: Env, guardian: String, user: String, new_address: Address, delay_secs: u64) -> Result<(), ProtocolError> {
    if guardian.is_empty() || user.is_empty() { return Err(ProtocolError::InvalidAddress); }
    let gaddr = Address::from_string(&guardian);
    let uaddr = Address::from_string(&user);
    let gmap = RecoveryStorage::get_guardians(&env);
    let guardians = gmap.get(uaddr.clone()).ok_or(ProtocolError::GuardianNotFound)?;
    let mut authorized = false;
    for ga in guardians.iter() { if ga == gaddr { authorized = true; break; } }
    if !authorized { return Err(ProtocolError::Unauthorized); }

    let mut reqs = RecoveryStorage::get_requests(&env);
    if reqs.contains_key(uaddr.clone()) { return Err(ProtocolError::RecoveryRequestAlreadyExists); }
    let mut req = RecoveryRequest::new(&env, uaddr.clone(), new_address, delay_secs);
    req.approvals.set(gaddr, true);
    reqs.set(uaddr, req);
    RecoveryStorage::put_requests(&env, &reqs);
    Ok(())
}

pub fn approve_recovery(env: Env, guardian: String, user: String) -> Result<(), ProtocolError> {
    if guardian.is_empty() || user.is_empty() { return Err(ProtocolError::InvalidAddress); }
    let gaddr = Address::from_string(&guardian);
    let uaddr = Address::from_string(&user);
    let gmap = RecoveryStorage::get_guardians(&env);
    let guardians = gmap.get(uaddr.clone()).ok_or(ProtocolError::GuardianNotFound)?;
    let mut authorized = false;
    for ga in guardians.iter() { if ga == gaddr { authorized = true; break; } }
    if !authorized { return Err(ProtocolError::Unauthorized); }

    let mut reqs = RecoveryStorage::get_requests(&env);
    let mut req = reqs.get(uaddr.clone()).ok_or(ProtocolError::RecoveryRequestNotFound)?;
    req.approvals.set(gaddr, true);
    reqs.set(uaddr, req);
    RecoveryStorage::put_requests(&env, &reqs);
    Ok(())
}

pub fn execute_recovery(env: Env, user: String, min_approvals: i128) -> Result<Address, ProtocolError> {
    if user.is_empty() { return Err(ProtocolError::InvalidAddress); }
    let uaddr = Address::from_string(&user);
    let mut reqs = RecoveryStorage::get_requests(&env);
    let mut req = reqs.get(uaddr.clone()).ok_or(ProtocolError::RecoveryRequestNotFound)?;
    if req.executed { return Err(ProtocolError::RecoveryFailed); }
    // check approvals
    let mut count: i128 = 0;
    for (_, v) in req.approvals.iter() { if v { count += 1; } }
    if count < min_approvals { return Err(ProtocolError::MultiSigNotReady); }
    // timelock
    if env.ledger().timestamp() < req.created_at + req.delay_secs { return Err(ProtocolError::RecoveryNotReady); }
    req.executed = true;
    reqs.set(uaddr.clone(), req);
    RecoveryStorage::put_requests(&env, &reqs);

    let mut map = RecoveryStorage::get_mapping(&env);
    map.set(uaddr.clone(), reqs.get(uaddr.clone()).unwrap().new_address);
    RecoveryStorage::put_mapping(&env, &map);
    Ok(map.get(uaddr).unwrap())
}

// Simple MultiSig for admin operations
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub enum MsActionKind { SetMinCR(i128) }

#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct MsProposal {
    pub id: u64,
    pub action: MsActionKind,
    pub approvals: Map<Address, bool>,
    pub executed: bool,
}

pub struct MsStorage;

impl MsStorage {
    fn admins_key(env: &Env) -> Symbol { Symbol::new(env, "ms_admins") }
    fn threshold_key(env: &Env) -> Symbol { Symbol::new(env, "ms_threshold") }
    fn counter_key(env: &Env) -> Symbol { Symbol::new(env, "ms_counter") }
    fn props_key(env: &Env) -> Symbol { Symbol::new(env, "ms_props") }

    pub fn get_admins(env: &Env) -> Vec<Address> { env.storage().instance().get(&Self::admins_key(env)).unwrap_or_else(|| Vec::new(env)) }
    pub fn set_admins(env: &Env, v: &Vec<Address>) { env.storage().instance().set(&Self::admins_key(env), v); }
    pub fn get_threshold(env: &Env) -> i128 { env.storage().instance().get(&Self::threshold_key(env)).unwrap_or(2) }
    pub fn set_threshold(env: &Env, t: i128) { env.storage().instance().set(&Self::threshold_key(env), &t); }
    pub fn next_id(env: &Env) -> u64 { let mut c: u64 = env.storage().instance().get(&Self::counter_key(env)).unwrap_or(0u64); c+=1; env.storage().instance().set(&Self::counter_key(env), &c); c }
    pub fn get_props(env: &Env) -> Map<u64, MsProposal> { env.storage().instance().get(&Self::props_key(env)).unwrap_or_else(|| Map::new(env)) }
    pub fn put_props(env: &Env, m: &Map<u64, MsProposal>) { env.storage().instance().set(&Self::props_key(env), m); }
}

fn ms_require_admin(env: &Env, caller: &Address) -> Result<(), ProtocolError> {
    let admins = MsStorage::get_admins(env);
    for a in admins.iter() { if a == *caller { return Ok(()); } }
    Err(ProtocolError::Unauthorized)
}

pub fn ms_set_admins(env: Env, caller: String, admins: Vec<Address>, threshold: i128) -> Result<(), ProtocolError> {
    let caller_addr = Address::from_string(&caller);
    ProtocolConfig::require_admin(&env, &caller_addr)?;
    if threshold <= 0 { return Err(ProtocolError::InvalidInput); }
    MsStorage::set_admins(&env, &admins);
    MsStorage::set_threshold(&env, threshold);
    Ok(())
}

pub fn ms_propose_set_min_cr(env: Env, caller: String, ratio: i128) -> Result<u64, ProtocolError> {
    let caller_addr = Address::from_string(&caller);
    ms_require_admin(&env, &caller_addr)?;
    if ratio <= 0 { return Err(ProtocolError::InvalidInput); }
    let id = MsStorage::next_id(&env);
    let mut props = MsStorage::get_props(&env);
    let mut approvals = Map::new(&env);
    approvals.set(caller_addr, true);
    let prop = MsProposal { id, action: MsActionKind::SetMinCR(ratio), approvals, executed: false };
    props.set(id, prop);
    MsStorage::put_props(&env, &props);
    Ok(id)
}

pub fn ms_approve(env: Env, caller: String, id: u64) -> Result<(), ProtocolError> {
    let caller_addr = Address::from_string(&caller);
    ms_require_admin(&env, &caller_addr)?;
    let mut props = MsStorage::get_props(&env);
    let mut p = props.get(id).ok_or(ProtocolError::MultiSigProposalNotFound)?;
    p.approvals.set(caller_addr, true);
    props.set(id, p);
    MsStorage::put_props(&env, &props);
    Ok(())
}

pub fn ms_execute(env: Env, id: u64) -> Result<(), ProtocolError> {
    let mut props = MsStorage::get_props(&env);
    let mut p = props.get(id).ok_or(ProtocolError::MultiSigProposalNotFound)?;
    if p.executed { return Err(ProtocolError::InvalidOperation); }
    // count approvals
    let mut cnt: i128 = 0;
    for (_, v) in p.approvals.iter() { if v { cnt += 1; } }
    if cnt < MsStorage::get_threshold(&env) { return Err(ProtocolError::MultiSigNotReady); }
    // execute action
    match p.action.clone() {
        MsActionKind::SetMinCR(ratio) => {
            let admin = ProtocolConfig::get_admin(&env).ok_or(ProtocolError::Unauthorized)?;
            ProtocolConfig::set_min_collateral_ratio(&env, &admin, ratio)?;
        }
    }
    p.executed = true;
    props.set(id, p);
    MsStorage::put_props(&env, &props);
    Ok(())
}

/// Minimum collateral ratio required (e.g., 150%)
const MIN_COLLATERAL_RATIO: i128 = 150;

// --- Upgrade Mechanism ---
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct UpgradeInfo {
    pub current_version: u32,
    pub previous_version: u32,
    pub pending_version: u32,
    pub pending_hash: String,
    pub approved: bool,
    pub last_update: u64,
}

impl UpgradeInfo { pub fn initial(env: &Env) -> Self { Self { current_version: 1, previous_version: 0, pending_version: 0, pending_hash: String::from_str(env, ""), approved: false, last_update: 0 } } }

pub struct UpgradeStorage;

impl UpgradeStorage {
    fn key(env: &Env) -> Symbol { Symbol::new(env, "upgrade_info") }
    pub fn get(env: &Env) -> UpgradeInfo { env.storage().instance().get(&Self::key(env)).unwrap_or_else(|| UpgradeInfo::initial(env)) }
    pub fn put(env: &Env, u: &UpgradeInfo) { env.storage().instance().set(&Self::key(env), u); }
}

pub fn upgrade_propose(env: Env, caller: String, new_version: u32, code_hash: String) -> Result<(), ProtocolError> {
    let caller_addr = Address::from_string(&caller);
    ProtocolConfig::require_admin(&env, &caller_addr)?;
    if new_version == 0 { return Err(ProtocolError::InvalidInput); }
    let mut info = UpgradeStorage::get(&env);
    info.pending_version = new_version;
    info.pending_hash = code_hash;
    info.approved = false;
    info.last_update = env.ledger().timestamp();
    UpgradeStorage::put(&env, &info);
    Ok(())
}

pub fn upgrade_approve(env: Env, caller: String) -> Result<(), ProtocolError> {
    let caller_addr = Address::from_string(&caller);
    ProtocolConfig::require_admin(&env, &caller_addr)?;
    let mut info = UpgradeStorage::get(&env);
    if info.pending_version == 0 { return Err(ProtocolError::InvalidOperation); }
    info.approved = true;
    info.last_update = env.ledger().timestamp();
    UpgradeStorage::put(&env, &info);
    Ok(())
}

pub fn upgrade_execute(env: Env, caller: String) -> Result<u32, ProtocolError> {
    let caller_addr = Address::from_string(&caller);
    ProtocolConfig::require_admin(&env, &caller_addr)?;
    let mut info = UpgradeStorage::get(&env);
    if !info.approved || info.pending_version == 0 { return Err(ProtocolError::InvalidOperation); }
    info.previous_version = info.current_version;
    info.current_version = info.pending_version;
    info.pending_version = 0;
    info.pending_hash = String::from_str(&env, "");
    info.approved = false;
    info.last_update = env.ledger().timestamp();
    UpgradeStorage::put(&env, &info);
    Ok(info.current_version)
}

pub fn upgrade_rollback(env: Env, caller: String) -> Result<u32, ProtocolError> {
    let caller_addr = Address::from_string(&caller);
    ProtocolConfig::require_admin(&env, &caller_addr)?;
    let mut info = UpgradeStorage::get(&env);
    if info.previous_version == 0 { return Err(ProtocolError::InvalidOperation); }
    let tmp = info.current_version;
    info.current_version = info.previous_version;
    info.previous_version = tmp;
    info.last_update = env.ledger().timestamp();
    UpgradeStorage::put(&env, &info);
    Ok(info.current_version)
}

pub fn upgrade_status(env: Env) -> (u32, u32, u32, String, bool, u64) {
    let i = UpgradeStorage::get(&env);
    (i.current_version, i.previous_version, i.pending_version, i.pending_hash, i.approved, i.last_update)
}

// --- Data Management & Storage Optimization ---
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct DataBlob {
    pub version: u32,
    pub compressed: bool,
    pub data: Bytes,
}

pub struct DataStorage;

impl DataStorage {
    fn prefix(env: &Env) -> Symbol { Symbol::new(env, "data_store") }
    fn backup_prefix(env: &Env) -> Symbol { Symbol::new(env, "data_store_backup") }

    fn key_for(env: &Env, name: &Symbol) -> (Symbol, Symbol) { (Self::prefix(env), name.clone()) }
    fn bkey_for(env: &Env, name: &Symbol) -> (Symbol, Symbol) { (Self::backup_prefix(env), name.clone()) }

    pub fn save(env: &Env, name: &Symbol, blob: &DataBlob) {
        env.storage().persistent().set(&Self::key_for(env, name), blob);
    }
    pub fn load(env: &Env, name: &Symbol) -> Option<DataBlob> {
        env.storage().persistent().get(&Self::key_for(env, name))
    }
    pub fn backup(env: &Env, name: &Symbol) -> Result<(), ProtocolError> {
        let blob = Self::load(env, name).ok_or(ProtocolError::NotFound)?;
        env.storage().persistent().set(&Self::bkey_for(env, name), &blob);
        Ok(())
    }
    pub fn restore(env: &Env, name: &Symbol) -> Result<(), ProtocolError> {
        let blob: DataBlob = env.storage().persistent().get(&Self::bkey_for(env, name)).ok_or(ProtocolError::NotFound)?;
        env.storage().persistent().set(&Self::key_for(env, name), &blob);
        Ok(())
    }
}

fn compress_identity(_env: &Env, data: &Bytes) -> Bytes { data.clone() }
fn decompress_identity(_env: &Env, data: &Bytes) -> Bytes { data.clone() }

pub fn data_save(env: Env, name: Symbol, version: u32, data: Bytes, compress: bool) -> Result<(), ProtocolError> {
    if version == 0 { return Err(ProtocolError::InvalidInput); }
    let d = if compress { compress_identity(&env, &data) } else { data };
    let blob = DataBlob { version, compressed: compress, data: d };
    DataStorage::save(&env, &name, &blob);
    Ok(())
}

pub fn data_load(env: Env, name: Symbol) -> Result<(u32, Bytes), ProtocolError> {
    let b = DataStorage::load(&env, &name).ok_or(ProtocolError::NotFound)?;
    let d = if b.compressed { decompress_identity(&env, &b.data) } else { b.data };
    Ok((b.version, d))
}

pub fn data_backup(env: Env, name: Symbol) -> Result<(), ProtocolError> { DataStorage::backup(&env, &name) }
pub fn data_restore(env: Env, name: Symbol) -> Result<(), ProtocolError> { DataStorage::restore(&env, &name) }

pub fn data_migrate_bump_version(env: Env, name: Symbol, new_version: u32) -> Result<(), ProtocolError> {
    if new_version == 0 { return Err(ProtocolError::InvalidInput); }
    let mut b = DataStorage::load(&env, &name).ok_or(ProtocolError::NotFound)?;
    if new_version <= b.version { return Err(ProtocolError::InvalidOperation); }
    b.version = new_version;
    DataStorage::save(&env, &name, &b);
    Ok(())
}

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

        // If token transfers enforced, perform transferFrom depositor -> this contract for configured base asset (single-asset path)
        if AssetRegistryStorage::get_enforce_transfers(&env) {
            if let Some(token_addr) = AssetRegistryStorage::get_base_token(&env) {
                let client = TokenClient::new(&env, &token_addr);
                client.transfer_from(&depositor_addr, &env.current_contract_address(), &depositor_addr, &amount);
            }
        }

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

        // Analytics
        analytics_record_action(&env, &Address::from_string(&depositor), "deposit", amount);

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

        // Analytics
        analytics_record_action(&env, &Address::from_string(&borrower), "borrow", amount);

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

        // Analytics
        analytics_record_action(&env, &Address::from_string(&repayer), "repay", repay_amount);

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

        // If token transfers enforced, perform transfer to withdrawer for configured base asset (single-asset path)
        if AssetRegistryStorage::get_enforce_transfers(&env) {
            if let Some(token_addr) = AssetRegistryStorage::get_base_token(&env) {
                let client = TokenClient::new(&env, &token_addr);
                client.transfer(&env.current_contract_address(), &withdrawer_addr, &amount);
            }
        }

        // Emit event
        ProtocolEvent::PositionUpdated(
            withdrawer_addr,
            position.collateral,
            position.debt,
            collateral_ratio,
        ).emit(&env);

        // Analytics
        analytics_record_action(&env, &Address::from_string(&withdrawer), "withdraw", amount);

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

/// Set pairwise asset correlation in bps (-10000..=10000)
pub fn set_asset_correlation(env: Env, caller: String, a: Address, b: Address, corr_bps: i128) -> Result<(), ProtocolError> {
    let caller_addr = Address::from_string(&caller);
    ProtocolConfig::require_admin(&env, &caller_addr)?;
    if corr_bps < -10000 || corr_bps > 10000 { return Err(ProtocolError::InvalidInput); }
    let mut map = AssetRegistryStorage::get_correlations(&env);
    map.set(PairKey::ordered(a, b), corr_bps);
    AssetRegistryStorage::put_correlations(&env, &map);
    Ok(())
}

/// Portfolio-adjusted ratio using correlations (placeholder: reduces collateral by average positive corr)
pub fn get_portfolio_risk_ratio(env: Env, user: String) -> Result<i128, ProtocolError> {
    if user.is_empty() { return Err(ProtocolError::InvalidAddress); }
    let user_addr = Address::from_string(&user);
    let x = CrossStateHelper::get_or_init_position(&env, &user_addr);
    let (tc, td) = calc_cross_totals(&env, &x)?;
    if td == 0 { return Ok(0); }
    let corr = AssetRegistryStorage::get_correlations(&env);
    // naive penalty: if any positive corr exists, reduce 5%
    let penalty = if corr.len() > 0 { 5 } else { 0 };
    let ratio = (tc * 100) / td;
    Ok(ratio - penalty)
}

// ---- Compliance: KYC/AML scaffolding ----
pub fn set_kyc_status(env: Env, caller: String, user: Address, status: bool) -> Result<(), ProtocolError> {
    let caller_addr = Address::from_string(&caller);
    ProtocolConfig::require_admin(&env, &caller_addr)?;
    let mut map = AssetRegistryStorage::get_kyc_map(&env);
    map.set(user.clone(), status);
    AssetRegistryStorage::put_kyc_map(&env, &map);
    ProtocolEvent::ComplianceKycUpdated(user, status).emit(&env);
    Ok(())
}

pub fn get_kyc_status(env: Env, user: Address) -> bool {
    let map = AssetRegistryStorage::get_kyc_map(&env);
    map.get(user).unwrap_or(false)
}

pub fn report_compliance_event(env: Env, reporter: String, user: Address, code: Symbol) -> Result<(), ProtocolError> {
    if reporter.is_empty() { return Err(ProtocolError::InvalidAddress); }
    ProtocolEvent::ComplianceAlert(user, code).emit(&env);
    Ok(())
}

// ---- Market Making: params & incentives ----
pub fn set_mm_params(env: Env, caller: String, spread_bps: i128, inventory_cap: i128) -> Result<(), ProtocolError> {
    let caller_addr = Address::from_string(&caller);
    ProtocolConfig::require_admin(&env, &caller_addr)?;
    if spread_bps < 0 || inventory_cap < 0 { return Err(ProtocolError::InvalidInput); }
    AssetRegistryStorage::save_mm_params(&env, spread_bps, inventory_cap);
    ProtocolEvent::MMParamsUpdated(spread_bps, inventory_cap).emit(&env);
    Ok(())
}

pub fn get_mm_params(env: Env) -> (i128, i128) { AssetRegistryStorage::get_mm_params(&env) }

pub fn accrue_mm_incentive(env: Env, user: Address, amount: i128) -> Result<(), ProtocolError> {
    if amount <= 0 { return Err(ProtocolError::InvalidAmount); }
    ProtocolEvent::MMIncentiveAccrued(user, amount).emit(&env);
    Ok(())
}

// ---- Integration/API: webhook registry and basic views ----
pub fn register_webhook(env: Env, caller: String, topic: Symbol, target: Address) -> Result<(), ProtocolError> {
    let caller_addr = Address::from_string(&caller);
    ProtocolConfig::require_admin(&env, &caller_addr)?;
    let mut map = AssetRegistryStorage::get_webhooks(&env);
    map.set(topic.clone(), target.clone());
    AssetRegistryStorage::put_webhooks(&env, &map);
    ProtocolEvent::WebhookRegistered(target, topic).emit(&env);
    Ok(())
}

pub fn get_system_overview(env: Env) -> (i128, i128, i128, i128) {
    get_system_stats(env).unwrap_or((0,0,0,0))
}

// ---- Security: logging & audit ----
pub fn log_bug_report(env: Env, reporter: String, code: Symbol) -> Result<(), ProtocolError> {
    if reporter.is_empty() { return Err(ProtocolError::InvalidAddress); }
    let reporter_addr = Address::from_string(&reporter);
    ProtocolEvent::BugReportLogged(reporter_addr, code).emit(&env);
    Ok(())
}

pub fn log_audit_event(env: Env, action: Symbol, reference: Symbol) {
    ProtocolEvent::AuditTrail(action, reference).emit(&env);
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

// ---- Fees: dynamic/tiered configuration and computation ----
pub fn set_fees(env: Env, caller: String, base_bps: i128, tier1_bps: i128) -> Result<(), ProtocolError> {
    let caller_addr = Address::from_string(&caller);
    ProtocolConfig::require_admin(&env, &caller_addr)?;
    if base_bps < 0 || tier1_bps < 0 { return Err(ProtocolError::InvalidInput); }
    AssetRegistryStorage::save_fees(&env, base_bps, tier1_bps);
    ProtocolEvent::FeesUpdated(base_bps, tier1_bps).emit(&env);
    Ok(())
}

pub fn get_fees(env: Env) -> (i128, i128) { AssetRegistryStorage::get_fees(&env) }

pub fn compute_user_fee_bps(env: Env, user: Address, utilization_bps: i128, activity_score: i128) -> i128 {
    let (base, tier1) = AssetRegistryStorage::get_fees(&env);
    let util_adj = utilization_bps / 100; // simple 1% of util
    let tier_adj = if activity_score > 100 { -tier1 } else { 0 };
    let mut fee = base + util_adj + tier_adj;
    if fee < 0 { fee = 0; }
    fee
}

// ---- Insurance & Safety ----
pub fn set_insurance_params(env: Env, caller: String, premium_bps: i128, coverage_cap: i128) -> Result<(), ProtocolError> {
    let caller_addr = Address::from_string(&caller);
    ProtocolConfig::require_admin(&env, &caller_addr)?;
    AssetRegistryStorage::save_insurance(&env, premium_bps, coverage_cap);
    ProtocolEvent::InsuranceParamsUpdated(premium_bps, coverage_cap).emit(&env);
    Ok(())
}
pub fn get_insurance_params(env: Env) -> (i128, i128) { AssetRegistryStorage::get_insurance(&env) }
pub fn set_circuit_breaker(env: Env, caller: String, flag: bool) -> Result<(), ProtocolError> {
    let caller_addr = Address::from_string(&caller);
    ProtocolConfig::require_admin(&env, &caller_addr)?;
    AssetRegistryStorage::set_breaker(&env, flag);
    ProtocolEvent::CircuitBreaker(flag).emit(&env);
    Ok(())
}
pub fn is_circuit_breaker(env: Env) -> bool { AssetRegistryStorage::get_breaker(&env) }
pub fn file_insurance_claim(env: Env, user: String, amount: i128, reason: Symbol) -> Result<(), ProtocolError> {
    if user.is_empty() || amount <= 0 { return Err(ProtocolError::InvalidInput); }
    let user_addr = Address::from_string(&user);
    ProtocolEvent::ClaimFiled(user_addr, amount, reason).emit(&env);
    Ok(())
}

// ---- Advanced Liquidation: Auction Scaffold ----
pub fn start_liquidation_auction(env: Env, caller: String, user: Address, asset: Address, debt_portion: i128) -> Result<(), ProtocolError> {
    let caller_addr = Address::from_string(&caller);
    // Allow anyone to start if position is eligible
    let mut book = AssetRegistryStorage::get_auction_book(&env);
    if book.get(user.clone()).is_some() { return Err(ProtocolError::InvalidOperation); }
    let auction = LiquidationAuction { user: user.clone(), asset: asset.clone(), debt_portion, highest_bid: 0, highest_bidder: None, start_time: env.ledger().timestamp() };
    book.set(user.clone(), auction);
    AssetRegistryStorage::put_auction_book(&env, &book);
    ProtocolEvent::AuctionStarted(user, asset, debt_portion).emit(&env);
    Ok(())
}

pub fn place_liquidation_bid(env: Env, bidder: String, user: Address, amount: i128) -> Result<(), ProtocolError> {
    if bidder.is_empty() { return Err(ProtocolError::InvalidAddress); }
    if amount <= 0 { return Err(ProtocolError::InvalidAmount); }
    let bidder_addr = Address::from_string(&bidder);
    let mut book = AssetRegistryStorage::get_auction_book(&env);
    let mut auction = book.get(user.clone()).ok_or(ProtocolError::NotFound)?;
    if amount <= auction.highest_bid { return Err(ProtocolError::InvalidOperation); }
    auction.highest_bid = amount;
    auction.highest_bidder = Some(bidder_addr.clone());
    book.set(user.clone(), auction);
    AssetRegistryStorage::put_auction_book(&env, &book);
    ProtocolEvent::AuctionBidPlaced(bidder_addr, user, amount).emit(&env);
    Ok(())
}

pub fn settle_liquidation_auction(env: Env, caller: String, user: Address) -> Result<(), ProtocolError> {
    let _caller_addr = Address::from_string(&caller);
    let mut book = AssetRegistryStorage::get_auction_book(&env);
    let auction = book.get(user.clone()).ok_or(ProtocolError::NotFound)?;
    let winner = auction.highest_bidder.ok_or(ProtocolError::NotFound)?;
    let seized = auction.highest_bid; // placeholder
    let repaid = auction.debt_portion;
    book.remove(user.clone());
    AssetRegistryStorage::put_auction_book(&env, &book);
    ProtocolEvent::AuctionSettled(winner, user, seized, repaid).emit(&env);
    Ok(())
}

// Liquidation queue helpers
pub fn enqueue_for_liquidation(env: Env, user: Address) {
    let mut q = AssetRegistryStorage::get_liq_queue(&env);
    q.push_back(user);
    AssetRegistryStorage::put_liq_queue(&env, &q);
}
pub fn dequeue_liquidation(env: Env) -> Option<Address> {
    let mut q = AssetRegistryStorage::get_liq_queue(&env);
    if q.len() == 0 { return None; }
    let head = q.get(0);
    // rebuild without head (simple O(n))
    let mut nq: Vec<Address> = Vec::new(&env);
    for i in 1..q.len() { nq.push_back(q.get(i).unwrap()); }
    AssetRegistryStorage::put_liq_queue(&env, &nq);
    head
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
    ProtocolEvent::UserRiskUpdated(user_addr.clone(), st.score, st.credit_limit_value).emit(&env);
    // Alert when score exceeds threshold (placeholder)
    if st.score > 800 { ProtocolEvent::RiskAlert(user_addr, st.score).emit(&env); }
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
        let _ = get_asset_price(&env, &asset)?;
        let initiator_addr = Address::from_string(&initiator);
        let bps = ProtocolConfig::get_flash_loan_fee_bps(&env);
        FlashLoan::execute(&env, &initiator_addr, &asset, amount, bps, &receiver_contract)
    })();
    ReentrancyGuard::exit(&env);
    result
}

// --- Cross-Chain Bridge Operations ---
fn bridge_require_network(env: &Env, network_id: &String) -> Result<BridgeConfig, ProtocolError> {
    let cfg = BridgeStorage::get(env, network_id).ok_or(ProtocolError::NotFound)?;
    if !cfg.enabled { return Err(ProtocolError::InvalidOperation); }
    Ok(cfg)
}

pub fn register_bridge_admin(env: Env, caller: String, network_id: String, bridge: Address, fee_bps: i128) -> Result<(), ProtocolError> {
    let caller_addr = Address::from_string(&caller);
    ProtocolConfig::require_admin(&env, &caller_addr)?;
    if network_id.is_empty() { return Err(ProtocolError::InvalidInput); }
    if fee_bps < 0 || fee_bps > 10000 { return Err(ProtocolError::InvalidInput); }
    let mut reg = BridgeStorage::get_registry(&env);
    if reg.contains_key(network_id.clone()) { return Err(ProtocolError::AlreadyExists); }
    let cfg = BridgeConfig::new(network_id.clone(), bridge.clone(), fee_bps);
    reg.set(network_id.clone(), cfg);
    BridgeStorage::put_registry(&env, &reg);
    ProtocolEvent::BridgeRegistered(network_id, bridge, fee_bps).emit(&env);
    Ok(())
}

pub fn set_bridge_fee_admin(env: Env, caller: String, network_id: String, fee_bps: i128) -> Result<(), ProtocolError> {
    let caller_addr = Address::from_string(&caller);
    ProtocolConfig::require_admin(&env, &caller_addr)?;
    if fee_bps < 0 || fee_bps > 10000 { return Err(ProtocolError::InvalidInput); }
    let mut reg = BridgeStorage::get_registry(&env);
    let mut cfg = reg.get(network_id.clone()).ok_or(ProtocolError::NotFound)?;
    cfg.fee_bps = fee_bps;
    reg.set(network_id.clone(), cfg);
    BridgeStorage::put_registry(&env, &reg);
    ProtocolEvent::BridgeFeeUpdated(network_id, fee_bps).emit(&env);
    Ok(())
}

pub fn bridge_in(env: Env, user: String, network_id: String, asset: Address, amount: i128) -> Result<i128, ProtocolError> {
    ReentrancyGuard::enter(&env)?;
    let result = (|| {
        if user.is_empty() { return Err(ProtocolError::InvalidAddress); }
        ensure_amount_positive(amount)?;
        let cfg = bridge_require_network(&env, &network_id)?;
        let user_addr = Address::from_string(&user);
        let mut pos = CrossStateHelper::get_or_init_position(&env, &user_addr);
        let cur = pos.collateral.get(asset.clone()).unwrap_or(0);
        let fee = amount * cfg.fee_bps / 10000;
        let net = amount - fee;
        pos.collateral.set(asset.clone(), cur + net);
        CrossStateHelper::save_position(&env, &pos);
        ProtocolEvent::AssetBridgedIn(user_addr, network_id, asset, amount, fee).emit(&env);
        Ok(fee)
    })();
    ReentrancyGuard::exit(&env);
    result
}

pub fn bridge_out(env: Env, user: String, network_id: String, asset: Address, amount: i128) -> Result<i128, ProtocolError> {
    ReentrancyGuard::enter(&env)?;
    let result = (|| {
        if user.is_empty() { return Err(ProtocolError::InvalidAddress); }
        ensure_amount_positive(amount)?;
        let cfg = bridge_require_network(&env, &network_id)?;
        let user_addr = Address::from_string(&user);
        let mut pos = CrossStateHelper::get_or_init_position(&env, &user_addr);
        let cur = pos.collateral.get(asset.clone()).unwrap_or(0);
        if cur < amount { return Err(ProtocolError::InsufficientCollateral); }
        let fee = amount * cfg.fee_bps / 10000;
        let net = amount - fee;
        pos.collateral.set(asset.clone(), cur - amount);
        CrossStateHelper::save_position(&env, &pos);
        ProtocolEvent::AssetBridgedOut(user_addr, network_id, asset, net, fee).emit(&env);
        Ok(fee)
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
    pub fn set_asset_correlation(env: Env, caller: String, a: Address, b: Address, corr_bps: i128) -> Result<(), ProtocolError> {
        set_asset_correlation(env, caller, a, b, corr_bps)
    }
    pub fn get_portfolio_risk_ratio(env: Env, user: String) -> Result<i128, ProtocolError> {
        get_portfolio_risk_ratio(env, user)
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
    pub fn register_webhook(env: Env, caller: String, topic: Symbol, target: Address) -> Result<(), ProtocolError> {
        register_webhook(env, caller, topic, target)
    }
    pub fn get_system_overview(env: Env) -> (i128, i128, i128, i128) { get_system_overview(env) }
    pub fn log_bug_report(env: Env, reporter: String, code: Symbol) -> Result<(), ProtocolError> { log_bug_report(env, reporter, code) }
    pub fn log_audit_event(env: Env, action: Symbol, reference: Symbol) { log_audit_event(env, action, reference) }

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
    pub fn oracle_set_mode(env: Env, caller: String, mode: i128) -> Result<(), ProtocolError> {
        let caller_addr = Address::from_string(&caller);
        ProtocolConfig::require_admin(&env, &caller_addr)?;
        OracleStorage::set_mode(&env, mode);
        Ok(())
    }

    // Compliance entrypoints
    pub fn set_kyc_status(env: Env, caller: String, user: Address, status: bool) -> Result<(), ProtocolError> {
        set_kyc_status(env, caller, user, status)
    }
    pub fn get_kyc_status(env: Env, user: Address) -> bool { get_kyc_status(env, user) }
    pub fn report_compliance_event(env: Env, reporter: String, user: Address, code: Symbol) -> Result<(), ProtocolError> {
        report_compliance_event(env, reporter, user, code)
    }
    // MM
    pub fn set_mm_params(env: Env, caller: String, spread_bps: i128, inventory_cap: i128) -> Result<(), ProtocolError> {
        set_mm_params(env, caller, spread_bps, inventory_cap)
    }
    pub fn get_mm_params(env: Env) -> (i128, i128) { get_mm_params(env) }
    pub fn accrue_mm_incentive(env: Env, user: Address, amount: i128) -> Result<(), ProtocolError> { accrue_mm_incentive(env, user, amount) }

    // Token transfer admin controls
    pub fn set_enforce_transfers(env: Env, caller: String, flag: bool) -> Result<(), ProtocolError> {
        let caller_addr = Address::from_string(&caller);
        ProtocolConfig::require_admin(&env, &caller_addr)?;
        AssetRegistryStorage::set_enforce_transfers(&env, flag);
        Ok(())
    }
    pub fn set_base_token(env: Env, caller: String, token: Address) -> Result<(), ProtocolError> {
        let caller_addr = Address::from_string(&caller);
        ProtocolConfig::require_admin(&env, &caller_addr)?;
        AssetRegistryStorage::set_base_token(&env, &token);
        Ok(())
    }
    // Insurance & Safety
    pub fn set_insurance_params(env: Env, caller: String, premium_bps: i128, coverage_cap: i128) -> Result<(), ProtocolError> { set_insurance_params(env, caller, premium_bps, coverage_cap) }
    pub fn get_insurance_params(env: Env) -> (i128, i128) { get_insurance_params(env) }
    pub fn set_circuit_breaker(env: Env, caller: String, flag: bool) -> Result<(), ProtocolError> { set_circuit_breaker(env, caller, flag) }
    pub fn is_circuit_breaker(env: Env) -> bool { is_circuit_breaker(env) }
    pub fn file_insurance_claim(env: Env, user: String, amount: i128, reason: Symbol) -> Result<(), ProtocolError> { file_insurance_claim(env, user, amount, reason) }
    // Liquidation queue
    pub fn enqueue_for_liquidation(env: Env, user: Address) { enqueue_for_liquidation(env, user) }
    pub fn dequeue_liquidation(env: Env) -> Option<Address> { dequeue_liquidation(env) }
    // Fees
    pub fn set_fees(env: Env, caller: String, base_bps: i128, tier1_bps: i128) -> Result<(), ProtocolError> { set_fees(env, caller, base_bps, tier1_bps) }
    pub fn get_fees(env: Env) -> (i128, i128) { get_fees(env) }
    pub fn compute_user_fee_bps(env: Env, user: Address, utilization_bps: i128, activity_score: i128) -> i128 { compute_user_fee_bps(env, user, utilization_bps, activity_score) }

    // Governance entrypoints
    pub fn gov_set_quorum_bps(env: Env, caller: String, bps: i128) -> Result<(), ProtocolError> {
        let caller_addr = Address::from_string(&caller);
        ProtocolConfig::require_admin(&env, &caller_addr)?;
        GovStorage::set_quorum_bps(&env, bps);
        Ok(())
    }
    pub fn gov_set_timelock(env: Env, caller: String, secs: u64) -> Result<(), ProtocolError> {
        let caller_addr = Address::from_string(&caller);
        ProtocolConfig::require_admin(&env, &caller_addr)?;
        GovStorage::set_timelock(&env, secs);
        Ok(())
    }
    pub fn gov_propose(env: Env, proposer: String, title: String, voting_period_secs: u64) -> Proposal {
        let proposer_addr = Address::from_string(&proposer);
        Governance::propose(&env, &proposer_addr, title, voting_period_secs)
    }
    pub fn gov_vote(env: Env, id: u64, voter: String, support: bool, weight: i128) -> Proposal {
        let voter_addr = Address::from_string(&voter);
        Governance::vote(&env, id, &voter_addr, support, weight)
    }
    pub fn gov_queue(env: Env, id: u64) -> Proposal { Governance::queue(&env, id) }
    pub fn gov_execute(env: Env, id: u64) -> Proposal { Governance::execute(&env, id) }
    pub fn gov_delegate(env: Env, from: String, to: String) {
        let from_addr = Address::from_string(&from);
        let to_addr = Address::from_string(&to);
        Governance::delegate(&env, &from_addr, &to_addr);
    }
    pub fn gov_get_delegate(env: Env, from: String) -> Option<Address> {
        let from_addr = Address::from_string(&from);
        Governance::get_delegate(&env, &from_addr)
    }

    // Bridge admin/user entrypoints
    pub fn register_bridge(env: Env, caller: String, network_id: String, bridge: Address, fee_bps: i128) -> Result<(), ProtocolError> {
        register_bridge_admin(env, caller, network_id, bridge, fee_bps)
    }
    pub fn set_bridge_fee(env: Env, caller: String, network_id: String, fee_bps: i128) -> Result<(), ProtocolError> {
        set_bridge_fee_admin(env, caller, network_id, fee_bps)
    }
    pub fn bridge_deposit(env: Env, user: String, network_id: String, asset: Address, amount: i128) -> Result<i128, ProtocolError> {
        bridge_in(env, user, network_id, asset, amount)
    }
    pub fn bridge_withdraw(env: Env, user: String, network_id: String, asset: Address, amount: i128) -> Result<i128, ProtocolError> {
        bridge_out(env, user, network_id, asset, amount)
    }
    pub fn get_bridge_config(env: Env, network_id: String) -> Option<(Address, i128, bool)> {
        BridgeStorage::get(&env, &network_id).map(|c| (c.bridge, c.fee_bps, c.enabled))
    }
    pub fn list_bridges(env: Env) -> Vec<String> {
        let reg = BridgeStorage::get_registry(&env);
        let mut out = Vec::new(&env);
        for (k, _) in reg.iter() { out.push_back(k); }
        out
    }

    // Upgrades
    pub fn upgrade_propose(env: Env, caller: String, new_version: u32, code_hash: String) -> Result<(), ProtocolError> {
        upgrade_propose(env, caller, new_version, code_hash)
    }
    pub fn upgrade_approve(env: Env, caller: String) -> Result<(), ProtocolError> {
        upgrade_approve(env, caller)
    }
    pub fn upgrade_execute(env: Env, caller: String) -> Result<u32, ProtocolError> {
        upgrade_execute(env, caller)
    }
    pub fn upgrade_rollback(env: Env, caller: String) -> Result<u32, ProtocolError> {
        upgrade_rollback(env, caller)
    }
    pub fn upgrade_status(env: Env) -> (u32, u32, u32, String, bool, u64) {
        upgrade_status(env)
    }

    // Data management entrypoints
    pub fn data_save(env: Env, name: Symbol, version: u32, data: Bytes, compress: bool) -> Result<(), ProtocolError> {
        data_save(env, name, version, data, compress)
    }
    pub fn data_load(env: Env, name: Symbol) -> Result<(u32, Bytes), ProtocolError> { data_load(env, name) }
    pub fn data_backup(env: Env, name: Symbol) -> Result<(), ProtocolError> { data_backup(env, name) }
    pub fn data_restore(env: Env, name: Symbol) -> Result<(), ProtocolError> { data_restore(env, name) }
    pub fn data_migrate_bump_version(env: Env, name: Symbol, new_version: u32) -> Result<(), ProtocolError> { data_migrate_bump_version(env, name, new_version) }

    // Social recovery entrypoints
    pub fn set_guardians(env: Env, user: String, guardians: Vec<Address>) -> Result<(), ProtocolError> {
        set_guardians(env, user, guardians)
    }
    pub fn start_recovery(env: Env, guardian: String, user: String, new_address: Address, delay_secs: u64) -> Result<(), ProtocolError> {
        start_recovery(env, guardian, user, new_address, delay_secs)
    }
    pub fn approve_recovery(env: Env, guardian: String, user: String) -> Result<(), ProtocolError> {
        approve_recovery(env, guardian, user)
    }
    pub fn execute_recovery(env: Env, user: String, min_approvals: i128) -> Result<Address, ProtocolError> {
        execute_recovery(env, user, min_approvals)
    }

    // MultiSig entrypoints
    pub fn ms_set_admins(env: Env, caller: String, admins: Vec<Address>, threshold: i128) -> Result<(), ProtocolError> {
        ms_set_admins(env, caller, admins, threshold)
    }
    pub fn ms_propose_set_min_cr(env: Env, caller: String, ratio: i128) -> Result<u64, ProtocolError> {
        ms_propose_set_min_cr(env, caller, ratio)
    }
    pub fn ms_approve(env: Env, caller: String, id: u64) -> Result<(), ProtocolError> {
        ms_approve(env, caller, id)
    }
    pub fn ms_execute(env: Env, id: u64) -> Result<(), ProtocolError> {
        ms_execute(env, id)
    }
}

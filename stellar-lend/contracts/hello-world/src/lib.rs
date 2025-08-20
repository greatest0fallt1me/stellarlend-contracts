//! StellarLend Soroban Smart Contract
//
//! This contract provides the foundation for the StellarLend DeFi Lending & Borrowing Protocol.
//! Core features will be implemented incrementally in separate modules.

#![no_std]
extern crate alloc;

use alloc::format;
use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, Address, Env,
    String, Symbol,
};

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
}

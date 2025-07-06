//! StellarLend Soroban Smart Contract
//
//! This contract provides the foundation for the StellarLend DeFi Lending & Borrowing Protocol.
//! Core features will be implemented incrementally in separate modules.

#![no_std]
use soroban_sdk::{contract, contractimpl, vec, Env, String, Vec, Symbol, Address, storage, contracttype, contracterror, IntoVal};

// Module placeholders for future expansion
// mod deposit;
// mod borrow;
// mod repay;
// mod withdraw;
// mod liquidate;

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
}

impl Position {
    /// Create a new position
    pub fn new(user: Address, collateral: i128, debt: i128) -> Self {
        Self { user, collateral, debt }
    }
}

/// Helper functions for state management
pub struct StateHelper;

impl StateHelper {
    /// Save a position to storage
    pub fn save_position(env: &Env, position: &Position) {
        let key = (Symbol::short("position"), position.user.clone());
        env.storage().instance().set(&key, position);
    }

    /// Retrieve a position from storage
    pub fn get_position(env: &Env, user: &Address) -> Option<Position> {
        let key = (Symbol::short("position"), user.clone());
        env.storage().instance().get(&key)
    }

    /// Remove a position from storage
    pub fn remove_position(env: &Env, user: &Address) {
        let key = (Symbol::short("position"), user.clone());
        env.storage().instance().remove(&key);
    }

    /// Calculate the collateral ratio for a position (collateral / debt, scaled by 100 for percent)
    pub fn collateral_ratio(position: &Position) -> i128 {
        if position.debt == 0 {
            return i128::MAX; // Infinite ratio if no debt
        }
        // Ratio as percent (e.g., 150 means 150%)
        (position.collateral * 100) / position.debt
    }

    /// Calculate the dynamic collateral ratio for a position using price oracle
    /// (collateral * price) / debt, scaled by 100 for percent
    pub fn dynamic_collateral_ratio<P: PriceOracle>(env: &Env, position: &Position) -> i128 {
        if position.debt == 0 {
            return i128::MAX;
        }
        let price = P::get_price(env); // price is scaled by 1e8
        // Ratio as percent (e.g., 150 means 150%)
        ((position.collateral * price * 100) / 100_000_000) / position.debt
    }
}

/// Event types for protocol actions
pub enum ProtocolEvent {
    Deposit { user: String, amount: i128 },
    Borrow { user: String, amount: i128 },
    Repay { user: String, amount: i128 },
    Withdraw { user: String, amount: i128 },
    Liquidate { user: String, amount: i128 },
}

impl ProtocolEvent {
    /// Emit the event using Soroban's event system
    pub fn emit(&self, env: &Env) {
        match self {
            ProtocolEvent::Deposit { user, amount } => {
                env.events().publish((Symbol::short("deposit"), Symbol::short("user")), (Symbol::short("user"), *amount));
            }
            ProtocolEvent::Borrow { user, amount } => {
                env.events().publish((Symbol::short("borrow"), Symbol::short("user")), (Symbol::short("user"), *amount));
            }
            ProtocolEvent::Repay { user, amount } => {
                env.events().publish((Symbol::short("repay"), Symbol::short("user")), (Symbol::short("user"), *amount));
            }
            ProtocolEvent::Withdraw { user, amount } => {
                env.events().publish((Symbol::short("withdraw"), Symbol::short("user")), (Symbol::short("user"), *amount));
            }
            ProtocolEvent::Liquidate { user, amount } => {
                env.events().publish((Symbol::short("liquidate"), Symbol::short("user")), (Symbol::short("user"), *amount));
            }
        }
    }
}

/// Trait for price oracle integration
pub trait PriceOracle {
    /// Returns the price of the collateral asset in terms of the debt asset (scaled by 1e8)
    fn get_price(env: &Env) -> i128;
}

/// Mock implementation of the price oracle
pub struct MockOracle;

impl PriceOracle for MockOracle {
    fn get_price(_env: &Env) -> i128 {
        // For demo: 1 collateral = 2 debt (price = 2e8)
        200_000_000 // 2.0 * 1e8
    }
}

/// Protocol configuration and admin management
pub struct ProtocolConfig;

impl ProtocolConfig {
    /// Storage key for admin address
    fn admin_key() -> Symbol { Symbol::short("admin") }
    /// Storage key for oracle address
    fn oracle_key() -> Symbol { Symbol::short("oracle") }
    /// Storage key for min collateral ratio
    fn min_collateral_ratio_key() -> Symbol { Symbol::short("min_ratio") }

    /// Set the admin address (only callable once)
    pub fn set_admin(env: &Env, admin: &Address) {
        if env.storage().instance().has(&Self::admin_key()) {
            panic!("Admin already set");
        }
        env.storage().instance().set(&Self::admin_key(), admin);
    }

    /// Get the admin address
    pub fn get_admin(env: &Env) -> Address {
        env.storage().instance().get::<Symbol, Address>(&Self::admin_key()).expect("Admin not set")
    }

    /// Require that the caller is admin
    pub fn require_admin(env: &Env, caller: &Address) -> Result<(), ProtocolError> {
        let admin = Self::get_admin(env);
        if &admin != caller {
            return Err(ProtocolError::NotAdmin);
        }
        Ok(())
    }

    /// Set the oracle address (admin only)
    pub fn set_oracle(env: &Env, caller: &Address, oracle: &Address) -> Result<(), ProtocolError> {
        Self::require_admin(env, caller)?;
        env.storage().instance().set(&Self::oracle_key(), oracle);
        Ok(())
    }

    /// Get the oracle address
    pub fn get_oracle(env: &Env) -> Address {
        env.storage().instance().get::<Symbol, Address>(&Self::oracle_key()).expect("Oracle not set")
    }

    /// Set the minimum collateral ratio (admin only)
    pub fn set_min_collateral_ratio(env: &Env, caller: &Address, ratio: i128) -> Result<(), ProtocolError> {
        Self::require_admin(env, caller)?;
        env.storage().instance().set(&Self::min_collateral_ratio_key(), &ratio);
        Ok(())
    }

    /// Get the minimum collateral ratio
    pub fn get_min_collateral_ratio(env: &Env) -> i128 {
        env.storage().instance().get::<Symbol, i128>(&Self::min_collateral_ratio_key()).unwrap_or(150)
    }
}

/// Custom error type for protocol errors
#[contracterror]
#[derive(Debug, Eq, PartialEq)]
pub enum ProtocolError {
    Unauthorized = 1,
    InsufficientCollateral = 2,
    InsufficientCollateralRatio = 3,
    InvalidAmount = 4,
    InvalidAddress = 5,
    PositionNotFound = 6,
    AlreadyInitialized = 7,
    NotAdmin = 8,
    OracleNotSet = 9,
    AdminNotSet = 10,
    NotEligibleForLiquidation = 11,
    Unknown = 12,
}

impl ProtocolError {
    pub fn to_str(&self) -> &'static str {
        match self {
            ProtocolError::Unauthorized => "Unauthorized",
            ProtocolError::InsufficientCollateral => "InsufficientCollateral",
            ProtocolError::InsufficientCollateralRatio => "InsufficientCollateralRatio",
            ProtocolError::InvalidAmount => "InvalidAmount",
            ProtocolError::InvalidAddress => "InvalidAddress",
            ProtocolError::PositionNotFound => "PositionNotFound",
            ProtocolError::AlreadyInitialized => "AlreadyInitialized",
            ProtocolError::NotAdmin => "NotAdmin",
            ProtocolError::OracleNotSet => "OracleNotSet",
            ProtocolError::AdminNotSet => "AdminNotSet",
            ProtocolError::NotEligibleForLiquidation => "NotEligibleForLiquidation",
            ProtocolError::Unknown => "Unknown",
        }
    }
}

// This is a sample contract. Replace this placeholder with your own contract logic.
// A corresponding test example is available in `test.rs`.
//
// For comprehensive examples, visit <https://github.com/stellar/soroban-examples>.
// The repository includes use cases for the Stellar ecosystem, such as data storage on
// the blockchain, token swaps, liquidity pools, and more.
//
// Refer to the official documentation:
// <https://developers.stellar.org/docs/build/smart-contracts/overview>.
#[contractimpl]
impl Contract {
    /// Initializes the contract and sets the admin address
    pub fn initialize(env: Env, admin: String) -> Result<(), ProtocolError> {
        let admin_addr = Address::from_string(&admin);
        if env.storage().instance().has(&ProtocolConfig::admin_key()) {
            return Err(ProtocolError::AlreadyInitialized);
        }
        ProtocolConfig::set_admin(&env, &admin_addr);
        Ok(())
    }

    /// Set the oracle address (admin only)
    pub fn set_oracle(env: Env, caller: String, oracle: String) -> Result<(), ProtocolError> {
        let caller_addr = Address::from_string(&caller);
        let oracle_addr = Address::from_string(&oracle);
        ProtocolConfig::set_oracle(&env, &caller_addr, &oracle_addr)?;
        Ok(())
    }

    /// Set the minimum collateral ratio (admin only)
    pub fn set_min_collateral_ratio(env: Env, caller: String, ratio: i128) -> Result<(), ProtocolError> {
        let caller_addr = Address::from_string(&caller);
        ProtocolConfig::set_min_collateral_ratio(&env, &caller_addr, ratio)?;
        Ok(())
    }

    /// Minimum collateral ratio required (e.g., 150%)
    const MIN_COLLATERAL_RATIO: i128 = 150;

    // --- Core Protocol Function Placeholders ---

    /// Deposit collateral into the protocol
    pub fn deposit_collateral(env: Env, depositor: String, amount: i128) -> Result<(), ProtocolError> {
        if depositor.is_empty() {
            return Err(ProtocolError::InvalidAddress);
        }
        if amount <= 0 {
            return Err(ProtocolError::InvalidAmount);
        }
        let depositor_addr = Address::from_string(&depositor);
        let mut position = StateHelper::get_position(&env, &depositor_addr)
            .unwrap_or(Position::new(depositor_addr.clone(), 0, 0));
        position.collateral += amount;
        StateHelper::save_position(&env, &position);
        ProtocolEvent::Deposit { user: depositor, amount }.emit(&env);
        Ok(())
    }

    /// Borrow assets from the protocol with dynamic risk check
    pub fn borrow(env: Env, borrower: String, amount: i128) -> Result<(), ProtocolError> {
        if borrower.is_empty() {
            return Err(ProtocolError::InvalidAddress);
        }
        if amount <= 0 {
            return Err(ProtocolError::InvalidAmount);
        }
        let borrower_addr = Address::from_string(&borrower);
        let mut position = StateHelper::get_position(&env, &borrower_addr)
            .unwrap_or(Position::new(borrower_addr.clone(), 0, 0));
        let new_debt = position.debt + amount;
        let mut new_position = position.clone();
        new_position.debt = new_debt;
        let min_ratio = ProtocolConfig::get_min_collateral_ratio(&env);
        let ratio = StateHelper::dynamic_collateral_ratio::<MockOracle>(&env, &new_position);
        if ratio < min_ratio {
            return Err(ProtocolError::InsufficientCollateralRatio);
        }
        position.debt = new_debt;
        StateHelper::save_position(&env, &position);
        ProtocolEvent::Borrow { user: borrower, amount }.emit(&env);
        Ok(())
    }

    /// Repay borrowed assets
    pub fn repay(env: Env, repayer: String, amount: i128) -> Result<(), ProtocolError> {
        if repayer.is_empty() {
            return Err(ProtocolError::InvalidAddress);
        }
        if amount <= 0 {
            return Err(ProtocolError::InvalidAmount);
        }
        let repayer_addr = Address::from_string(&repayer);
        let mut position = StateHelper::get_position(&env, &repayer_addr)
            .unwrap_or(Position::new(repayer_addr.clone(), 0, 0));
        position.debt = (position.debt - amount).max(0);
        StateHelper::save_position(&env, &position);
        ProtocolEvent::Repay { user: repayer, amount }.emit(&env);
        Ok(())
    }

    /// Withdraw collateral with dynamic risk check
    pub fn withdraw(env: Env, withdrawer: String, amount: i128) -> Result<(), ProtocolError> {
        if withdrawer.is_empty() {
            return Err(ProtocolError::InvalidAddress);
        }
        if amount <= 0 {
            return Err(ProtocolError::InvalidAmount);
        }
        let withdrawer_addr = Address::from_string(&withdrawer);
        let mut position = StateHelper::get_position(&env, &withdrawer_addr)
            .unwrap_or(Position::new(withdrawer_addr.clone(), 0, 0));
        if position.collateral < amount {
            return Err(ProtocolError::InsufficientCollateral);
        }
        let mut new_position = position.clone();
        new_position.collateral -= amount;
        let min_ratio = ProtocolConfig::get_min_collateral_ratio(&env);
        let ratio = StateHelper::dynamic_collateral_ratio::<MockOracle>(&env, &new_position);
        if position.debt > 0 && ratio < min_ratio {
            return Err(ProtocolError::InsufficientCollateralRatio);
        }
        position.collateral = new_position.collateral;
        StateHelper::save_position(&env, &position);
        ProtocolEvent::Withdraw { user: withdrawer, amount }.emit(&env);
        Ok(())
    }

    /// Liquidate undercollateralized positions using dynamic risk check
    pub fn liquidate(env: Env, liquidator: String, target: String, amount: i128) -> Result<(), ProtocolError> {
        if liquidator.is_empty() || target.is_empty() {
            return Err(ProtocolError::InvalidAddress);
        }
        if amount <= 0 {
            return Err(ProtocolError::InvalidAmount);
        }
        let target_addr = Address::from_string(&target);
        let mut position = match StateHelper::get_position(&env, &target_addr) {
            Some(pos) => pos,
            None => return Err(ProtocolError::PositionNotFound),
        };
        let min_ratio = ProtocolConfig::get_min_collateral_ratio(&env);
        let ratio = StateHelper::dynamic_collateral_ratio::<MockOracle>(&env, &position);
        if ratio >= min_ratio {
            return Err(ProtocolError::NotEligibleForLiquidation);
        }
        let repay_amount = amount.min(position.debt);
        position.debt -= repay_amount;
        let penalty = (position.collateral * 10) / 100;
        position.collateral = position.collateral.saturating_sub(penalty);
        StateHelper::save_position(&env, &position);
        ProtocolEvent::Liquidate { user: target, amount: repay_amount }.emit(&env);
        Ok(())
    }

    pub fn hello(env: Env, to: String) -> Vec<String> {
        vec![&env, String::from_str(&env, "Hello"), to]
    }

    /// Query a user's position (collateral, debt, dynamic ratio)
    pub fn get_position(env: Env, user: String) -> Result<(i128, i128, i128), ProtocolError> {
        let user_addr = Address::from_string(&user);
        let position = StateHelper::get_position(&env, &user_addr)
            .unwrap_or(Position::new(user_addr, 0, 0));
        let ratio = StateHelper::dynamic_collateral_ratio::<MockOracle>(&env, &position);
        Ok((position.collateral, position.debt, ratio))
    }

    /// Query protocol parameters (admin, oracle, min collateral ratio)
    pub fn get_protocol_params(env: Env) -> Result<(Address, Address, i128), ProtocolError> {
        let admin = ProtocolConfig::get_admin(&env);
        let oracle = ProtocolConfig::get_oracle(&env);
        let min_ratio = ProtocolConfig::get_min_collateral_ratio(&env);
        Ok((admin, oracle, min_ratio))
    }

    /// Query system-wide stats (total collateral, total debt)
    pub fn get_system_stats(_env: Env) -> Result<(i128, i128), ProtocolError> {
        Ok((0, 0))
    }

    /// Query event logs for a given user and event type (stub for off-chain indexer)
    ///
    /// # Parameters
    /// - `user`: The user address as a string
    /// - `event_type`: The event type as a string ("deposit", "borrow", "repay", "withdraw", "liquidate")
    ///
    /// # Returns
    /// A vector of (event_type, amount, block/tx info) tuples (stubbed)
    pub fn get_user_event_history(_env: Env, _user: String, _event_type: String) -> Result<Vec<(String, i128, String)>, ProtocolError> {
        // NOTE: Soroban contracts cannot query historical events on-chain.
        // This function is a stub for off-chain indexer integration.
        // In production, an off-chain service would index events and provide this data.
        Ok(Vec::new(&_env))
    }

    /// Fetch recent protocol events (stub for off-chain indexer)
    ///
    /// # Parameters
    /// - `limit`: The maximum number of events to return
    ///
    /// # Returns
    /// A vector of (event_type, user, amount, block/tx info) tuples (stubbed)
    pub fn get_recent_events(_env: Env, _limit: u32) -> Result<Vec<(String, String, i128, String)>, ProtocolError> {
        // NOTE: Soroban contracts cannot query historical events on-chain.
        // This function is a stub for off-chain indexer integration.
        // In production, an off-chain service would index events and provide this data.
        Ok(Vec::new(&_env))
    }

    /// Example: Document how to use off-chain indexers for event history
    ///
    /// # Note
    /// See the Soroban docs for event indexing: https://soroban.stellar.org/docs/learn/events
    ///
    /// # Example
    /// ```
    /// // Off-chain indexer would listen for events like:
    /// // env.events().publish((Symbol::short("deposit"), Symbol::short("user")), (Symbol::short("user"), amount));
    /// // and store them in a database for querying.
    /// ```

    pub fn event_indexer_example_doc() -> Result<(), ProtocolError> { Ok(()) }
}

mod test;

// Additional documentation and module expansion will be added as features are implemented.

// Add doc comments and placeholder for future event logic
// pub enum ProtocolEvent { ... }
// impl ProtocolEvent { ... }
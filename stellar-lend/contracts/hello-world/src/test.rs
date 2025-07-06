#![cfg(test)]

use super::*;
use soroban_sdk::{Address, Env, String};

/// Test utilities for creating test environments and addresses
pub struct TestUtils;

impl TestUtils {
    /// Create a test environment
    pub fn create_test_env() -> Env {
        let env = Env::default();
        env.mock_all_auths();
        env
    }

    /// Create a test address from a string
    pub fn create_test_address(env: &Env, address_str: &str) -> Address {
        Address::from_string(&String::from_str(env, address_str))
    }

    /// Create a test admin address
    pub fn create_admin_address(env: &Env) -> Address {
        Self::create_test_address(env, "GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF")
    }

    /// Create a test user address
    pub fn create_user_address(env: &Env, user_id: u32) -> Address {
        if user_id == 0 {
            Self::create_test_address(env, "GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF")
        } else {
            Self::create_test_address(env, "G1AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF")
        }
    }

    /// Create a test oracle address
    pub fn create_oracle_address(env: &Env) -> Address {
        Self::create_test_address(env, "GORACLEAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF")
    }

    /// Initialize the contract with test admin
    pub fn initialize_contract(env: &Env) -> Address {
        let admin = Self::create_admin_address(env);
        let contract_id = env.register(Contract, ());
        env.as_contract(&contract_id, || {
            Contract::initialize(env.clone(), admin.to_string()).unwrap();
        });
        admin
    }
}

/// Mock price oracle for testing
pub struct TestOracle;

impl PriceOracle for TestOracle {
    fn get_price(_env: &Env) -> i128 {
        // Test price: 1 collateral = 1.5 debt (price = 1.5e8)
        150_000_000 // 1.5 * 1e8
    }
}

#[test]
fn test_contract_initialization() {
    let env = TestUtils::create_test_env();
    let admin = TestUtils::create_admin_address(&env);
    
    let contract_id = env.register(Contract, ());
    env.as_contract(&contract_id, || {
        let result = Contract::initialize(env.clone(), admin.to_string());
        assert!(result.is_ok());
        
        // Test that admin is set correctly
        let (stored_admin, _, _) = Contract::get_protocol_params(env.clone()).unwrap();
        assert_eq!(stored_admin, admin);
    });
}

#[test]
fn test_contract_initialization_already_initialized() {
    let env = TestUtils::create_test_env();
    let admin = TestUtils::create_admin_address(&env);
    
    let contract_id = env.register(Contract, ());
    env.as_contract(&contract_id, || {
        // First initialization should succeed
        let result = Contract::initialize(env.clone(), admin.to_string());
        assert!(result.is_ok());
        
        // Second initialization should fail
        let result = Contract::initialize(env.clone(), admin.to_string());
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), ProtocolError::AlreadyInitialized);
    });
}

#[test]
fn test_deposit_collateral() {
    let env = TestUtils::create_test_env();
    let _admin = TestUtils::initialize_contract(&env);
    let user = TestUtils::create_user_address(&env, 1);
    
    let contract_id = env.register(Contract, ());
    env.as_contract(&contract_id, || {
        // Test successful deposit
        let result = Contract::deposit_collateral(env.clone(), user.to_string(), 1000);
        assert!(result.is_ok());
        
        // Verify position is updated
        let (collateral, debt, _ratio) = Contract::get_position(env.clone(), user.to_string()).unwrap();
        assert_eq!(collateral, 1000);
        assert_eq!(debt, 0);
    });
}

#[test]
fn test_deposit_collateral_invalid_amount() {
    let env = TestUtils::create_test_env();
    let _admin = TestUtils::initialize_contract(&env);
    let user = TestUtils::create_user_address(&env, 1);
    
    let contract_id = env.register(Contract, ());
    env.as_contract(&contract_id, || {
        // Test deposit with zero amount
        let result = Contract::deposit_collateral(env.clone(), user.to_string(), 0);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), ProtocolError::InvalidAmount);
        
        // Test deposit with negative amount
        let result = Contract::deposit_collateral(env.clone(), user.to_string(), -100);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), ProtocolError::InvalidAmount);
    });
}

#[test]
fn test_deposit_collateral_invalid_address() {
    let env = TestUtils::create_test_env();
    let _admin = TestUtils::initialize_contract(&env);
    
    let contract_id = env.register(Contract, ());
    env.as_contract(&contract_id, || {
        // Test deposit with empty address
        let result = Contract::deposit_collateral(env.clone(), String::from_str(&env, ""), 1000);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), ProtocolError::InvalidAddress);
    });
}

#[test]
fn test_borrow_success() {
    let env = TestUtils::create_test_env();
    let _admin = TestUtils::initialize_contract(&env);
    let user = TestUtils::create_user_address(&env, 1);
    
    let contract_id = env.register(Contract, ());
    env.as_contract(&contract_id, || {
        // First deposit collateral
        Contract::deposit_collateral(env.clone(), user.to_string(), 2000).unwrap();
        
        // Then borrow (should succeed with sufficient collateral)
        let result = Contract::borrow(env.clone(), user.to_string(), 1000);
        assert!(result.is_ok());
        
        // Verify position is updated
        let (collateral, debt, _ratio) = Contract::get_position(env.clone(), user.to_string()).unwrap();
        assert_eq!(collateral, 2000);
        assert_eq!(debt, 1000);
    });
}

#[test]
fn test_borrow_insufficient_collateral_ratio() {
    let env = TestUtils::create_test_env();
    let _admin = TestUtils::initialize_contract(&env);
    let user = TestUtils::create_user_address(&env, 1);
    
    let contract_id = env.register(Contract, ());
    env.as_contract(&contract_id, || {
        // Deposit small amount of collateral
        Contract::deposit_collateral(env.clone(), user.to_string(), 100).unwrap();
        
        // Try to borrow large amount (should fail)
        let result = Contract::borrow(env.clone(), user.to_string(), 1000);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), ProtocolError::InsufficientCollateralRatio);
    });
}

#[test]
fn test_repay_success() {
    let env = TestUtils::create_test_env();
    let _admin = TestUtils::initialize_contract(&env);
    let user = TestUtils::create_user_address(&env, 1);
    
    let contract_id = env.register(Contract, ());
    env.as_contract(&contract_id, || {
        // Setup: deposit and borrow
        Contract::deposit_collateral(env.clone(), user.to_string(), 2000).unwrap();
        Contract::borrow(env.clone(), user.to_string(), 1000).unwrap();
        
        // Repay part of the debt
        let result = Contract::repay(env.clone(), user.to_string(), 500);
        assert!(result.is_ok());
        
        // Verify position is updated
        let (collateral, debt, _ratio) = Contract::get_position(env.clone(), user.to_string()).unwrap();
        assert_eq!(collateral, 2000);
        assert_eq!(debt, 500);
    });
}

#[test]
fn test_repay_full_amount() {
    let env = TestUtils::create_test_env();
    let _admin = TestUtils::initialize_contract(&env);
    let user = TestUtils::create_user_address(&env, 1);
    
    let contract_id = env.register(Contract, ());
    env.as_contract(&contract_id, || {
        // Setup: deposit and borrow
        Contract::deposit_collateral(env.clone(), user.to_string(), 2000).unwrap();
        Contract::borrow(env.clone(), user.to_string(), 1000).unwrap();
        
        // Repay full amount
        let result = Contract::repay(env.clone(), user.to_string(), 1000);
        assert!(result.is_ok());
        
        // Verify debt is zero
        let (collateral, debt, _ratio) = Contract::get_position(env.clone(), user.to_string()).unwrap();
        assert_eq!(collateral, 2000);
        assert_eq!(debt, 0);
    });
}

#[test]
fn test_withdraw_success() {
    let env = TestUtils::create_test_env();
    let _admin = TestUtils::initialize_contract(&env);
    let user = TestUtils::create_user_address(&env, 1);
    
    let contract_id = env.register(Contract, ());
    env.as_contract(&contract_id, || {
        // Setup: deposit collateral
        Contract::deposit_collateral(env.clone(), user.to_string(), 2000).unwrap();
        
        // Withdraw part of collateral
        let result = Contract::withdraw(env.clone(), user.to_string(), 1000);
        assert!(result.is_ok());
        
        // Verify position is updated
        let (collateral, debt, _ratio) = Contract::get_position(env.clone(), user.to_string()).unwrap();
        assert_eq!(collateral, 1000);
        assert_eq!(debt, 0);
    });
}

#[test]
fn test_withdraw_insufficient_collateral() {
    let env = TestUtils::create_test_env();
    let _admin = TestUtils::initialize_contract(&env);
    let user = TestUtils::create_user_address(&env, 1);
    
    let contract_id = env.register(Contract, ());
    env.as_contract(&contract_id, || {
        // Setup: deposit small amount
        Contract::deposit_collateral(env.clone(), user.to_string(), 100).unwrap();
        
        // Try to withdraw more than available
        let result = Contract::withdraw(env.clone(), user.to_string(), 200);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), ProtocolError::InsufficientCollateral);
    });
}

#[test]
fn test_withdraw_insufficient_collateral_ratio() {
    let env = TestUtils::create_test_env();
    let _admin = TestUtils::initialize_contract(&env);
    let user = TestUtils::create_user_address(&env, 1);
    
    let contract_id = env.register(Contract, ());
    env.as_contract(&contract_id, || {
        // Setup: deposit and borrow
        Contract::deposit_collateral(env.clone(), user.to_string(), 2000).unwrap();
        Contract::borrow(env.clone(), user.to_string(), 1000).unwrap();
        
        // Try to withdraw too much (would breach collateral ratio)
        let result = Contract::withdraw(env.clone(), user.to_string(), 1500);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), ProtocolError::InsufficientCollateralRatio);
    });
}

#[test]
fn test_liquidate_success() {
    let env = TestUtils::create_test_env();
    let _admin = TestUtils::initialize_contract(&env);
    let user = TestUtils::create_user_address(&env, 1);
    let liquidator = TestUtils::create_user_address(&env, 2);
    
    let contract_id = env.register(Contract, ());
    env.as_contract(&contract_id, || {
        // Setup: deposit small collateral and borrow large amount
        Contract::deposit_collateral(env.clone(), user.to_string(), 100).unwrap();
        Contract::borrow(env.clone(), user.to_string(), 1000).unwrap();
        
        // Liquidate (should succeed as position is undercollateralized)
        let result = Contract::liquidate(env.clone(), liquidator.to_string(), 500);
        assert!(result.is_ok());
        
        // Verify position is updated (debt reduced, collateral penalized)
        let (collateral, debt, _ratio) = Contract::get_position(env.clone(), user.to_string()).unwrap();
        assert_eq!(debt, 500); // Debt reduced by 500
        assert!(collateral < 100); // Collateral penalized
    });
}

#[test]
fn test_liquidate_not_eligible() {
    let env = TestUtils::create_test_env();
    let _admin = TestUtils::initialize_contract(&env);
    let user = TestUtils::create_user_address(&env, 1);
    let liquidator = TestUtils::create_user_address(&env, 2);
    
    let contract_id = env.register(Contract, ());
    env.as_contract(&contract_id, || {
        // Setup: deposit sufficient collateral and borrow small amount
        Contract::deposit_collateral(env.clone(), user.to_string(), 2000).unwrap();
        Contract::borrow(env.clone(), user.to_string(), 1000).unwrap();
        
        // Try to liquidate (should fail as position is well-collateralized)
        let result = Contract::liquidate(env.clone(), liquidator.to_string(), 500);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), ProtocolError::NotEligibleForLiquidation);
    });
}

#[test]
fn test_admin_functions() {
    let env = TestUtils::create_test_env();
    let admin = TestUtils::initialize_contract(&env);
    let non_admin = TestUtils::create_user_address(&env, 1);
    let oracle = TestUtils::create_oracle_address(&env);
    
    let contract_id = env.register(Contract, ());
    env.as_contract(&contract_id, || {
        // Test admin can set oracle
        let result = Contract::set_oracle(env.clone(), admin.to_string(), oracle.to_string());
        assert!(result.is_ok());
        
        // Test non-admin cannot set oracle
        let result = Contract::set_oracle(env.clone(), non_admin.to_string(), oracle.to_string());
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), ProtocolError::NotAdmin);
        
        // Test admin can set min collateral ratio
        let result = Contract::set_min_collateral_ratio(env.clone(), admin.to_string(), 200);
        assert!(result.is_ok());
        
        // Test non-admin cannot set min collateral ratio
        let result = Contract::set_min_collateral_ratio(env.clone(), non_admin.to_string(), 200);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), ProtocolError::NotAdmin);
    });
}

#[test]
fn test_protocol_params() {
    let env = TestUtils::create_test_env();
    let admin = TestUtils::initialize_contract(&env);
    let oracle = TestUtils::create_oracle_address(&env);
    
    let contract_id = env.register(Contract, ());
    env.as_contract(&contract_id, || {
        // Set oracle
        Contract::set_oracle(env.clone(), admin.to_string(), oracle.to_string()).unwrap();
        
        // Get protocol params
        let (stored_admin, stored_oracle, min_ratio) = Contract::get_protocol_params(env.clone()).unwrap();
        assert_eq!(stored_admin, admin);
        assert_eq!(stored_oracle, oracle);
        assert_eq!(min_ratio, 150); // Default value
    });
}

#[test]
fn test_system_stats() {
    let env = TestUtils::create_test_env();
    let _admin = TestUtils::initialize_contract(&env);
    
    let contract_id = env.register(Contract, ());
    env.as_contract(&contract_id, || {
        let (total_collateral, total_debt) = Contract::get_system_stats(env.clone()).unwrap();
        // For now, these are stubbed to return (0, 0)
        assert_eq!(total_collateral, 0);
        assert_eq!(total_debt, 0);
    });
}

#[test]
fn test_event_history_stubs() {
    let env = TestUtils::create_test_env();
    let _admin = TestUtils::initialize_contract(&env);
    let user = TestUtils::create_user_address(&env, 1);
    
    let contract_id = env.register(Contract, ());
    env.as_contract(&contract_id, || {
        // Test user event history (stubbed)
        let events = Contract::get_user_event_history(env.clone(), user.to_string(), String::from_str(&env, "deposit")).unwrap();
        assert_eq!(events.len(), 0); // Empty for now
        
        // Test recent events (stubbed)
        let events = Contract::get_recent_events(env.clone(), 10).unwrap();
        assert_eq!(events.len(), 0); // Empty for now
    });
}

#[test]
fn test_edge_cases() {
    let env = TestUtils::create_test_env();
    let _admin = TestUtils::initialize_contract(&env);
    let user = TestUtils::create_user_address(&env, 1);
    
    let contract_id = env.register(Contract, ());
    env.as_contract(&contract_id, || {
        // Test with maximum i128 values
        let max_amount = i128::MAX;
        let result = Contract::deposit_collateral(env.clone(), user.to_string(), max_amount);
        assert!(result.is_ok());
        
        // Test with minimum i128 values
        let result = Contract::deposit_collateral(env.clone(), user.to_string(), i128::MIN);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), ProtocolError::InvalidAmount);
    });
}

#[test]
fn test_multiple_users() {
    let env = TestUtils::create_test_env();
    let _admin = TestUtils::initialize_contract(&env);
    let user1 = TestUtils::create_user_address(&env, 1);
    let user2 = TestUtils::create_user_address(&env, 2);
    
    let contract_id = env.register(Contract, ());
    env.as_contract(&contract_id, || {
        // User 1 deposits and borrows
        Contract::deposit_collateral(env.clone(), user1.to_string(), 2000).unwrap();
        Contract::borrow(env.clone(), user1.to_string(), 1000).unwrap();
        
        // User 2 deposits and borrows
        Contract::deposit_collateral(env.clone(), user2.to_string(), 3000).unwrap();
        Contract::borrow(env.clone(), user2.to_string(), 1500).unwrap();
        
        // Verify positions are independent
        let (collateral1, debt1, _) = Contract::get_position(env.clone(), user1.to_string()).unwrap();
        let (collateral2, debt2, _) = Contract::get_position(env.clone(), user2.to_string()).unwrap();
        
        assert_eq!(collateral1, 2000);
        assert_eq!(debt1, 1000);
        assert_eq!(collateral2, 3000);
        assert_eq!(debt2, 1500);
    });
}

#[test]
fn test_error_enum_values() {
    // Test that all error variants have correct string representations
    assert_eq!(ProtocolError::Unauthorized.to_str(), "Unauthorized");
    assert_eq!(ProtocolError::InsufficientCollateral.to_str(), "InsufficientCollateral");
    assert_eq!(ProtocolError::InsufficientCollateralRatio.to_str(), "InsufficientCollateralRatio");
    assert_eq!(ProtocolError::InvalidAmount.to_str(), "InvalidAmount");
    assert_eq!(ProtocolError::InvalidAddress.to_str(), "InvalidAddress");
    assert_eq!(ProtocolError::PositionNotFound.to_str(), "PositionNotFound");
    assert_eq!(ProtocolError::AlreadyInitialized.to_str(), "AlreadyInitialized");
    assert_eq!(ProtocolError::NotAdmin.to_str(), "NotAdmin");
    assert_eq!(ProtocolError::OracleNotSet.to_str(), "OracleNotSet");
    assert_eq!(ProtocolError::AdminNotSet.to_str(), "AdminNotSet");
    assert_eq!(ProtocolError::NotEligibleForLiquidation.to_str(), "NotEligibleForLiquidation");
    assert_eq!(ProtocolError::Unknown.to_str(), "Unknown");
}
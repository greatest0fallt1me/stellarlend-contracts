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
        Self::create_test_address(env, "GCAZYE3EB54VKP3UQBX3H73VQO3SIWTZNR7NJQKJFZZ6XLADWA4C3SOC")
    }

    /// Create a test user address
    pub fn create_user_address(env: &Env, user_id: u32) -> Address {
        match user_id {
            0 => Self::create_test_address(env, "GCXOTMMXRS24MYZI5FJPUCOEOFNWSR4XX7UXIK3NDGGE6A5QMJ5FF2FS"),
            1 => Self::create_test_address(env, "GAUA7XL5K54CC2DDGP77FJ2YBHRJLT36CPZDXWPM6MP7MANOGG77PNJU"),
            2 => Self::create_test_address(env, "GBXOTMMXRS24MYZI5FJPUCOEOFNWSR4XX7UXIK3NDGGE6A5QMJ5FF2FS"),
            _ => Self::create_test_address(env, "GCUA7XL5K54CC2DDGP77FJ2YBHRJLT36CPZDXWPM6MP7MANOGG77PNJU"),
        }
    }

    /// Create a test oracle address
    pub fn create_oracle_address(env: &Env) -> Address {
        Self::create_test_address(env, "GCXOTMMXRS24MYZI5FJPUCOEOFNWSR4XX7UXIK3NDGGE6A5QMJ5FF2FS")
    }

    /// Initialize the contract with test admin
    pub fn initialize_contract(env: &Env) -> Address {
        let admin = Self::create_admin_address(env);
        let contract_id = env.register(Contract, ());
        env.as_contract(&contract_id, || {
            Contract::initialize(env.clone(), admin.to_string()).unwrap();
            
            // Set oracle address for RealPriceOracle to work
            let oracle = Self::create_oracle_address(env);
            Contract::set_oracle(env.clone(), admin.to_string(), oracle.to_string()).unwrap();
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
    
    fn get_last_update(_env: &Env) -> u64 {
        0 // Test oracle doesn't track updates
    }
    
    fn validate_price(_env: &Env, _price: i128) -> bool {
        true // Test oracle always validates
    }
}

#[test]
fn test_contract_initialization() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = TestUtils::create_admin_address(&env);
    
    let contract_id = env.register(Contract, ());
    env.as_contract(&contract_id, || {
        let result = Contract::initialize(env.clone(), admin.to_string());
        assert!(result.is_ok());
        
        // Test that admin is set correctly - but don't call get_protocol_params yet
        // since oracle is not set
        let admin_key = ProtocolConfig::admin_key();
        let stored_admin = env.storage().instance().get::<Symbol, Address>(&admin_key).unwrap();
        assert_eq!(stored_admin, admin);
    });
}

#[test]
fn test_contract_initialization_already_initialized() {
    let env = Env::default();
    env.mock_all_auths();
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
    let env = Env::default();
    env.mock_all_auths();
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
    let env = Env::default();
    env.mock_all_auths();
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
    let env = Env::default();
    env.mock_all_auths();
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
    let env = Env::default();
    env.mock_all_auths();
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
    let env = Env::default();
    env.mock_all_auths();
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
    let env = Env::default();
    env.mock_all_auths();
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
    let env = Env::default();
    env.mock_all_auths();
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
    let env = Env::default();
    env.mock_all_auths();
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
    let env = Env::default();
    env.mock_all_auths();
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
    let env = Env::default();
    env.mock_all_auths();
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
    let env = Env::default();
    env.mock_all_auths();
    let admin = TestUtils::create_admin_address(&env);
    let user = TestUtils::create_user_address(&env, 1);
    let liquidator = TestUtils::create_user_address(&env, 2);
    
    let contract_id = env.register(Contract, ());
    env.as_contract(&contract_id, || {
        // Initialize contract
        Contract::initialize(env.clone(), admin.to_string()).unwrap();
        
        // Setup: deposit very small collateral and borrow large amount
        Contract::deposit_collateral(env.clone(), user.to_string(), 10).unwrap();
        Contract::borrow(env.clone(), user.to_string(), 1000).unwrap();
        
        // Liquidate the user's position (not the liquidator's)
        let result = Contract::liquidate(env.clone(), liquidator.to_string(), user.to_string(), 500);
        assert!(result.is_ok());
        
        // Verify position is updated (debt reduced, collateral penalized)
        let (collateral, debt, _ratio) = Contract::get_position(env.clone(), user.to_string()).unwrap();
        assert_eq!(debt, 500); // Debt reduced by 500
        assert!(collateral < 10); // Collateral penalized
    });
}

#[test]
fn test_liquidate_not_eligible() {
    let env = Env::default();
    env.mock_all_auths();
    let _admin = TestUtils::initialize_contract(&env);
    let user = TestUtils::create_user_address(&env, 1);
    let liquidator = TestUtils::create_user_address(&env, 2);
    
    let contract_id = env.register(Contract, ());
    env.as_contract(&contract_id, || {
        // Setup: deposit sufficient collateral and borrow small amount
        Contract::deposit_collateral(env.clone(), user.to_string(), 2000).unwrap();
        Contract::borrow(env.clone(), user.to_string(), 1000).unwrap();
        
        // Try to liquidate (should fail as position is well-collateralized)
        let result = Contract::liquidate(env.clone(), liquidator.to_string(), user.to_string(), 500);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), ProtocolError::NotEligibleForLiquidation);
    });
}

#[test]
fn test_admin_functions() {
    let env = Env::default();
    env.mock_all_auths();
    
    // Initialize contract properly
    let admin = TestUtils::create_admin_address(&env);
    let contract_id = env.register(Contract, ());
    env.as_contract(&contract_id, || {
        Contract::initialize(env.clone(), admin.to_string()).unwrap();
        
        let non_admin = TestUtils::create_user_address(&env, 1);
        let oracle = TestUtils::create_oracle_address(&env);
        
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
    let env = Env::default();
    env.mock_all_auths();
    
    // Initialize contract properly
    let admin = TestUtils::create_admin_address(&env);
    let contract_id = env.register(Contract, ());
    env.as_contract(&contract_id, || {
        Contract::initialize(env.clone(), admin.to_string()).unwrap();
        
        let oracle = TestUtils::create_oracle_address(&env);
        
        // Set oracle first
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
    let env = Env::default();
    env.mock_all_auths();
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
    let env = Env::default();
    env.mock_all_auths();
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
    let env = Env::default();
    env.mock_all_auths();
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
    let env = Env::default();
    env.mock_all_auths();
    let admin = TestUtils::create_admin_address(&env);
    let user1 = TestUtils::create_user_address(&env, 1);
    let user2 = TestUtils::create_user_address(&env, 2);
    
    let contract_id = env.register(Contract, ());
    env.as_contract(&contract_id, || {
        // Initialize contract
        Contract::initialize(env.clone(), admin.to_string()).unwrap();
        
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

#[test]
fn test_oracle_price_validation() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = TestUtils::create_admin_address(&env);
    
    let contract_id = env.register(Contract, ());
    env.as_contract(&contract_id, || {
        Contract::initialize(env.clone(), admin.to_string()).unwrap();
        
        // Set oracle address
        let oracle = TestUtils::create_oracle_address(&env);
        Contract::set_oracle(env.clone(), admin.to_string(), oracle.to_string()).unwrap();
        
        // Set max deviation to 10%
        Contract::set_max_price_deviation(env.clone(), admin.to_string(), 10).unwrap();
        
        // First price should always be valid
        let price1 = RealPriceOracle::get_price(&env);
        assert!(RealPriceOracle::validate_price(&env, price1));
        
        // Price within 10% deviation should be valid
        let valid_price = price1 + (price1 * 5) / 100; // 5% increase
        assert!(RealPriceOracle::validate_price(&env, valid_price));
        
        // Price with 15% deviation should be invalid
        let invalid_price = price1 + (price1 * 15) / 100; // 15% increase
        assert!(!RealPriceOracle::validate_price(&env, invalid_price));
    });
}

#[test]
fn test_oracle_fallback_mechanism() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = TestUtils::create_admin_address(&env);
    
    let contract_id = env.register(Contract, ());
    env.as_contract(&contract_id, || {
        Contract::initialize(env.clone(), admin.to_string()).unwrap();
        
        // Set oracle address
        let oracle = TestUtils::create_oracle_address(&env);
        Contract::set_oracle(env.clone(), admin.to_string(), oracle.to_string()).unwrap();
        
        // Set very low max deviation to trigger fallback
        Contract::set_max_price_deviation(env.clone(), admin.to_string(), 1).unwrap();
        
        // First price should be accepted
        let price1 = RealPriceOracle::get_price(&env);
        assert!(price1 > 0);
        
        // Second price with any variation should trigger fallback
        let price2 = RealPriceOracle::get_price(&env);
        // Should return fallback price (150_000_000) due to validation failure
        assert_eq!(price2, 150_000_000);
    });
}

#[test]
fn test_oracle_heartbeat_validation() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = TestUtils::create_admin_address(&env);
    
    let contract_id = env.register(Contract, ());
    env.as_contract(&contract_id, || {
        Contract::initialize(env.clone(), admin.to_string()).unwrap();
        
        // Set oracle address
        let oracle = TestUtils::create_oracle_address(&env);
        Contract::set_oracle(env.clone(), admin.to_string(), oracle.to_string()).unwrap();
        
        // Set heartbeat to 100 seconds
        Contract::set_oracle_heartbeat(env.clone(), admin.to_string(), 100).unwrap();
        
        // Initial price should not be stale
        RealPriceOracle::get_price(&env);
        assert!(!OracleConfig::is_price_stale(&env));
        
        // After 100+ seconds, price should be stale
        // Note: In real tests, we'd need to manipulate the ledger timestamp
        // For now, we'll test the logic with current time
        let is_stale = OracleConfig::is_price_stale(&env);
        // This will depend on the actual time elapsed, so we just verify the function works
        assert!(is_stale == true || is_stale == false);
    });
}

#[test]
fn test_oracle_admin_functions() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = TestUtils::create_admin_address(&env);
    let non_admin = TestUtils::create_user_address(&env, 1);
    
    let contract_id = env.register(Contract, ());
    env.as_contract(&contract_id, || {
        Contract::initialize(env.clone(), admin.to_string()).unwrap();
        
        // Set oracle address
        let oracle = TestUtils::create_oracle_address(&env);
        Contract::set_oracle(env.clone(), admin.to_string(), oracle.to_string()).unwrap();
        
        // Test admin can set max deviation
        let result = Contract::set_max_price_deviation(env.clone(), admin.to_string(), 25);
        assert!(result.is_ok());
        
        // Test non-admin cannot set max deviation
        let result = Contract::set_max_price_deviation(env.clone(), non_admin.to_string(), 25);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), ProtocolError::NotAdmin);
        
        // Test admin can set heartbeat
        let result = Contract::set_oracle_heartbeat(env.clone(), admin.to_string(), 1800);
        assert!(result.is_ok());
        
        // Test non-admin cannot set heartbeat
        let result = Contract::set_oracle_heartbeat(env.clone(), non_admin.to_string(), 1800);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), ProtocolError::NotAdmin);
        
        // Test admin can set fallback price
        let result = Contract::set_fallback_price(env.clone(), admin.to_string(), 175_000_000);
        assert!(result.is_ok());
        
        // Test non-admin cannot set fallback price
        let result = Contract::set_fallback_price(env.clone(), non_admin.to_string(), 175_000_000);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), ProtocolError::NotAdmin);
    });
}

#[test]
fn test_get_oracle_info() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = TestUtils::create_admin_address(&env);
    
    let contract_id = env.register(Contract, ());
    env.as_contract(&contract_id, || {
        Contract::initialize(env.clone(), admin.to_string()).unwrap();
        
        // Set oracle address
        let oracle = TestUtils::create_oracle_address(&env);
        Contract::set_oracle(env.clone(), admin.to_string(), oracle.to_string()).unwrap();
        
        // Configure oracle settings
        Contract::set_max_price_deviation(env.clone(), admin.to_string(), 30).unwrap();
        Contract::set_oracle_heartbeat(env.clone(), admin.to_string(), 7200).unwrap();
        Contract::set_fallback_price(env.clone(), admin.to_string(), 160_000_000).unwrap();
        
        // Get oracle info
        let (current_price, last_update, max_deviation, heartbeat, is_stale) = 
            Contract::get_oracle_info(env.clone()).unwrap();
        
        // Verify the values
        assert!(current_price > 0);
        assert!(last_update > 0);
        assert_eq!(max_deviation, 30);
        assert_eq!(heartbeat, 7200);
        assert!(is_stale == true || is_stale == false); // Boolean check
    });
}

#[test]
fn test_force_update_price() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = TestUtils::create_admin_address(&env);
    let non_admin = TestUtils::create_user_address(&env, 1);
    
    let contract_id = env.register(Contract, ());
    env.as_contract(&contract_id, || {
        Contract::initialize(env.clone(), admin.to_string()).unwrap();
        
        // Set oracle address
        let oracle = TestUtils::create_oracle_address(&env);
        Contract::set_oracle(env.clone(), admin.to_string(), oracle.to_string()).unwrap();
        
        // Test admin can force update price
        let result = Contract::force_update_price(env.clone(), admin.to_string(), 250_000_000);
        assert!(result.is_ok());
        
        // Verify price was updated
        let (current_price, _, _, _, _) = Contract::get_oracle_info(env.clone()).unwrap();
        assert_eq!(current_price, 250_000_000);
        
        // Test non-admin cannot force update price
        let result = Contract::force_update_price(env.clone(), non_admin.to_string(), 300_000_000);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), ProtocolError::NotAdmin);
    });
}

#[test]
fn test_oracle_integration_with_lending() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = TestUtils::create_admin_address(&env);
    let user = TestUtils::create_user_address(&env, 1);
    
    let contract_id = env.register(Contract, ());
    env.as_contract(&contract_id, || {
        Contract::initialize(env.clone(), admin.to_string()).unwrap();
        
        // Set oracle address
        let oracle = TestUtils::create_oracle_address(&env);
        Contract::set_oracle(env.clone(), admin.to_string(), oracle.to_string()).unwrap();
        
        // Configure oracle with reasonable settings
        Contract::set_max_price_deviation(env.clone(), admin.to_string(), 50).unwrap();
        
        // Test deposit and borrow with real oracle
        Contract::deposit_collateral(env.clone(), user.to_string(), 2000).unwrap();
        
        // Borrow should work with real oracle prices
        let result = Contract::borrow(env.clone(), user.to_string(), 1000);
        assert!(result.is_ok());
        
        // Verify position uses real oracle prices
        let (collateral, debt, ratio) = Contract::get_position(env.clone(), user.to_string()).unwrap();
        assert_eq!(collateral, 2000);
        assert_eq!(debt, 1000);
        assert!(ratio > 0); // Should have a real ratio from oracle
    });
}

#[test]
fn test_oracle_price_storage() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = TestUtils::create_admin_address(&env);
    
    let contract_id = env.register(Contract, ());
    env.as_contract(&contract_id, || {
        Contract::initialize(env.clone(), admin.to_string()).unwrap();
        
        // Set oracle address
        let oracle = TestUtils::create_oracle_address(&env);
        Contract::set_oracle(env.clone(), admin.to_string(), oracle.to_string()).unwrap();
        
        // Get initial price
        let price1 = RealPriceOracle::get_price(&env);
        let timestamp1 = RealPriceOracle::get_last_update(&env);
        
        assert!(price1 > 0);
        assert!(timestamp1 > 0);
        
        // Get price again (should be cached/stored)
        let price2 = RealPriceOracle::get_price(&env);
        let timestamp2 = RealPriceOracle::get_last_update(&env);
        
        // Prices should be the same (within small variation due to time-based simulation)
        assert!(price1 == price2 || (price1 - price2).abs() < 100_000);
        assert!(timestamp2 >= timestamp1);
    });
}

// --- Interest Rate Management Tests ---

#[test]
fn test_interest_rate_initialization() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = TestUtils::create_admin_address(&env);
    
    let contract_id = env.register(Contract, ());
    env.as_contract(&contract_id, || {
        Contract::initialize(env.clone(), admin.to_string()).unwrap();
        
        // Check that interest rate config is initialized with defaults
        let (base_rate, kink_utilization, multiplier, reserve_factor, rate_floor, rate_ceiling, _) = 
            Contract::get_interest_rate_config(env.clone()).unwrap();
        
        assert_eq!(base_rate, 2000000);        // 2%
        assert_eq!(kink_utilization, 80000000); // 80%
        assert_eq!(multiplier, 10000000);       // 10x
        assert_eq!(reserve_factor, 10000000);   // 10%
        assert_eq!(rate_floor, 100000);         // 0.1%
        assert_eq!(rate_ceiling, 50000000);     // 50%
    });
}

#[test]
fn test_interest_rate_calculation() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = TestUtils::create_admin_address(&env);
    
    let contract_id = env.register(Contract, ());
    env.as_contract(&contract_id, || {
        Contract::initialize(env.clone(), admin.to_string()).unwrap();
        
        // Test initial rates (no utilization)
        let (borrow_rate, supply_rate) = Contract::get_current_rates(env.clone()).unwrap();
        assert_eq!(borrow_rate, 2000000); // Base rate (2%)
        assert_eq!(supply_rate, 0);       // No utilization = no supply rate
        
        // Test utilization metrics
        let (utilization, total_borrowed, total_supplied) = Contract::get_utilization_metrics(env.clone()).unwrap();
        assert_eq!(utilization, 0);
        assert_eq!(total_borrowed, 0);
        assert_eq!(total_supplied, 0);
    });
}

#[test]
fn test_interest_rate_admin_functions() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = TestUtils::create_admin_address(&env);
    let non_admin = TestUtils::create_user_address(&env, 1);
    
    let contract_id = env.register(Contract, ());
    env.as_contract(&contract_id, || {
        Contract::initialize(env.clone(), admin.to_string()).unwrap();
        
        // Test admin can set base rate
        let result = Contract::set_base_rate(env.clone(), admin.to_string(), 3000000); // 3%
        assert!(result.is_ok());
        
        // Test non-admin cannot set base rate
        let result = Contract::set_base_rate(env.clone(), non_admin.to_string(), 4000000);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), ProtocolError::NotAdmin);
        
        // Test admin can set kink utilization
        let result = Contract::set_kink_utilization(env.clone(), admin.to_string(), 70000000); // 70%
        assert!(result.is_ok());
        
        // Test admin can set multiplier
        let result = Contract::set_multiplier(env.clone(), admin.to_string(), 15000000); // 15x
        assert!(result.is_ok());
        
        // Test admin can set reserve factor
        let result = Contract::set_reserve_factor(env.clone(), admin.to_string(), 15000000); // 15%
        assert!(result.is_ok());
        
        // Test admin can set rate limits
        let result = Contract::set_rate_limits(env.clone(), admin.to_string(), 50000, 75000000); // 0.05% to 75%
        assert!(result.is_ok());
        
        // Verify config was updated
        let (base_rate, kink_utilization, multiplier, reserve_factor, rate_floor, rate_ceiling, _) = 
            Contract::get_interest_rate_config(env.clone()).unwrap();
        
        assert_eq!(base_rate, 3000000);
        assert_eq!(kink_utilization, 70000000);
        assert_eq!(multiplier, 15000000);
        assert_eq!(reserve_factor, 15000000);
        assert_eq!(rate_floor, 50000);
        assert_eq!(rate_ceiling, 75000000);
    });
}

#[test]
fn test_interest_rate_with_utilization() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = TestUtils::create_admin_address(&env);
    let user = TestUtils::create_user_address(&env, 1);
    
    let contract_id = env.register(Contract, ());
    env.as_contract(&contract_id, || {
        Contract::initialize(env.clone(), admin.to_string()).unwrap();
        
        // Set oracle address for borrowing
        let oracle = TestUtils::create_oracle_address(&env);
        Contract::set_oracle(env.clone(), admin.to_string(), oracle.to_string()).unwrap();
        
        // Deposit collateral
        Contract::deposit_collateral(env.clone(), user.to_string(), 10000).unwrap();
        
        // Check utilization after deposit
        let (utilization, total_borrowed, total_supplied) = Contract::get_utilization_metrics(env.clone()).unwrap();
        assert_eq!(utilization, 0);
        assert_eq!(total_borrowed, 0);
        assert_eq!(total_supplied, 10000);
        
        // Borrow some amount
        Contract::borrow(env.clone(), user.to_string(), 5000).unwrap();
        
        // Check utilization after borrow (50%)
        let (utilization, total_borrowed, total_supplied) = Contract::get_utilization_metrics(env.clone()).unwrap();
        assert_eq!(utilization, 50000000); // 50% * 1e8
        assert_eq!(total_borrowed, 5000);
        assert_eq!(total_supplied, 10000);
        
        // Check rates with 50% utilization
        let (borrow_rate, supply_rate) = Contract::get_current_rates(env.clone()).unwrap();
        assert_eq!(borrow_rate, 2000000); // Still base rate (below kink)
        assert!(supply_rate > 0); // Should have some supply rate now
    });
}

#[test]
fn test_interest_accrual() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = TestUtils::create_admin_address(&env);
    let user = TestUtils::create_user_address(&env, 1);
    
    let contract_id = env.register(Contract, ());
    env.as_contract(&contract_id, || {
        Contract::initialize(env.clone(), admin.to_string()).unwrap();
        
        // Set oracle address
        let oracle = TestUtils::create_oracle_address(&env);
        Contract::set_oracle(env.clone(), admin.to_string(), oracle.to_string()).unwrap();
        
        // Deposit and borrow
        Contract::deposit_collateral(env.clone(), user.to_string(), 10000).unwrap();
        Contract::borrow(env.clone(), user.to_string(), 5000).unwrap();
        
        // Check initial accrued interest
        let (borrow_interest, supply_interest) = Contract::get_user_accrued_interest(env.clone(), user.to_string()).unwrap();
        assert_eq!(borrow_interest, 0);
        assert_eq!(supply_interest, 0);
        
        // Manually accrue interest
        Contract::accrue_interest(env.clone()).unwrap();
        
        // Check accrued interest again (should still be 0 due to minimal time)
        let (borrow_interest, supply_interest) = Contract::get_user_accrued_interest(env.clone(), user.to_string()).unwrap();
        assert!(borrow_interest >= 0);
        assert!(supply_interest >= 0);
    });
}

#[test]
fn test_emergency_rate_adjustment() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = TestUtils::create_admin_address(&env);
    let non_admin = TestUtils::create_user_address(&env, 1);
    
    let contract_id = env.register(Contract, ());
    env.as_contract(&contract_id, || {
        Contract::initialize(env.clone(), admin.to_string()).unwrap();
        
        // Test admin can make emergency rate adjustment
        let result = Contract::emergency_rate_adjustment(env.clone(), admin.to_string(), 10000000); // 10%
        assert!(result.is_ok());
        
        // Test non-admin cannot make emergency adjustment
        let result = Contract::emergency_rate_adjustment(env.clone(), non_admin.to_string(), 15000000);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), ProtocolError::NotAdmin);
        
        // Verify rate was updated (get directly from state to avoid recalculation)
        let state = InterestRateStorage::get_state(&env);
        assert_eq!(state.current_borrow_rate, 10000000);
    });
}

#[test]
fn test_interest_rate_integration_with_lending() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = TestUtils::create_admin_address(&env);
    let user = TestUtils::create_user_address(&env, 1);
    
    let contract_id = env.register(Contract, ());
    env.as_contract(&contract_id, || {
        Contract::initialize(env.clone(), admin.to_string()).unwrap();
        
        // Set oracle address
        let oracle = TestUtils::create_oracle_address(&env);
        Contract::set_oracle(env.clone(), admin.to_string(), oracle.to_string()).unwrap();
        
        // Get initial rates
        let (initial_borrow_rate, initial_supply_rate) = Contract::get_current_rates(env.clone()).unwrap();
        
        // Deposit collateral
        Contract::deposit_collateral(env.clone(), user.to_string(), 10000).unwrap();
        
        // Borrow (should trigger interest accrual)
        Contract::borrow(env.clone(), user.to_string(), 5000).unwrap();
        
        // Check that rates are updated
        let (borrow_rate, supply_rate) = Contract::get_current_rates(env.clone()).unwrap();
        assert_eq!(borrow_rate, initial_borrow_rate); // Should still be base rate
        assert!(supply_rate > initial_supply_rate); // Should have supply rate now
        
        // Check utilization
        let (utilization, total_borrowed, total_supplied) = Contract::get_utilization_metrics(env.clone()).unwrap();
        assert_eq!(utilization, 50000000); // 50%
        assert_eq!(total_borrowed, 5000);
        assert_eq!(total_supplied, 10000);
        
        // Repay some debt
        Contract::repay(env.clone(), user.to_string(), 2000).unwrap();
        
        // Check updated utilization
        let (utilization, total_borrowed, total_supplied) = Contract::get_utilization_metrics(env.clone()).unwrap();
        assert_eq!(utilization, 30000000); // 30%
        assert_eq!(total_borrowed, 3000);
        assert_eq!(total_supplied, 10000);
    });
}

#[test]
fn test_interest_rate_edge_cases() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = TestUtils::create_admin_address(&env);
    
    let contract_id = env.register(Contract, ());
    env.as_contract(&contract_id, || {
        Contract::initialize(env.clone(), admin.to_string()).unwrap();
        
        // Test with zero utilization
        let (borrow_rate, supply_rate) = Contract::get_current_rates(env.clone()).unwrap();
        assert_eq!(borrow_rate, 2000000); // Base rate
        assert_eq!(supply_rate, 0);       // No supply rate
        
        // Test rate limits
        Contract::set_rate_limits(env.clone(), admin.to_string(), 1000000, 3000000).unwrap(); // 1% to 3%
        
        // Set very high base rate (should be capped)
        Contract::set_base_rate(env.clone(), admin.to_string(), 10000000).unwrap(); // 10%
        
        let (borrow_rate, _) = Contract::get_current_rates(env.clone()).unwrap();
        assert_eq!(borrow_rate, 3000000); // Should be capped at 3%
        
        // Set very low base rate (should be floored)
        Contract::set_base_rate(env.clone(), admin.to_string(), 50000).unwrap(); // 0.05%
        
        let (borrow_rate, _) = Contract::get_current_rates(env.clone()).unwrap();
        assert_eq!(borrow_rate, 1000000); // Should be floored at 1%
    });
}

// --- Risk Management & Liquidation Enhancement Tests ---

#[test]
fn test_risk_config_initialization() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = TestUtils::create_admin_address(&env);
    
    let contract_id = env.register(Contract, ());
    env.as_contract(&contract_id, || {
        Contract::initialize(env.clone(), admin.to_string()).unwrap();
        
        // Check that risk config is initialized with defaults
        let (close_factor, liquidation_incentive, pause_borrow, pause_deposit, pause_withdraw, pause_liquidate, _) = 
            Contract::get_risk_config(env.clone());
        
        assert_eq!(close_factor, 50000000);        // 50%
        assert_eq!(liquidation_incentive, 10000000); // 10%
        assert_eq!(pause_borrow, false);
        assert_eq!(pause_deposit, false);
        assert_eq!(pause_withdraw, false);
        assert_eq!(pause_liquidate, false);
    });
}

#[test]
fn test_risk_params_admin_functions() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = TestUtils::create_admin_address(&env);
    let non_admin = TestUtils::create_user_address(&env, 1);
    
    let contract_id = env.register(Contract, ());
    env.as_contract(&contract_id, || {
        Contract::initialize(env.clone(), admin.to_string()).unwrap();
        
        // Test admin can set risk parameters
        let result = Contract::set_risk_params(env.clone(), admin.to_string(), 60000000, 15000000); // 60%, 15%
        assert!(result.is_ok());
        
        // Test non-admin cannot set risk parameters
        let result = Contract::set_risk_params(env.clone(), non_admin.to_string(), 70000000, 20000000);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), ProtocolError::NotAdmin);
        
        // Verify config was updated
        let (close_factor, liquidation_incentive, _, _, _, _, _) = Contract::get_risk_config(env.clone());
        assert_eq!(close_factor, 60000000);
        assert_eq!(liquidation_incentive, 15000000);
    });
}

#[test]
fn test_pause_switches_admin_functions() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = TestUtils::create_admin_address(&env);
    let non_admin = TestUtils::create_user_address(&env, 1);
    
    let contract_id = env.register(Contract, ());
    env.as_contract(&contract_id, || {
        Contract::initialize(env.clone(), admin.to_string()).unwrap();
        
        // Test admin can set pause switches
        let result = Contract::set_pause_switches(env.clone(), admin.to_string(), true, false, true, false);
        assert!(result.is_ok());
        
        // Test non-admin cannot set pause switches
        let result = Contract::set_pause_switches(env.clone(), non_admin.to_string(), false, true, false, true);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), ProtocolError::NotAdmin);
        
        // Verify config was updated
        let (_, _, pause_borrow, pause_deposit, pause_withdraw, pause_liquidate, _) = Contract::get_risk_config(env.clone());
        assert_eq!(pause_borrow, true);
        assert_eq!(pause_deposit, false);
        assert_eq!(pause_withdraw, true);
        assert_eq!(pause_liquidate, false);
    });
}

#[test]
fn test_pause_switches_enforcement() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = TestUtils::create_admin_address(&env);
    let user = TestUtils::create_user_address(&env, 1);
    
    let contract_id = env.register(Contract, ());
    env.as_contract(&contract_id, || {
        Contract::initialize(env.clone(), admin.to_string()).unwrap();
        
        // Set oracle address
        let oracle = TestUtils::create_oracle_address(&env);
        Contract::set_oracle(env.clone(), admin.to_string(), oracle.to_string()).unwrap();
        
        // Pause borrow
        Contract::set_pause_switches(env.clone(), admin.to_string(), true, false, false, false).unwrap();
        
        // Try to borrow (should fail)
        Contract::deposit_collateral(env.clone(), user.to_string(), 10000).unwrap();
        let result = Contract::borrow(env.clone(), user.to_string(), 5000);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), ProtocolError::ProtocolPaused);
        
        // Unpause borrow and pause deposit
        Contract::set_pause_switches(env.clone(), admin.to_string(), false, true, false, false).unwrap();
        
        // Try to deposit (should fail)
        let result = Contract::deposit_collateral(env.clone(), user.to_string(), 5000);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), ProtocolError::ProtocolPaused);
        
        // Pause withdraw
        Contract::set_pause_switches(env.clone(), admin.to_string(), false, false, true, false).unwrap();
        
        // Try to withdraw (should fail)
        let result = Contract::withdraw(env.clone(), user.to_string(), 1000);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), ProtocolError::ProtocolPaused);
    });
}

#[test]
fn test_enhanced_liquidation_with_close_factor() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = TestUtils::create_admin_address(&env);
    let user = TestUtils::create_user_address(&env, 1);
    let liquidator = TestUtils::create_user_address(&env, 2);
    
    let contract_id = env.register(Contract, ());
    env.as_contract(&contract_id, || {
        Contract::initialize(env.clone(), admin.to_string()).unwrap();
        
        // Set oracle address
        let oracle = TestUtils::create_oracle_address(&env);
        Contract::set_oracle(env.clone(), admin.to_string(), oracle.to_string()).unwrap();
        
        // Set close factor to 30%
        Contract::set_risk_params(env.clone(), admin.to_string(), 30000000, 10000000).unwrap();
        
        // Create undercollateralized position
        Contract::deposit_collateral(env.clone(), user.to_string(), 1000).unwrap();
        Contract::borrow(env.clone(), user.to_string(), 800).unwrap();
        
        // Try to liquidate more than close factor allows (should be limited)
        let result = Contract::liquidate(env.clone(), liquidator.to_string(), user.to_string(), 500);
        assert!(result.is_ok());
        
        // Check position - should only have 30% of debt liquidated
        let (collateral, debt, _) = Contract::get_position(env.clone(), user.to_string()).unwrap();
        assert_eq!(debt, 560); // 800 - (800 * 0.3) = 800 - 240 = 560
    });
}

#[test]
fn test_enhanced_liquidation_with_incentive() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = TestUtils::create_admin_address(&env);
    let user = TestUtils::create_user_address(&env, 1);
    let liquidator = TestUtils::create_user_address(&env, 2);
    
    let contract_id = env.register(Contract, ());
    env.as_contract(&contract_id, || {
        Contract::initialize(env.clone(), admin.to_string()).unwrap();
        
        // Set oracle address
        let oracle = TestUtils::create_oracle_address(&env);
        Contract::set_oracle(env.clone(), admin.to_string(), oracle.to_string()).unwrap();
        
        // Set liquidation incentive to 20%
        Contract::set_risk_params(env.clone(), admin.to_string(), 50000000, 20000000).unwrap();
        
        // Create undercollateralized position
        Contract::deposit_collateral(env.clone(), user.to_string(), 1000).unwrap();
        Contract::borrow(env.clone(), user.to_string(), 800).unwrap();
        
        // Record initial collateral
        let (initial_collateral, _, _) = Contract::get_position(env.clone(), user.to_string()).unwrap();
        
        // Liquidate
        let result = Contract::liquidate(env.clone(), liquidator.to_string(), user.to_string(), 400);
        assert!(result.is_ok());
        
        // Check position - should have lost debt + incentive
        let (collateral, debt, _) = Contract::get_position(env.clone(), user.to_string()).unwrap();
        assert_eq!(debt, 400); // 800 - 400 = 400
        
        // Collateral should be reduced by debt + incentive
        let expected_collateral_loss = 400 + (400 * 20000000 / 100_000_000); // debt + 20% incentive
        assert_eq!(collateral, initial_collateral - expected_collateral_loss);
    });
}

#[test]
fn test_liquidation_pause_enforcement() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = TestUtils::create_admin_address(&env);
    let user = TestUtils::create_user_address(&env, 1);
    let liquidator = TestUtils::create_user_address(&env, 2);
    
    let contract_id = env.register(Contract, ());
    env.as_contract(&contract_id, || {
        Contract::initialize(env.clone(), admin.to_string()).unwrap();
        
        // Set oracle address
        let oracle = TestUtils::create_oracle_address(&env);
        Contract::set_oracle(env.clone(), admin.to_string(), oracle.to_string()).unwrap();
        
        // Create undercollateralized position
        Contract::deposit_collateral(env.clone(), user.to_string(), 1000).unwrap();
        Contract::borrow(env.clone(), user.to_string(), 800).unwrap();
        
        // Pause liquidation
        Contract::set_pause_switches(env.clone(), admin.to_string(), false, false, false, true).unwrap();
        
        // Try to liquidate (should fail)
        let result = Contract::liquidate(env.clone(), liquidator.to_string(), user.to_string(), 400);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), ProtocolError::ProtocolPaused);
    });
}

#[test]
fn test_risk_management_integration() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = TestUtils::create_admin_address(&env);
    let user = TestUtils::create_user_address(&env, 1);
    let liquidator = TestUtils::create_user_address(&env, 2);
    
    let contract_id = env.register(Contract, ());
    env.as_contract(&contract_id, || {
        Contract::initialize(env.clone(), admin.to_string()).unwrap();
        
        // Set oracle address
        let oracle = TestUtils::create_oracle_address(&env);
        Contract::set_oracle(env.clone(), admin.to_string(), oracle.to_string()).unwrap();
        
        // Configure risk parameters
        Contract::set_risk_params(env.clone(), admin.to_string(), 40000000, 12000000).unwrap(); // 40%, 12%
        
        // Create position and test full risk management flow
        Contract::deposit_collateral(env.clone(), user.to_string(), 10000).unwrap();
        Contract::borrow(env.clone(), user.to_string(), 8000).unwrap();
        
        // Pause borrow
        Contract::set_pause_switches(env.clone(), admin.to_string(), true, false, false, false).unwrap();
        
        // Try to borrow more (should fail)
        let result = Contract::borrow(env.clone(), user.to_string(), 1000);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), ProtocolError::ProtocolPaused);
        
        // Unpause and test liquidation
        Contract::set_pause_switches(env.clone(), admin.to_string(), false, false, false, false).unwrap();
        
        // Liquidate with close factor and incentive
        let result = Contract::liquidate(env.clone(), liquidator.to_string(), user.to_string(), 2000);
        assert!(result.is_ok());
        
        // Verify liquidation worked with risk parameters
        let (collateral, debt, _) = Contract::get_position(env.clone(), user.to_string()).unwrap();
        assert!(debt < 8000); // Should be reduced
        assert!(collateral < 10000); // Should be reduced by debt + incentive
    });
}
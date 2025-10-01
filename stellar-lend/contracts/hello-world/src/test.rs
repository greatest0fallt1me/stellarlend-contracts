#![cfg(test)]

use super::*;
use soroban_sdk::{testutils::Address as TestAddress, Address, Env, String};

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
        Self::create_test_address(
            env,
            "GCAZYE3EB54VKP3UQBX3H73VQO3SIWTZNR7NJQKJFZZ6XLADWA4C3SOC",
        )
    }

    /// Create a test user address
    pub fn create_user_address(env: &Env, user_id: u32) -> Address {
        match user_id {
            0 => Self::create_test_address(
                env,
                "GCXOTMMXRS24MYZI5FJPUCOEOFNWSR4XX7UXIK3NDGGE6A5QMJ5FF2FS",
            ),
            1 => Self::create_test_address(
                env,
                "GAUA7XL5K54CC2DDGP77FJ2YBHRJLT36CPZDXWPM6MP7MANOGG77PNJU",
            ),
            2 => Self::create_test_address(
                env,
                "GCUA7XL5K54CC2DDGP77FJ2YBHRJLT36CPZDXWPM6MP7MANOGG77PNJU",
            ),
            _ => Self::create_test_address(
                env,
                "GCUA7XL5K54CC2DDGP77FJ2YBHRJLT36CPZDXWPM6MP7MANOGG77PNJU",
            ),
        }
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

    /// Mark a user as verified for testing convenience
    pub fn verify_user(env: &Env, admin: &Address, user: &Address) {
        Contract::set_user_verification(
            env.clone(),
            admin.to_string(),
            user.clone(),
            VerificationStatus::Verified,
        )
        .unwrap();
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
    });
}

#[test]
fn test_contract_already_initialized() {
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

    let admin = TestUtils::create_admin_address(&env);
    let user = TestUtils::create_user_address(&env, 0);

    let contract_id = env.register(Contract, ());
    env.as_contract(&contract_id, || {
        // Initialize contract
        Contract::initialize(env.clone(), admin.to_string()).unwrap();
        TestUtils::verify_user(&env, &admin, &user);

        // Test successful deposit
        let result = Contract::deposit_collateral(env.clone(), user.to_string(), 1000);
        assert!(result.is_ok());

        // Verify position
        let position = Contract::get_position(env.clone(), user.to_string()).unwrap();
        assert_eq!(position.0, 1000); // collateral
        assert_eq!(position.1, 0); // debt
    });
}

#[test]
fn test_deposit_collateral_invalid_amount() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = TestUtils::create_admin_address(&env);
    let user = TestUtils::create_user_address(&env, 0);

    let contract_id = env.register(Contract, ());
    env.as_contract(&contract_id, || {
        // Initialize contract
        Contract::initialize(env.clone(), admin.to_string()).unwrap();
        TestUtils::verify_user(&env, &admin, &user);

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

    let admin = TestUtils::create_admin_address(&env);

    let contract_id = env.register(Contract, ());
    env.as_contract(&contract_id, || {
        // Initialize contract
        Contract::initialize(env.clone(), admin.to_string()).unwrap();

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

    let admin = TestUtils::create_admin_address(&env);
    let user = TestUtils::create_user_address(&env, 0);

    let contract_id = env.register(Contract, ());
    env.as_contract(&contract_id, || {
        // Initialize contract
        Contract::initialize(env.clone(), admin.to_string()).unwrap();
        TestUtils::verify_user(&env, &admin, &user);

        // Deposit collateral first
        Contract::deposit_collateral(env.clone(), user.to_string(), 2000).unwrap();

        // Test successful borrow
        let result = Contract::borrow(env.clone(), user.to_string(), 1000);
        assert!(result.is_ok());

        // Verify position
        let position = Contract::get_position(env.clone(), user.to_string()).unwrap();
        assert_eq!(position.0, 2000); // collateral
        assert_eq!(position.1, 1000); // debt
    });
}

#[test]
fn test_borrow_insufficient_collateral_ratio() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = TestUtils::create_admin_address(&env);
    let user = TestUtils::create_user_address(&env, 0);

    let contract_id = env.register(Contract, ());
    env.as_contract(&contract_id, || {
        // Initialize contract
        Contract::initialize(env.clone(), admin.to_string()).unwrap();
        TestUtils::verify_user(&env, &admin, &user);

        // Deposit small amount of collateral
        Contract::deposit_collateral(env.clone(), user.to_string(), 100).unwrap();

        // Try to borrow too much (should fail due to insufficient collateral ratio)
        let result = Contract::borrow(env.clone(), user.to_string(), 1000);
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err(),
            ProtocolError::InsufficientCollateralRatio
        );
    });
}

#[test]
fn test_emergency_pause_blocks_deposit() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = TestUtils::create_admin_address(&env);
    let user = TestUtils::create_user_address(&env, 0);

    let contract_id = env.register(Contract, ());
    env.as_contract(&contract_id, || {
        Contract::initialize(env.clone(), admin.to_string()).unwrap();
        TestUtils::verify_user(&env, &admin, &user);

        let reason = Some(String::from_str(&env, "halt"));
        Contract::trigger_emergency_pause(env.clone(), admin.to_string(), reason).unwrap();

        let result = Contract::deposit_collateral(env.clone(), user.to_string(), 1000);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), ProtocolError::ProtocolPaused);

        Contract::resume_operations(env.clone(), admin.to_string()).unwrap();
        let result = Contract::deposit_collateral(env.clone(), user.to_string(), 1000);
        assert!(result.is_ok());
    });
}

#[test]
fn test_recovery_mode_allows_repay_blocks_borrow() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = TestUtils::create_admin_address(&env);
    let user = TestUtils::create_user_address(&env, 0);

    let contract_id = env.register(Contract, ());
    env.as_contract(&contract_id, || {
        Contract::initialize(env.clone(), admin.to_string()).unwrap();
        TestUtils::verify_user(&env, &admin, &user);

        Contract::deposit_collateral(env.clone(), user.to_string(), 2000).unwrap();
        Contract::borrow(env.clone(), user.to_string(), 500).unwrap();

        let plan = Some(String::from_str(&env, "staged restart"));
        Contract::enter_recovery_mode(env.clone(), admin.to_string(), plan).unwrap();

        Contract::record_recovery_step(
            env.clone(),
            admin.to_string(),
            String::from_str(&env, "notified stakeholders"),
        )
        .unwrap();

        // Repay should be allowed in recovery mode
        let repay_result = Contract::repay(env.clone(), user.to_string(), 200);
        assert!(repay_result.is_ok());

        // Borrow should be restricted while in recovery
        let borrow_result = Contract::borrow(env.clone(), user.to_string(), 100);
        assert!(borrow_result.is_err());
        assert_eq!(
            borrow_result.unwrap_err(),
            ProtocolError::RecoveryModeRestricted
        );

        let state = Contract::get_emergency_state(env.clone()).unwrap();
        assert_eq!(state.status, EmergencyStatus::Recovery);
        assert_eq!(state.recovery_steps.len(), 1u32);
    });
}

#[test]
fn test_emergency_param_updates_apply() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = TestUtils::create_admin_address(&env);

    let contract_id = env.register(Contract, ());
    env.as_contract(&contract_id, || {
        Contract::initialize(env.clone(), admin.to_string()).unwrap();

        let base_rate_symbol = Symbol::new(&env, "base_rate");
        Contract::queue_emergency_param_update(
            env.clone(),
            admin.to_string(),
            base_rate_symbol,
            5000000,
        )
        .unwrap();
        Contract::apply_emergency_param_updates(env.clone(), admin.to_string()).unwrap();

        let config = InterestRateStorage::get_config(&env);
        assert_eq!(config.base_rate, 5000000);

        let state = Contract::get_emergency_state(env.clone()).unwrap();
        assert_eq!(state.pending_param_updates.len(), 0u32);
    });
}

#[test]
fn test_emergency_fund_management() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = TestUtils::create_admin_address(&env);
    let recipient = TestUtils::create_user_address(&env, 1);

    let contract_id = env.register(Contract, ());
    env.as_contract(&contract_id, || {
        Contract::initialize(env.clone(), admin.to_string()).unwrap();

        let token = Some(recipient.clone());
        Contract::adjust_emergency_fund(
            env.clone(),
            admin.to_string(),
            token.clone(),
            1_000_000,
            500_000,
        )
        .unwrap();

        let state = Contract::get_emergency_state(env.clone()).unwrap();
        assert_eq!(state.fund.balance, 1_000_000);
        assert_eq!(state.fund.reserved, 500_000);
        assert_eq!(state.fund.token, token);

        let err =
            Contract::adjust_emergency_fund(env.clone(), admin.to_string(), None, -2_000_000, 0)
                .unwrap_err();
        assert_eq!(err, ProtocolError::EmergencyFundInsufficient);
    });
}

#[test]
fn test_repay_success() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = TestUtils::create_admin_address(&env);
    let user = TestUtils::create_user_address(&env, 0);

    let contract_id = env.register(Contract, ());
    env.as_contract(&contract_id, || {
        // Initialize contract
        Contract::initialize(env.clone(), admin.to_string()).unwrap();
        TestUtils::verify_user(&env, &admin, &user);

        // Deposit and borrow
        Contract::deposit_collateral(env.clone(), user.to_string(), 2000).unwrap();
        Contract::borrow(env.clone(), user.to_string(), 1000).unwrap();

        // Test successful repayment
        let result = Contract::repay(env.clone(), user.to_string(), 500);
        assert!(result.is_ok());

        // Verify position
        let position = Contract::get_position(env.clone(), user.to_string()).unwrap();
        assert_eq!(position.0, 2000); // collateral
        assert_eq!(position.1, 500); // debt
    });
}

#[test]
fn test_repay_full_amount() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = TestUtils::create_admin_address(&env);
    let user = TestUtils::create_user_address(&env, 0);

    let contract_id = env.register(Contract, ());
    env.as_contract(&contract_id, || {
        // Initialize contract
        Contract::initialize(env.clone(), admin.to_string()).unwrap();
        TestUtils::verify_user(&env, &admin, &user);

        // Deposit and borrow
        Contract::deposit_collateral(env.clone(), user.to_string(), 2000).unwrap();
        Contract::borrow(env.clone(), user.to_string(), 1000).unwrap();

        // Test full repayment
        let result = Contract::repay(env.clone(), user.to_string(), 1000);
        assert!(result.is_ok());

        // Verify position
        let position = Contract::get_position(env.clone(), user.to_string()).unwrap();
        assert_eq!(position.0, 2000); // collateral
        assert_eq!(position.1, 0); // debt
    });
}

#[test]
fn test_withdraw_success() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = TestUtils::create_admin_address(&env);
    let user = TestUtils::create_user_address(&env, 0);

    let contract_id = env.register(Contract, ());
    env.as_contract(&contract_id, || {
        // Initialize contract
        Contract::initialize(env.clone(), admin.to_string()).unwrap();
        TestUtils::verify_user(&env, &admin, &user);

        // Deposit collateral
        Contract::deposit_collateral(env.clone(), user.to_string(), 2000).unwrap();

        // Test successful withdrawal
        let result = Contract::withdraw(env.clone(), user.to_string(), 1000);
        assert!(result.is_ok());

        // Verify position
        let position = Contract::get_position(env.clone(), user.to_string()).unwrap();
        assert_eq!(position.0, 1000); // collateral
        assert_eq!(position.1, 0); // debt
    });
}

#[test]
fn test_withdraw_insufficient_collateral() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = TestUtils::create_admin_address(&env);
    let user = TestUtils::create_user_address(&env, 0);

    let contract_id = env.register(Contract, ());
    env.as_contract(&contract_id, || {
        // Initialize contract
        Contract::initialize(env.clone(), admin.to_string()).unwrap();
        TestUtils::verify_user(&env, &admin, &user);

        // Deposit small amount
        Contract::deposit_collateral(env.clone(), user.to_string(), 100).unwrap();

        // Try to withdraw more than deposited
        let result = Contract::withdraw(env.clone(), user.to_string(), 200);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), ProtocolError::InsufficientCollateral);
    });
}

#[test]
fn test_withdraw_insufficient_collateral_ratio() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = TestUtils::create_admin_address(&env);
    let user = TestUtils::create_user_address(&env, 0);

    let contract_id = env.register(Contract, ());
    env.as_contract(&contract_id, || {
        // Initialize contract
        Contract::initialize(env.clone(), admin.to_string()).unwrap();
        TestUtils::verify_user(&env, &admin, &user);

        // Deposit and borrow
        Contract::deposit_collateral(env.clone(), user.to_string(), 2000).unwrap();
        Contract::borrow(env.clone(), user.to_string(), 1000).unwrap();

        // Try to withdraw too much (would make collateral ratio too low)
        let result = Contract::withdraw(env.clone(), user.to_string(), 1500);
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err(),
            ProtocolError::InsufficientCollateralRatio
        );
    });
}

#[test]
fn test_liquidate_success() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = TestUtils::create_admin_address(&env);
    let user = TestUtils::create_user_address(&env, 0);
    let liquidator = TestUtils::create_user_address(&env, 1);

    let contract_id = env.register(Contract, ());
    env.as_contract(&contract_id, || {
        // Initialize contract
        Contract::initialize(env.clone(), admin.to_string()).unwrap();
        TestUtils::verify_user(&env, &admin, &user);
        TestUtils::verify_user(&env, &admin, &liquidator);

        // Set a very low minimum collateral ratio for testing
        Contract::set_min_collateral_ratio(env.clone(), admin.to_string(), 50).unwrap();

        // Deposit collateral and borrow to create undercollateralized position
        Contract::deposit_collateral(env.clone(), user.to_string(), 1000).unwrap();
        Contract::borrow(env.clone(), user.to_string(), 1000).unwrap();

        // Now set the minimum ratio back to a higher value to make the position undercollateralized
        Contract::set_min_collateral_ratio(env.clone(), admin.to_string(), 150).unwrap();

        // Test successful liquidation
        let result =
            Contract::liquidate(env.clone(), liquidator.to_string(), user.to_string(), 500);
        assert!(result.is_ok());
    });
}

#[test]
fn test_liquidate_not_eligible() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = TestUtils::create_admin_address(&env);
    let user = TestUtils::create_user_address(&env, 0);
    let liquidator = TestUtils::create_user_address(&env, 1);

    let contract_id = env.register(Contract, ());
    env.as_contract(&contract_id, || {
        // Initialize contract
        Contract::initialize(env.clone(), admin.to_string()).unwrap();
        TestUtils::verify_user(&env, &admin, &user);
        TestUtils::verify_user(&env, &admin, &liquidator);

        // Deposit large amount and borrow small amount (healthy position)
        Contract::deposit_collateral(env.clone(), user.to_string(), 2000).unwrap();
        Contract::borrow(env.clone(), user.to_string(), 1000).unwrap();

        // Try to liquidate (should fail)
        let result =
            Contract::liquidate(env.clone(), liquidator.to_string(), user.to_string(), 500);
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err(),
            ProtocolError::NotEligibleForLiquidation
        );
    });
}

#[test]
fn test_set_risk_params() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = TestUtils::create_admin_address(&env);

    let contract_id = env.register(Contract, ());
    env.as_contract(&contract_id, || {
        // Initialize contract
        Contract::initialize(env.clone(), admin.to_string()).unwrap();

        // Test setting risk parameters
        let result = Contract::set_risk_params(env.clone(), admin.to_string(), 60000000, 15000000);
        assert!(result.is_ok());

        // Verify the parameters were set
        let risk_config = Contract::get_risk_config(env.clone()).unwrap();
        assert_eq!(risk_config.0, 60000000); // close_factor
        assert_eq!(risk_config.1, 15000000); // liquidation_incentive
    });
}

#[test]
fn test_set_risk_params_unauthorized() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = TestUtils::create_admin_address(&env);
    let user = TestUtils::create_user_address(&env, 0);

    let contract_id = env.register(Contract, ());
    env.as_contract(&contract_id, || {
        // Initialize contract
        Contract::initialize(env.clone(), admin.to_string()).unwrap();

        // Test setting risk parameters with non-admin (should fail)
        let result = Contract::set_risk_params(env.clone(), user.to_string(), 60000000, 15000000);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), ProtocolError::Unauthorized);
    });
}

#[test]
fn test_set_pause_switches() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = TestUtils::create_admin_address(&env);

    let contract_id = env.register(Contract, ());
    env.as_contract(&contract_id, || {
        // Initialize contract
        Contract::initialize(env.clone(), admin.to_string()).unwrap();

        // Test setting pause switches
        let result = Contract::set_pause_switches(
            env.clone(),
            admin.to_string(),
            true,  // pause_borrow
            false, // pause_deposit
            true,  // pause_withdraw
            false, // pause_liquidate
        );
        assert!(result.is_ok());

        // Verify the switches were set
        let risk_config = Contract::get_risk_config(env.clone()).unwrap();
        assert_eq!(risk_config.2, true); // pause_borrow
        assert_eq!(risk_config.3, false); // pause_deposit
        assert_eq!(risk_config.4, true); // pause_withdraw
        assert_eq!(risk_config.5, false); // pause_liquidate
    });
}

#[test]
fn test_get_protocol_params() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = TestUtils::create_admin_address(&env);

    let contract_id = env.register(Contract, ());
    env.as_contract(&contract_id, || {
        // Initialize contract
        Contract::initialize(env.clone(), admin.to_string()).unwrap();

        // Test getting protocol parameters
        let params = Contract::get_protocol_params(env.clone()).unwrap();
        assert_eq!(params.0, 2000000); // base_rate
        assert_eq!(params.1, 80000000); // kink_utilization
        assert_eq!(params.2, 10000000); // multiplier
        assert_eq!(params.3, 10000000); // reserve_factor
        assert_eq!(params.4, 50000000); // close_factor
        assert_eq!(params.5, 10000000); // liquidation_incentive
    });
}

#[test]
fn test_get_system_stats() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = TestUtils::create_admin_address(&env);

    let contract_id = env.register(Contract, ());
    env.as_contract(&contract_id, || {
        // Initialize contract
        Contract::initialize(env.clone(), admin.to_string()).unwrap();

        // Test getting system stats
        let stats = Contract::get_system_stats(env.clone()).unwrap();
        assert_eq!(stats.0, 0); // total_supplied
        assert_eq!(stats.1, 0); // total_borrowed
        assert_eq!(stats.2, 0); // current_borrow_rate
        assert_eq!(stats.3, 0); // current_supply_rate
    });
}

#[test]
fn test_get_position_not_found() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = TestUtils::create_admin_address(&env);
    let user = TestUtils::create_user_address(&env, 0);

    let contract_id = env.register(Contract, ());
    env.as_contract(&contract_id, || {
        // Initialize contract
        Contract::initialize(env.clone(), admin.to_string()).unwrap();

        // Test getting position for user who hasn't deposited
        let result = Contract::get_position(env.clone(), user.to_string());
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), ProtocolError::PositionNotFound);
    });
}

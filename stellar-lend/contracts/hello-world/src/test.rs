#![cfg(test)]

use super::*;
use soroban_sdk::{Env, String};

/// Helper to create a non-empty address string for tests
fn valid_address() -> String {
    String::from_str(&Env::default(), "user1")
}

#[test]
fn test_deposit_collateral_valid() {
    let env = Env::default();
    let depositor = valid_address();
    let amount = 100;
    // Should not panic
    Contract::deposit_collateral(env, depositor, amount);
}

#[test]
#[should_panic(expected = "Depositor address cannot be empty")]
fn test_deposit_collateral_empty_address() {
    let env = Env::default();
    let depositor = String::from_str(&env, "");
    let amount = 100;
    Contract::deposit_collateral(env, depositor, amount);
}

#[test]
#[should_panic(expected = "Deposit amount must be positive")]
fn test_deposit_collateral_zero_amount() {
    let env = Env::default();
    let depositor = valid_address();
    let amount = 0;
    Contract::deposit_collateral(env, depositor, amount);
}

#[test]
fn test_borrow_valid() {
    let env = Env::default();
    let borrower = valid_address();
    let amount = 50;
    Contract::borrow(env, borrower, amount);
}

#[test]
#[should_panic(expected = "Borrower address cannot be empty")]
fn test_borrow_empty_address() {
    let env = Env::default();
    let borrower = String::from_str(&env, "");
    let amount = 50;
    Contract::borrow(env, borrower, amount);
}

#[test]
#[should_panic(expected = "Borrow amount must be positive")]
fn test_borrow_negative_amount() {
    let env = Env::default();
    let borrower = valid_address();
    let amount = -10;
    Contract::borrow(env, borrower, amount);
}

#[test]
fn test_repay_valid() {
    let env = Env::default();
    let repayer = valid_address();
    let amount = 20;
    Contract::repay(env, repayer, amount);
}

#[test]
#[should_panic(expected = "Repayer address cannot be empty")]
fn test_repay_empty_address() {
    let env = Env::default();
    let repayer = String::from_str(&env, "");
    let amount = 20;
    Contract::repay(env, repayer, amount);
}

#[test]
#[should_panic(expected = "Repay amount must be positive")]
fn test_repay_zero_amount() {
    let env = Env::default();
    let repayer = valid_address();
    let amount = 0;
    Contract::repay(env, repayer, amount);
}

#[test]
fn test_withdraw_valid() {
    let env = Env::default();
    let withdrawer = valid_address();
    let amount = 30;
    Contract::withdraw(env, withdrawer, amount);
}

#[test]
#[should_panic(expected = "Withdrawer address cannot be empty")]
fn test_withdraw_empty_address() {
    let env = Env::default();
    let withdrawer = String::from_str(&env, "");
    let amount = 30;
    Contract::withdraw(env, withdrawer, amount);
}

#[test]
#[should_panic(expected = "Withdraw amount must be positive")]
fn test_withdraw_negative_amount() {
    let env = Env::default();
    let withdrawer = valid_address();
    let amount = -5;
    Contract::withdraw(env, withdrawer, amount);
}

#[test]
fn test_liquidate_valid() {
    let env = Env::default();
    let liquidator = valid_address();
    let amount = 40;
    Contract::liquidate(env, liquidator, amount);
}

#[test]
#[should_panic(expected = "Liquidator address cannot be empty")]
fn test_liquidate_empty_address() {
    let env = Env::default();
    let liquidator = String::from_str(&env, "");
    let amount = 40;
    Contract::liquidate(env, liquidator, amount);
}

#[test]
#[should_panic(expected = "Liquidation amount must be positive")]
fn test_liquidate_zero_amount() {
    let env = Env::default();
    let liquidator = valid_address();
    let amount = 0;
    Contract::liquidate(env, liquidator, amount);
}
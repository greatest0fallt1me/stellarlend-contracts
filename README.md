# StellarLend Smart Contracts

## Overview

StellarLend is a decentralized finance (DeFi) lending protocol built on the Stellar blockchain using Soroban smart contracts. The protocol enables users to deposit collateral, borrow assets, accrue interest, and participate in a secure, transparent, and risk-managed lending market.

---

## Features

- **Collateralized Lending**: Users can deposit collateral and borrow against it.
- **Dynamic Interest Rate Model**: Interest rates adjust based on protocol utilization.
- **Oracle Integration**: Real-time price feeds with validation and fallback mechanisms.
- **Risk Management**: Admin-configurable risk parameters, pause switches, and advanced liquidation logic.
- **Partial Liquidation**: Supports close factor and liquidation incentive for liquidators.
- **Comprehensive Event Logging**: Emits events for all major protocol actions.
- **Admin Controls**: Secure admin functions for protocol configuration and emergency actions.

---

## Getting Started

### Prerequisites
- [Rust](https://www.rust-lang.org/tools/install)
- [Cargo](https://doc.rust-lang.org/cargo/getting-started/installation.html)
- [Soroban CLI](https://soroban.stellar.org/docs/getting-started/installation)

### Setup
```bash
# Clone the repository
$ git clone <repo-url>
$ cd stellarlend-contracts/stellar-lend
```

### Build
```bash
$ cargo build --release
```

### Test
```bash
$ cargo test
```

---

## Contract Modules

- **Lending**: Deposit, borrow, repay, and withdraw collateral.
- **Interest Rate Model**: Dynamic rates based on utilization, accrual on every action.
- **Oracle Integration**: Fetches and validates price data, supports fallback and heartbeat.
- **Risk Management**: Admin-settable close factor, liquidation incentive, and pause switches.
- **Liquidation**: Partial liquidation, incentive for liquidators, protocol safety checks.

---

## Entrypoints & Usage

| Function                      | Description                                      |
|-------------------------------|--------------------------------------------------|
| `initialize`                  | Initialize contract and set admin                 |
| `deposit_collateral`          | Deposit collateral to the protocol                |
| `borrow`                      | Borrow assets against collateral                  |
| `repay`                       | Repay borrowed assets                            |
| `withdraw`                    | Withdraw collateral                              |
| `liquidate`                   | Liquidate undercollateralized positions          |
| `set_risk_params`             | Admin: Set close factor and liquidation incentive |
| `set_pause_switches`          | Admin: Pause/unpause protocol actions            |
| `set_oracle`                  | Admin: Set oracle address                        |
| `set_min_collateral_ratio`    | Admin: Set minimum collateral ratio              |
| `set_base_rate`               | Admin: Set base interest rate                    |
| `set_kink_utilization`        | Admin: Set kink utilization point                |
| `set_multiplier`              | Admin: Set interest rate multiplier              |
| `set_reserve_factor`          | Admin: Set protocol reserve factor               |
| `set_rate_limits`             | Admin: Set interest rate floor/ceiling           |
| `emergency_rate_adjustment`   | Admin: Emergency interest rate adjustment        |
| `get_position`                | Query user position (collateral, debt, ratio)    |
| `get_protocol_params`         | Query protocol parameters                        |
| `get_risk_config`             | Query risk management configuration              |
| `get_system_stats`            | Query system-wide stats                          |

---

## Contributing

Contributions are welcome! Please open issues and pull requests for bug fixes, improvements, or new features. For major changes, please discuss them in an issue first.

---

## License

This project is licensed under the MIT License. See the [LICENSE](LICENSE) file for details.

---

## Links & Resources

- [Stellar Soroban Documentation](https://soroban.stellar.org/docs/)
- [Stellar Developers](https://developers.stellar.org/)
- [Rust Programming Language](https://www.rust-lang.org/)
- [Soroban Examples](https://github.com/stellar/soroban-examples)

# MR: Harden Interest Accrual Math and Document Units (#150)

Problem
- Interest accrual used 1e8 scaling without clear documentation and lacked overflow protections, risking incorrect balances over long durations.

Changes
- Added saturating math to interest accrual and rate computations in `InterestRateStorage::update_state` and `InterestRateManager::accrue_interest_for_position`.
- Documented units, scales, and formulas (rates scaled by 1e8, per-year seconds, utilization scale).
- Clamped rates to [0, 1e8] during accrual to avoid pathological inputs.

Acceptance
- cargo build/test successful.
- Existing tests pass; added docs in code to clarify units and prevent misuse.

Notes
- Follow-up: property-based tests for extreme rate/time combinations can be expanded in a dedicated test module if required.
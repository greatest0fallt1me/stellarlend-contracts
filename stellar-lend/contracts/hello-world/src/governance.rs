#![allow(dead_code)]
use soroban_sdk::{contracttype, Address, Env, Map, Symbol, Vec};

#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct Proposal {
    pub id: u64,
    pub proposer: Address,
    pub title: soroban_sdk::String,
    pub created: u64,
    pub voting_ends: u64,
    pub queued_until: u64,
    pub for_votes: i128,
    pub against_votes: i128,
    pub executed: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct VoteReceipt {
    pub voter: Address,
    pub support: bool,
    pub weight: i128,
}

pub struct GovStorage;

impl GovStorage {
    fn proposals_key(env: &Env) -> Symbol { Symbol::new(env, "gov_proposals") }
    fn receipts_key(env: &Env) -> Symbol { Symbol::new(env, "gov_receipts") }
    fn counter_key(env: &Env) -> Symbol { Symbol::new(env, "gov_counter") }
    fn quorum_bps_key(env: &Env) -> Symbol { Symbol::new(env, "gov_quorum_bps") }
    fn timelock_key(env: &Env) -> Symbol { Symbol::new(env, "gov_timelock") }
    fn delegation_key(env: &Env) -> Symbol { Symbol::new(env, "gov_delegation") }

    pub fn next_id(env: &Env) -> u64 {
        let id: u64 = env.storage().instance().get(&Self::counter_key(env)).unwrap_or(0);
        env.storage().instance().set(&Self::counter_key(env), &(id + 1));
        id + 1
    }

    pub fn save_proposal(env: &Env, p: &Proposal) {
        let mut map: Map<u64, Proposal> = env.storage().instance().get(&Self::proposals_key(env)).unwrap_or_else(|| Map::new(env));
        map.set(p.id, p.clone());
        env.storage().instance().set(&Self::proposals_key(env), &map);
    }

    pub fn get_proposal(env: &Env, id: u64) -> Option<Proposal> {
        let map: Map<u64, Proposal> = env.storage().instance().get(&Self::proposals_key(env)).unwrap_or_else(|| Map::new(env));
        map.get(id)
    }

    pub fn save_receipt(env: &Env, id: u64, r: &VoteReceipt) {
        let key = (Self::receipts_key(env), id);
        let mut map: Map<Address, VoteReceipt> = env.storage().instance().get(&key).unwrap_or_else(|| Map::new(env));
        map.set(r.voter.clone(), r.clone());
        env.storage().instance().set(&key, &map);
    }

    pub fn get_quorum_bps(env: &Env) -> i128 { env.storage().instance().get(&Self::quorum_bps_key(env)).unwrap_or(1000) }
    pub fn set_quorum_bps(env: &Env, bps: i128) { env.storage().instance().set(&Self::quorum_bps_key(env), &bps); }
    pub fn get_timelock(env: &Env) -> u64 { env.storage().instance().get(&Self::timelock_key(env)).unwrap_or(60) }
    pub fn set_timelock(env: &Env, secs: u64) { env.storage().instance().set(&Self::timelock_key(env), &secs); }
}

pub struct Governance;

impl Governance {
    pub fn propose(env: &Env, proposer: &Address, title: soroban_sdk::String, voting_period_secs: u64) -> Proposal {
        let now = env.ledger().timestamp();
        let id = GovStorage::next_id(env);
        let p = Proposal { id, proposer: proposer.clone(), title, created: now, voting_ends: now + voting_period_secs, queued_until: 0, for_votes: 0, against_votes: 0, executed: false };
        GovStorage::save_proposal(env, &p);
        p
    }

    pub fn vote(env: &Env, id: u64, voter: &Address, support: bool, weight: i128) -> Proposal {
        let mut p = GovStorage::get_proposal(env, id).unwrap();
        if env.ledger().timestamp() > p.voting_ends { return p; }
        if support { p.for_votes += weight; } else { p.against_votes += weight; }
        GovStorage::save_receipt(env, id, &VoteReceipt { voter: voter.clone(), support, weight });
        GovStorage::save_proposal(env, &p);
        p
    }

    pub fn queue(env: &Env, id: u64) -> Proposal {
        let mut p = GovStorage::get_proposal(env, id).unwrap();
        let now = env.ledger().timestamp();
        let quorum = GovStorage::get_quorum_bps(env);
        let total = p.for_votes + p.against_votes;
        let have_quorum = if total == 0 { false } else { (p.for_votes * 10000 / total) >= quorum };
        if have_quorum && now >= p.voting_ends { p.queued_until = now + GovStorage::get_timelock(env); }
        GovStorage::save_proposal(env, &p);
        p
    }

    pub fn execute(env: &Env, id: u64) -> Proposal {
        let mut p = GovStorage::get_proposal(env, id).unwrap();
        let now = env.ledger().timestamp();
        if now >= p.queued_until && p.queued_until != 0 { p.executed = true; }
        GovStorage::save_proposal(env, &p);
        p
    }

    pub fn delegate(env: &Env, from: &Address, to: &Address) {
        let key = (GovStorage::delegation_key(env), from.clone());
        env.storage().instance().set(&key, to);
    }

    pub fn get_delegate(env: &Env, from: &Address) -> Option<Address> {
        let key = (GovStorage::delegation_key(env), from.clone());
        env.storage().instance().get(&key)
    }
}

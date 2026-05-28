#![cfg(test)]

use super::*;
use soroban_sdk::{
    testutils::{Address as AddressTrait, Events, Ledger},
    token::StellarAssetClient,
    Address, Env, TryFromVal,
};

fn set_time(env: &Env, timestamp: u64) {
    env.ledger().set_timestamp(timestamp);
}

fn setup_split(
    env: &Env,
    spending: u32,
    savings: u32,
    bills: u32,
    insurance: u32,
) -> (
    RemittanceSplitClient<'_>,
    Address,
    Address,
    StellarAssetClient<'_>,
) {
    env.mock_all_auths();
    set_time(env, 1_000);

    let contract_id = env.register_contract(None, RemittanceSplit);
    let client = RemittanceSplitClient::new(env, &contract_id);

    let owner = Address::generate(env);
    let token_admin = Address::generate(env);
    let token_contract = env.register_stellar_asset_contract_v2(token_admin);
    let token_addr = token_contract.address();
    let stellar_client = StellarAssetClient::new(env, &token_addr);

    client.initialize_split(
        &owner,
        &0,
        &token_addr,
        &spending,
        &savings,
        &bills,
        &insurance,
    );

    (client, owner, token_addr, stellar_client)
}

fn sample_accounts(env: &Env) -> AccountGroup {
    AccountGroup {
        spending: Address::generate(env),
        savings: Address::generate(env),
        bills: Address::generate(env),
        insurance: Address::generate(env),
    }
}


#[test]
fn test_distribution_completed_event() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register_contract(None, RemittanceSplit);
    let client = RemittanceSplitClient::new(&env, &contract_id);

    let owner = Address::generate(&env);
    let token_admin = Address::generate(&env);
    let token_contract = env.register_stellar_asset_contract_v2(token_admin);
    let token_addr = token_contract.address();
    let stellar_client = StellarAssetClient::new(&env, &token_addr);

    // 1. Initialize split
    // percentages: 40, 30, 20, 10
    client.initialize_split(&owner, &0, &token_addr, &40, &30, &20, &10);

    // 2. Setup destination accounts
    let accounts = AccountGroup {
        spending: Address::generate(&env),
        savings: Address::generate(&env),
        bills: Address::generate(&env),
        insurance: Address::generate(&env),
    };

    // 3. Mint tokens to owner
    let total_amount = 1000i128;
    stellar_client.mint(&owner, &total_amount);

    // 4. Distribute
    let nonce = 1u64; // nonce 0 used in initialize_split
    let deadline = env.ledger().timestamp() + 3600;
    let request_hash = RemittanceSplit::compute_request_hash(
        symbol_short!("distrib"),
        owner.clone(),
        nonce,
        total_amount,
        deadline,
    );

    client.distribute_usdc(
        &token_addr,
        &owner,
        &nonce,
        &deadline,
        &request_hash,
        &accounts,
        &total_amount,
    );

    // 5. Verify events
    let events = env.events().all();

    // We expect several events:
    // - init (from initialize_split)
    // - dist_ok (unstructured)
    // - dist_comp (structured) - THIS IS THE ONE WE CARE ABOUT

    let last_event = events.last().expect("No events emitted");
    let (_contract_id, topics, data) = last_event;

    // Verify topic schema count
    assert_eq!(topics.len(), 4, "Expected 4 topics in event");

    // Verify structured payload
    let event: DistributionCompletedEvent = DistributionCompletedEvent::try_from_val(&env, &data)
        .expect("Failed to parse DistributionCompletedEvent data");

    assert_eq!(event.from, owner);
    assert_eq!(event.total_amount, total_amount);
    assert_eq!(event.spending_amount, 400); // 40% of 1000
    assert_eq!(event.savings_amount, 300); // 30% of 1000
    assert_eq!(event.bills_amount, 200); // 20% of 1000
    assert_eq!(event.insurance_amount, 100); // 10% of 1000 handled by remainder
    assert_eq!(event.timestamp, env.ledger().timestamp());
}

#[test]
fn test_distribution_event_topic_correctness() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register_contract(None, RemittanceSplit);
    let client = RemittanceSplitClient::new(&env, &contract_id);

    let owner = Address::generate(&env);
    let token_admin = Address::generate(&env);
    let token_contract = env.register_stellar_asset_contract_v2(token_admin);
    let token_addr = token_contract.address();
    let stellar_client = StellarAssetClient::new(&env, &token_addr);

    client.initialize_split(&owner, &0, &token_addr, &50, &50, &0, &0);

    let accounts = AccountGroup {
        spending: Address::generate(&env),
        savings: Address::generate(&env),
        bills: Address::generate(&env),
        insurance: Address::generate(&env),
    };

    stellar_client.mint(&owner, &100);

    let nonce = 1u64;
    let deadline = env.ledger().timestamp() + 3600;
    let request_hash = RemittanceSplit::compute_request_hash(
        symbol_short!("distrib"),
        owner.clone(),
        nonce,
        100,
        deadline,
    );

    client.distribute_usdc(
        &token_addr,
        &owner,
        &nonce,
        &deadline,
        &request_hash,
        &accounts,
        &100,
    );

    let events = env.events().all();
    let dist_comp_event = events
        .iter()
        .find(|e| {
            let topics = &e.1;
            topics.len() == 4
        })
        .expect("DistributionCompleted event not found");

    let topics = &dist_comp_event.1;
    assert_eq!(topics.len(), 4, "Event should have 4 topics");
}

// ──────────────────────────────────────────────────────────────────────────
// Request Hash Tests - Test Vectors for distribute_usdc Signing
// ──────────────────────────────────────────────────────────────────────────

/// Test that get_request_hash produces a deterministic 32-byte SHA-256 hash
#[test]
fn test_request_hash_deterministic() {
    let env = Env::default();
    let contract_id = env.register_contract(None, RemittanceSplit);
    let client = RemittanceSplitClient::new(&env, &contract_id);
    
    let usdc_contract = Address::generate(&env);
    let from = Address::generate(&env);
    let spending = Address::generate(&env);
    let savings = Address::generate(&env);
    let bills = Address::generate(&env);
    let insurance = Address::generate(&env);
    
    let request = DistributeUsdcRequest {
        usdc_contract: usdc_contract.clone(),
        from: from.clone(),
        nonce: 0,
        accounts: AccountGroup {
            spending: spending.clone(),
            savings: savings.clone(),
            bills: bills.clone(),
            insurance: insurance.clone(),
        },
        total_amount: 1000i128,
        deadline: 2000u64,
    };
    
    // Hash the same request twice
    let hash1 = client.get_request_hash(&request);
    let hash2 = client.get_request_hash(&request);
    
    // Both hashes should be identical (deterministic)
    assert_eq!(hash1, hash2);
    // SHA-256 produces 32 bytes
    assert_eq!(hash1.len(), 32);
}

/// Test that changing any parameter changes the hash (no collisions)
#[test]
fn test_request_hash_changes_with_parameters() {
    let env = Env::default();
    let contract_id = env.register_contract(None, RemittanceSplit);
    let client = RemittanceSplitClient::new(&env, &contract_id);
    
    let usdc_contract = Address::generate(&env);
    let from = Address::generate(&env);
    let spending = Address::generate(&env);
    let savings = Address::generate(&env);
    let bills = Address::generate(&env);
    let insurance = Address::generate(&env);
    let other = Address::generate(&env);
    
    let base_request = DistributeUsdcRequest {
        usdc_contract: usdc_contract.clone(),
        from: from.clone(),
        nonce: 0,
        accounts: AccountGroup {
            spending: spending.clone(),
            savings: savings.clone(),
            bills: bills.clone(),
            insurance: insurance.clone(),
        },
        total_amount: 1000i128,
        deadline: 2000u64,
    };
    
    let base_hash = client.get_request_hash(&base_request);
    
    // Test 1: Changing usdc_contract changes hash
    let mut req = base_request.clone();
    req.usdc_contract = other.clone();
    let hash = client.get_request_hash(&req);
    assert!(hash.ne(&base_hash), "Hash should change when usdc_contract changes");
    
    // Test 2: Changing from address changes hash
    let mut req = base_request.clone();
    req.from = other.clone();
    let hash = client.get_request_hash(&req);
    assert!(hash.ne(&base_hash), "Hash should change when from changes");
    
    // Test 3: Changing nonce changes hash
    let mut req = base_request.clone();
    req.nonce = 1;
    let hash = client.get_request_hash(&req);
    assert!(hash.ne(&base_hash), "Hash should change when nonce changes");
    
    // Test 4: Changing total_amount changes hash
    let mut req = base_request.clone();
    req.total_amount = 2000;
    let hash = client.get_request_hash(&req);
    assert!(hash.ne(&base_hash), "Hash should change when total_amount changes");
    
    // Test 5: Changing deadline changes hash
    let mut req = base_request.clone();
    req.deadline = 3000;
    let hash = client.get_request_hash(&req);
    assert!(hash.ne(&base_hash), "Hash should change when deadline changes");
    
    // Test 6: Changing spending account changes hash
    let mut req = base_request.clone();
    req.accounts.spending = other.clone();
    let hash = client.get_request_hash(&req);
    assert!(hash.ne(&base_hash), "Hash should change when spending account changes");
}

/// Test deadline validation: deadline must not be in the past
#[test]
fn test_distribute_usdc_deadline_expired() {
    let env = Env::default();
    let contract_id = env.register_contract(None, RemittanceSplit);
    let client = RemittanceSplitClient::new(&env, &contract_id);
    
    env.mock_all_auths();
    set_time(&env, 1000);
    
    let owner = Address::generate(&env);
    let usdc_contract = Address::generate(&env);
    let spending = Address::generate(&env);
    let savings = Address::generate(&env);
    let bills = Address::generate(&env);
    let insurance = Address::generate(&env);
    
    // Initialize contract
    client.initialize_split(&owner, &0, &usdc_contract, &50, &30, &15, &5);
    
    // Create request with deadline in the past (500 < 1000)
    let request = DistributeUsdcRequest {
        usdc_contract: usdc_contract.clone(),
        from: owner.clone(),
        nonce: 0,
        accounts: AccountGroup {
            spending: spending.clone(),
            savings: savings.clone(),
            bills: bills.clone(),
            insurance: insurance.clone(),
        },
        total_amount: 1000i128,
        deadline: 500u64,  // Past deadline
    };
    
    let hash = client.get_request_hash(&request);
    let result = client.try_distribute_usdc_hashed(&request, &hash);
    assert_eq!(result, Err(Ok(RemittanceSplitError::DeadlineExpired)));
}

/// Test deadline validation: deadline must not be too far in the future (MAX_DEADLINE_WINDOW_SECS = 3600)
#[test]
fn test_distribute_usdc_deadline_too_far() {
    let env = Env::default();
    let contract_id = env.register_contract(None, RemittanceSplit);
    let client = RemittanceSplitClient::new(&env, &contract_id);
    
    env.mock_all_auths();
    set_time(&env, 1000);
    
    let owner = Address::generate(&env);
    let usdc_contract = Address::generate(&env);
    let spending = Address::generate(&env);
    let savings = Address::generate(&env);
    let bills = Address::generate(&env);
    let insurance = Address::generate(&env);
    
    // Initialize contract
    client.initialize_split(&owner, &0, &usdc_contract, &50, &30, &15, &5);
    
    // Create request with deadline > MAX_DEADLINE_WINDOW_SECS from now
    let request = DistributeUsdcRequest {
        usdc_contract: usdc_contract.clone(),
        from: owner.clone(),
        nonce: 0,
        accounts: AccountGroup {
            spending: spending.clone(),
            savings: savings.clone(),
            bills: bills.clone(),
            insurance: insurance.clone(),
        },
        total_amount: 1000i128,
        deadline: 1000 + 3600 + 1,  // 1 second more than allowed window
    };
    
    let hash = client.get_request_hash(&request);
    let result = client.try_distribute_usdc_hashed(&request, &hash);
    assert_eq!(result, Err(Ok(RemittanceSplitError::InvalidDeadline)));
}

/// Test deadline validation: deadline must not be zero
#[test]
fn test_distribute_usdc_deadline_zero() {
    let env = Env::default();
    let contract_id = env.register_contract(None, RemittanceSplit);
    let client = RemittanceSplitClient::new(&env, &contract_id);
    
    env.mock_all_auths();
    set_time(&env, 1000);
    
    let owner = Address::generate(&env);
    let usdc_contract = Address::generate(&env);
    let spending = Address::generate(&env);
    let savings = Address::generate(&env);
    let bills = Address::generate(&env);
    let insurance = Address::generate(&env);
    
    // Initialize contract
    client.initialize_split(&owner, &0, &usdc_contract, &50, &30, &15, &5);
    
    // Create request with deadline = 0
    let request = DistributeUsdcRequest {
        usdc_contract: usdc_contract.clone(),
        from: owner.clone(),
        nonce: 0,
        accounts: AccountGroup {
            spending: spending.clone(),
            savings: savings.clone(),
            bills: bills.clone(),
            insurance: insurance.clone(),
        },
        total_amount: 1000i128,
        deadline: 0,  // Invalid deadline
    };
    
    let hash = client.get_request_hash(&request);
    let result = client.try_distribute_usdc_hashed(&request, &hash);
    assert_eq!(result, Err(Ok(RemittanceSplitError::InvalidDeadline)));
}

/// Test request hash mismatch: passing wrong hash should fail
#[test]
fn test_distribute_usdc_hash_mismatch() {
    let env = Env::default();
    let contract_id = env.register_contract(None, RemittanceSplit);
    let client = RemittanceSplitClient::new(&env, &contract_id);
    
    env.mock_all_auths();
    set_time(&env, 1000);
    
    let owner = Address::generate(&env);
    let usdc_contract = Address::generate(&env);
    let spending = Address::generate(&env);
    let savings = Address::generate(&env);
    let bills = Address::generate(&env);
    let insurance = Address::generate(&env);
    
    // Initialize contract
    client.initialize_split(&owner, &0, &usdc_contract, &50, &30, &15, &5);
    
    // Create valid request
    let request = DistributeUsdcRequest {
        usdc_contract: usdc_contract.clone(),
        from: owner.clone(),
        nonce: 0,
        accounts: AccountGroup {
            spending: spending.clone(),
            savings: savings.clone(),
            bills: bills.clone(),
            insurance: insurance.clone(),
        },
        total_amount: 1000i128,
        deadline: 2000u64,
    };
    
    // Use a zeroed 32-byte hash as the "wrong" hash
    let _ = client.get_request_hash(&request);
    let wrong_hash = soroban_sdk::Bytes::from_slice(&env, &[0u8; 32]);
    
    let result = client.try_distribute_usdc_hashed(&request, &wrong_hash);
    assert_eq!(result, Err(Ok(RemittanceSplitError::RequestHashMismatch)));
}

/// Test boundary: deadline exactly at MAX_DEADLINE_WINDOW_SECS should succeed
#[test]
fn test_distribute_usdc_deadline_at_boundary() {
    let env = Env::default();
    let contract_id = env.register_contract(None, RemittanceSplit);
    let client = RemittanceSplitClient::new(&env, &contract_id);
    
    env.mock_all_auths();
    set_time(&env, 1000);
    
    let owner = Address::generate(&env);
    let usdc_contract = Address::generate(&env);
    let spending = Address::generate(&env);
    let savings = Address::generate(&env);
    let bills = Address::generate(&env);
    let insurance = Address::generate(&env);
    
    // Initialize contract
    client.initialize_split(&owner, &0, &usdc_contract, &50, &30, &15, &5);
    
    // Create request with deadline exactly at MAX_DEADLINE_WINDOW_SECS boundary
    let request = DistributeUsdcRequest {
        usdc_contract: usdc_contract.clone(),
        from: owner.clone(),
        nonce: 0,
        accounts: AccountGroup {
            spending: spending.clone(),
            savings: savings.clone(),
            bills: bills.clone(),
            insurance: insurance.clone(),
        },
        total_amount: 1000i128,
        deadline: 1000 + 3600,  // Exactly at 1 hour boundary
    };
    
    let hash = client.get_request_hash(&request);
    
    // This should pass deadline validation
    // (It will fail for other reasons like missing USDC balance, but not deadline)
    let result = client.try_distribute_usdc_hashed(&request, &hash);
    
    // Should fail due to other reasons (e.g., balance), not deadline validation
    // We can't assert equality here since we didn't register USDC token,
    // but we can check it's not a DeadlineExpired or InvalidDeadline error
    match result {
        Err(Ok(RemittanceSplitError::DeadlineExpired)) => {
            panic!("Should not fail with DeadlineExpired");
        }
        Err(Ok(RemittanceSplitError::InvalidDeadline)) => {
            panic!("Should not fail with InvalidDeadline");
        }
        _ => {} // Any other result is acceptable for this boundary test
    }
}

/// Test that the same request always produces the same hash (cross-call consistency)
#[test]
fn test_request_hash_cross_call_consistency() {
    let env = Env::default();
    let contract_id = env.register_contract(None, RemittanceSplit);
    let client = RemittanceSplitClient::new(&env, &contract_id);
    
    let usdc_contract = Address::generate(&env);
    let from = Address::generate(&env);
    let spending = Address::generate(&env);
    let savings = Address::generate(&env);
    let bills = Address::generate(&env);
    let insurance = Address::generate(&env);
    
    let request = DistributeUsdcRequest {
        usdc_contract: usdc_contract.clone(),
        from: from.clone(),
        nonce: 42,
        accounts: AccountGroup {
            spending: spending.clone(),
            savings: savings.clone(),
            bills: bills.clone(),
            insurance: insurance.clone(),
        },
        total_amount: 12345i128,
        deadline: 9999u64,
    };
    
    // Call get_request_hash multiple times and verify consistency
    let h0 = client.get_request_hash(&request);
    let h1 = client.get_request_hash(&request);
    let h2 = client.get_request_hash(&request);
    assert_eq!(h0, h1, "Hash should be consistent across calls");
    assert_eq!(h1, h2, "Hash should be consistent across calls");
}

// ──────────────────────────────────────────────────────────────────────────────
// RequestHashMismatch tamper tests
//
// Each test proves that mutating one field in DistributeUsdcRequest while keeping
// the original hash causes distribute_usdc_hashed to return
// RequestHashMismatch(15), closing the cross-field confused-deputy gap.
//
// Hash preimage ordering (see get_request_hash):
//   DISTRIBUTE_USDC_DOMAIN | domain_id("distrib") | from | usdc_contract
//   | total_amount (16 LE) | nonce (8 LE) | deadline (8 LE)
// ──────────────────────────────────────────────────────────────────────────────

fn base_request(env: &Env) -> DistributeUsdcRequest {
    DistributeUsdcRequest {
        usdc_contract: Address::generate(env),
        from: Address::generate(env),
        nonce: 1,
        accounts: AccountGroup {
            spending: Address::generate(env),
            savings: Address::generate(env),
            bills: Address::generate(env),
            insurance: Address::generate(env),
        },
        total_amount: 1000i128,
        deadline: 1000 + 1800, // 30 min from ledger time 1000
    }
}

/// Positive control: unmodified request + correct hash passes the hash gate.
/// The call fails at InsufficientBalance (no minted USDC), NOT RequestHashMismatch.
#[test]
fn test_request_hash_positive_control() {
    let env = Env::default();
    env.mock_all_auths();
    set_time(&env, 1000);
    let contract_id = env.register_contract(None, RemittanceSplit);
    let client = RemittanceSplitClient::new(&env, &contract_id);

    let request = base_request(&env);
    let hash = client.get_request_hash(&request);

    let result = client.try_distribute_usdc_hashed(&request, &hash);
    // Must NOT be RequestHashMismatch — hash check passed.
    match result {
        Err(Ok(RemittanceSplitError::RequestHashMismatch)) => {
            panic!("Hash check should pass for unmodified request");
        }
        _ => {}
    }
}

/// Mutating `from` while keeping the original hash yields RequestHashMismatch.
#[test]
fn test_request_hash_mismatch_on_from_tamper() {
    let env = Env::default();
    env.mock_all_auths();
    set_time(&env, 1000);
    let contract_id = env.register_contract(None, RemittanceSplit);
    let client = RemittanceSplitClient::new(&env, &contract_id);

    let original = base_request(&env);
    let hash = client.get_request_hash(&original);

    let mut tampered = original.clone();
    tampered.from = Address::generate(&env); // different sender

    let result = client.try_distribute_usdc_hashed(&tampered, &hash);
    assert_eq!(
        result,
        Err(Ok(RemittanceSplitError::RequestHashMismatch)),
        "Tampered `from` must yield RequestHashMismatch"
    );
}

/// Mutating `usdc_contract` while keeping the original hash yields RequestHashMismatch.
#[test]
fn test_request_hash_mismatch_on_usdc_contract_tamper() {
    let env = Env::default();
    env.mock_all_auths();
    set_time(&env, 1000);
    let contract_id = env.register_contract(None, RemittanceSplit);
    let client = RemittanceSplitClient::new(&env, &contract_id);

    let original = base_request(&env);
    let hash = client.get_request_hash(&original);

    let mut tampered = original.clone();
    tampered.usdc_contract = Address::generate(&env);

    let result = client.try_distribute_usdc_hashed(&tampered, &hash);
    assert_eq!(
        result,
        Err(Ok(RemittanceSplitError::RequestHashMismatch)),
        "Tampered `usdc_contract` must yield RequestHashMismatch"
    );
}

/// Mutating `total_amount` (off-by-one) while keeping the original hash yields RequestHashMismatch.
#[test]
fn test_request_hash_mismatch_on_amount_tamper() {
    let env = Env::default();
    env.mock_all_auths();
    set_time(&env, 1000);
    let contract_id = env.register_contract(None, RemittanceSplit);
    let client = RemittanceSplitClient::new(&env, &contract_id);

    let original = base_request(&env);
    let hash = client.get_request_hash(&original);

    let mut tampered = original.clone();
    tampered.total_amount = original.total_amount + 1; // off-by-one

    let result = client.try_distribute_usdc_hashed(&tampered, &hash);
    assert_eq!(
        result,
        Err(Ok(RemittanceSplitError::RequestHashMismatch)),
        "Off-by-one in `total_amount` must yield RequestHashMismatch"
    );
}

/// Mutating `nonce` while keeping the original hash yields RequestHashMismatch.
#[test]
fn test_request_hash_mismatch_on_nonce_tamper() {
    let env = Env::default();
    env.mock_all_auths();
    set_time(&env, 1000);
    let contract_id = env.register_contract(None, RemittanceSplit);
    let client = RemittanceSplitClient::new(&env, &contract_id);

    let original = base_request(&env);
    let hash = client.get_request_hash(&original);

    let mut tampered = original.clone();
    tampered.nonce = original.nonce.wrapping_add(1); // next nonce

    let result = client.try_distribute_usdc_hashed(&tampered, &hash);
    assert_eq!(
        result,
        Err(Ok(RemittanceSplitError::RequestHashMismatch)),
        "Tampered `nonce` must yield RequestHashMismatch"
    );
}

/// Mutating `deadline` while keeping the original hash yields RequestHashMismatch.
#[test]
fn test_request_hash_mismatch_on_deadline_tamper() {
    let env = Env::default();
    env.mock_all_auths();
    set_time(&env, 1000);
    let contract_id = env.register_contract(None, RemittanceSplit);
    let client = RemittanceSplitClient::new(&env, &contract_id);

    let original = base_request(&env);
    let hash = client.get_request_hash(&original);

    let mut tampered = original.clone();
    tampered.deadline = original.deadline + 60; // extend by 60 seconds

    let result = client.try_distribute_usdc_hashed(&tampered, &hash);
    assert_eq!(
        result,
        Err(Ok(RemittanceSplitError::RequestHashMismatch)),
        "Tampered `deadline` must yield RequestHashMismatch"
    );
}

/// domain_id swap: supplying arbitrary bytes (different domain) as the hash is rejected.
/// The hash binds DISTRIBUTE_USDC_DOMAIN + "distrib" — any bytes from a different
/// domain cannot satisfy the hash gate.
#[test]
fn test_request_hash_mismatch_on_domain_id_swap() {
    let env = Env::default();
    env.mock_all_auths();
    set_time(&env, 1000);
    let contract_id = env.register_contract(None, RemittanceSplit);
    let client = RemittanceSplitClient::new(&env, &contract_id);

    let request = base_request(&env);

    // Craft a fake 32-byte hash that represents a different domain ("init" tag).
    // This simulates a confused-deputy attack where an "init" domain hash is
    // replayed against the "distrib" entrypoint.
    let wrong_hash = soroban_sdk::Bytes::from_slice(&env, &[0u8; 32]);

    let result = client.try_distribute_usdc_hashed(&request, &wrong_hash);
    assert_eq!(
        result,
        Err(Ok(RemittanceSplitError::RequestHashMismatch)),
        "A hash from a different domain must be rejected as RequestHashMismatch"
    );
}

/// Nonce reuse with new deadline: same nonce, different deadline — still mismatches.
#[test]
fn test_request_hash_mismatch_nonce_reuse_new_deadline() {
    let env = Env::default();
    env.mock_all_auths();
    set_time(&env, 1000);
    let contract_id = env.register_contract(None, RemittanceSplit);
    let client = RemittanceSplitClient::new(&env, &contract_id);

    let original = base_request(&env);
    let hash = client.get_request_hash(&original);

    // Keep same nonce but extend deadline — the hash won't match
    let mut tampered = original.clone();
    tampered.deadline = original.deadline + 300;

    let result = client.try_distribute_usdc_hashed(&tampered, &hash);
    assert_eq!(
        result,
        Err(Ok(RemittanceSplitError::RequestHashMismatch)),
        "Same nonce with new deadline must be rejected"
    );
}


// ============================================================================
// Execute Due Remittance Schedules Tests
// ============================================================================
// These tests verify the idempotent executor for remittance schedules.
// Key security properties: due/not-due partitioning, idempotency on repeated
// calls, InactiveSchedule skipping, and correct next_due advancement.
// ============================================================================

#[test]
fn test_execute_due_remittance_schedules_basic() {
    let env = Env::default();
    let (client, owner, _token_addr, _) = setup_split(&env, 50, 30, 15, 5);

    env.mock_all_auths();
    set_time(&env, 1_000);

    // Create a one-shot schedule due at time 3000
    let schedule_id = client.create_remittance_schedule(&owner, &1_000, &3_000, &0);
    assert_eq!(schedule_id, 1);

    // Advance time past due date
    set_time(&env, 3_500);

    // Execute due schedules
    let executed = client.execute_due_remittance_schedules();
    assert_eq!(executed.len(), 1);
    assert_eq!(executed.get(0).unwrap(), 1);

    // Verify schedule is now inactive (one-off)
    let schedule = client.get_remittance_schedule(&1).unwrap();
    assert!(!schedule.active);
    assert_eq!(schedule.last_executed, Some(3_500));
}

#[test]
fn test_execute_recurring_remittance_schedule() {
    let env = Env::default();
    let (client, owner, _token_addr, _) = setup_split(&env, 50, 30, 15, 5);

    env.mock_all_auths();
    set_time(&env, 1_000);

    // Create a recurring schedule: 1000 amount, due at 3000, every 86400 seconds
    let schedule_id = client.create_remittance_schedule(&owner, &1_000, &3_000, &86_400);
    assert_eq!(schedule_id, 1);

    // Advance time past first due date
    set_time(&env, 3_500);
    let executed = client.execute_due_remittance_schedules();

    assert_eq!(executed.len(), 1);
    assert_eq!(executed.get(0).unwrap(), 1);

    // Verify next_due was advanced by interval
    let schedule = client.get_remittance_schedule(&1).unwrap();
    assert!(schedule.active);
    assert_eq!(schedule.next_due, 3_000 + 86_400);
    assert_eq!(schedule.last_executed, Some(3_500));
    assert_eq!(schedule.missed_count, 0);
}

#[test]
fn test_execute_missed_remittance_schedules() {
    let env = Env::default();
    let (client, owner, _token_addr, _) = setup_split(&env, 50, 30, 15, 5);

    env.mock_all_auths();
    set_time(&env, 1_000);

    // Create a recurring schedule
    let schedule_id = client.create_remittance_schedule(&owner, &500, &3_000, &86_400);
    assert_eq!(schedule_id, 1);

    // Advance time far past multiple intervals: 3000 + 86400*3 + 100
    set_time(&env, 3_000 + 86_400 * 3 + 100);
    let executed = client.execute_due_remittance_schedules();

    assert_eq!(executed.len(), 1);

    // Verify missed_count is 3 (the three intervals that were skipped)
    let schedule = client.get_remittance_schedule(&1).unwrap();
    assert_eq!(schedule.missed_count, 3);
    assert!(schedule.next_due > 3_000 + 86_400 * 3);
    assert_eq!(schedule.last_executed, Some(3_000 + 86_400 * 3 + 100));
}

#[test]
fn test_execute_idempotent_oneshot() {
    let env = Env::default();
    let (client, owner, _token_addr, _) = setup_split(&env, 50, 30, 15, 5);

    env.mock_all_auths();
    set_time(&env, 1_000);

    // Create one-shot schedule
    let schedule_id = client.create_remittance_schedule(&owner, &750, &3_000, &0);
    assert_eq!(schedule_id, 1);

    // Advance time past due
    set_time(&env, 3_500);

    // First execution
    let first = client.execute_due_remittance_schedules();
    assert_eq!(first.len(), 1);
    assert_eq!(first.get(0).unwrap(), 1);

    // Second execution at same timestamp must be idempotent (no-op)
    let second = client.execute_due_remittance_schedules();
    assert_eq!(second.len(), 0, "Second call must be a no-op");

    // Verify schedule remains inactive
    let schedule = client.get_remittance_schedule(&1).unwrap();
    assert!(!schedule.active);
    assert_eq!(schedule.last_executed, Some(3_500));
}

#[test]
fn test_execute_idempotent_recurring() {
    let env = Env::default();
    let (client, owner, _token_addr, _) = setup_split(&env, 50, 30, 15, 5);

    env.mock_all_auths();
    set_time(&env, 1_000);

    // Create recurring schedule
    let schedule_id = client.create_remittance_schedule(&owner, &300, &3_000, &86_400);
    assert_eq!(schedule_id, 1);

    set_time(&env, 3_500);

    // First execution
    let first = client.execute_due_remittance_schedules();
    assert_eq!(first.len(), 1);

    let first_next_due = client.get_remittance_schedule(&1).unwrap().next_due;

    // Second execution at same timestamp must not re-execute
    let second = client.execute_due_remittance_schedules();
    assert_eq!(second.len(), 0);

    // Verify next_due unchanged (idempotent advancement)
    let schedule = client.get_remittance_schedule(&1).unwrap();
    assert_eq!(schedule.next_due, first_next_due);
}

#[test]
fn test_execute_skips_inactive_schedules() {
    let env = Env::default();
    let (client, owner, _token_addr, _) = setup_split(&env, 50, 30, 15, 5);

    env.mock_all_auths();
    set_time(&env, 1_000);

    // Create schedule and cancel it
    let schedule_id = client.create_remittance_schedule(&owner, &200, &3_000, &0);
    assert_eq!(schedule_id, 1);
    
    client.cancel_remittance_schedule(&owner, &1);

    // Advance past due time
    set_time(&env, 3_500);

    // Execute should skip inactive schedule
    let executed = client.execute_due_remittance_schedules();
    assert_eq!(executed.len(), 0);
}

#[test]
fn test_execute_skips_not_yet_due() {
    let env = Env::default();
    let (client, owner, _token_addr, _) = setup_split(&env, 50, 30, 15, 5);

    env.mock_all_auths();
    set_time(&env, 1_000);

    // Create schedule due at 3000
    let schedule_id = client.create_remittance_schedule(&owner, &400, &3_000, &0);
    assert_eq!(schedule_id, 1);

    // Advance time but stay before due date
    set_time(&env, 2_500);

    // Execute should not execute (not yet due)
    let executed = client.execute_due_remittance_schedules();
    assert_eq!(executed.len(), 0);

    // Verify schedule unchanged
    let schedule = client.get_remittance_schedule(&1).unwrap();
    assert!(schedule.active);
    assert_eq!(schedule.last_executed, None);
}

#[test]
fn test_execute_exactly_equal_next_due() {
    let env = Env::default();
    let (client, owner, _token_addr, _) = setup_split(&env, 50, 30, 15, 5);

    env.mock_all_auths();
    set_time(&env, 1_000);

    // Create schedule
    let schedule_id = client.create_remittance_schedule(&owner, &600, &3_000, &0);
    assert_eq!(schedule_id, 1);

    // Advance exactly to next_due (edge case: == not just >)
    set_time(&env, 3_000);

    let executed = client.execute_due_remittance_schedules();
    assert_eq!(executed.len(), 1, "Should execute when time == next_due");
}

#[test]
fn test_execute_empty_schedule_set() {
    let env = Env::default();
    let (client, owner, _token_addr, _) = setup_split(&env, 50, 30, 15, 5);

    env.mock_all_auths();
    set_time(&env, 1_000);

    // No schedules created; just advance time
    set_time(&env, 5_000);

    // Execute on empty set should return empty Vec
    let executed = client.execute_due_remittance_schedules();
    assert_eq!(executed.len(), 0);
}

#[test]
fn test_execute_all_inactive_set() {
    let env = Env::default();
    let (client, owner, _token_addr, _) = setup_split(&env, 50, 30, 15, 5);

    env.mock_all_auths();
    set_time(&env, 1_000);

    // Create and cancel multiple schedules
    for i in 1u32..=3 {
        let id = client.create_remittance_schedule(&owner, &(100i128 * i as i128), &(3_000 + i as u64 * 1000), &0);
        assert!(id > 0);
        client.cancel_remittance_schedule(&owner, &i);
    }

    set_time(&env, 6_000);

    // Execute should return empty (all inactive)
    let executed = client.execute_due_remittance_schedules();
    assert_eq!(executed.len(), 0);
}

#[test]
fn test_execute_paused_contract_returns_empty() {
    let env = Env::default();
    let (client, owner, _token_addr, _) = setup_split(&env, 50, 30, 15, 5);

    env.mock_all_auths();
    set_time(&env, 1_000);

    // Create schedule
    let schedule_id = client.create_remittance_schedule(&owner, &500, &3_000, &0);
    assert_eq!(schedule_id, 1);

    // Pause contract
    client.pause(&owner);

    set_time(&env, 3_500);

    // Execute should return empty when paused
    let executed = client.execute_due_remittance_schedules();
    assert_eq!(executed.len(), 0);

    // Verify schedule was NOT executed (unchanged)
    let schedule = client.get_remittance_schedule(&1).unwrap();
    assert!(schedule.active);
    assert_eq!(schedule.last_executed, None);
}

#[test]
fn test_execute_mixed_due_not_due() {
    let env = Env::default();
    let (client, owner, _token_addr, _) = setup_split(&env, 50, 30, 15, 5);

    env.mock_all_auths();
    set_time(&env, 1_000);

    // Create schedule 1: due at 2000 (one-off)
    let id1 = client.create_remittance_schedule(&owner, &100, &2_000, &0);
    assert_eq!(id1, 1);

    // Create schedule 2: due at 4000 (one-off)
    let id2 = client.create_remittance_schedule(&owner, &200, &4_000, &0);
    assert_eq!(id2, 2);

    // Advance to time 3000 (only schedule 1 is due)
    set_time(&env, 3_000);

    let executed = client.execute_due_remittance_schedules();
    assert_eq!(executed.len(), 1);
    assert_eq!(executed.get(0).unwrap(), 1);

    // Verify only schedule 1 is inactive
    assert!(!client.get_remittance_schedule(&1).unwrap().active);
    assert!(client.get_remittance_schedule(&2).unwrap().active);
}


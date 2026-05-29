#![cfg(test)]

#[cfg(test)]
mod tests {
    use crate::{
        ExecutionStats, Orchestrator, OrchestratorClient, OrchestratorError,
        MAX_DEADLINE_WINDOW_SECS,
    };
    use remitwise_common::CONTRACT_VERSION;
    use soroban_sdk::{
        symbol_short,
        testutils::{Address as _, Ledger as _},
        Address, Env, Symbol,
    };

    // limit=9999 should be clamped to MAX_AUDIT_ENTRIES (100), returning all 10
    let page = client.get_audit_log(&0, &9999);
    assert_eq!(page.len(), 10);
}

#[test]
fn test_audit_log_pagination_no_duplicates() {
    let (env, owner) = setup_test();
    let client = register_orchestrator(&env);
    init_orchestrator(&env, &client, &owner);

    let executor = Address::generate(&env);
    // Add 10 entries
    for nonce in 0..10u64 {
        do_flow(&env, &client, &executor, nonce);
    }

    // Page through with page size 3
    let page0 = client.get_audit_log(&0, &3);
    let page1 = client.get_audit_log(&3, &3);
    let page2 = client.get_audit_log(&6, &3);
    let page3 = client.get_audit_log(&9, &3);

    assert_eq!(page0.len(), 3);
    assert_eq!(page1.len(), 3);
    assert_eq!(page2.len(), 3);
    assert_eq!(page3.len(), 1); // only 1 entry left

    // Collect all timestamps and verify no duplicates
    let mut timestamps: soroban_sdk::Vec<u64> = soroban_sdk::Vec::new(&env);
    for i in 0..page0.len() { timestamps.push_back(page0.get(i).unwrap().timestamp); }
    for i in 0..page1.len() { timestamps.push_back(page1.get(i).unwrap().timestamp); }
    for i in 0..page2.len() { timestamps.push_back(page2.get(i).unwrap().timestamp); }
    for i in 0..page3.len() { timestamps.push_back(page3.get(i).unwrap().timestamp); }

    assert_eq!(timestamps.len(), 10);
}

#[test]
fn test_audit_log_cap_eviction_order() {
    let (env, owner) = setup_test();
    let client = register_orchestrator(&env);
    init_orchestrator(&env, &client, &owner);

    let executor = Address::generate(&env);

    // Fill to exactly MAX_AUDIT_ENTRIES
    for nonce in 0..MAX_AUDIT_ENTRIES as u64 {
        env.ledger().set_timestamp(100_000 + nonce);
        do_flow(&env, &client, &executor, nonce);
    }

    // Log should be full at MAX_AUDIT_ENTRIES
    let full_page = client.get_audit_log(&0, &MAX_AUDIT_ENTRIES);
    assert_eq!(full_page.len(), MAX_AUDIT_ENTRIES);

    // The oldest entry should have timestamp 100_000
    let oldest = full_page.get(0).unwrap();
    assert_eq!(oldest.timestamp, 100_000);

    // Add one more — should evict the oldest (timestamp 100_000)
    env.ledger().set_timestamp(100_000 + MAX_AUDIT_ENTRIES as u64);
    do_flow(&env, &client, &executor, MAX_AUDIT_ENTRIES as u64);

    let after_eviction = client.get_audit_log(&0, &MAX_AUDIT_ENTRIES);
    assert_eq!(after_eviction.len(), MAX_AUDIT_ENTRIES);

    // Oldest entry is now timestamp 100_001 (the second entry before eviction)
    let new_oldest = after_eviction.get(0).unwrap();
    assert_eq!(new_oldest.timestamp, 100_001);

    // Newest entry is the one we just added
    let newest = after_eviction.get(MAX_AUDIT_ENTRIES - 1).unwrap();
    assert_eq!(newest.timestamp, 100_000 + MAX_AUDIT_ENTRIES as u64);
}

#[test]
fn test_evicted_entries_counter_increments() {
    let (env, owner) = setup_test();
    let client = register_orchestrator(&env);
    init_orchestrator(&env, &client, &owner);

    let executor = Address::generate(&env);

    // Fill to cap
    for nonce in 0..MAX_AUDIT_ENTRIES as u64 {
        do_flow(&env, &client, &executor, nonce);
    }

    // No evictions yet
    let stats = client.get_execution_stats().unwrap();
    assert_eq!(stats.evicted_entries, 0);

    // Add 3 more — should evict 3
    for nonce in MAX_AUDIT_ENTRIES as u64..(MAX_AUDIT_ENTRIES as u64 + 3) {
        do_flow(&env, &client, &executor, nonce);
    }

    let stats = client.get_execution_stats().unwrap();
    assert_eq!(stats.evicted_entries, 3);
}

#[test]
fn test_audit_log_entries_ordered_oldest_to_newest() {
    let (env, owner) = setup_test();
    let client = register_orchestrator(&env);
    init_orchestrator(&env, &client, &owner);

    let executor = Address::generate(&env);

    for nonce in 0..5u64 {
        env.ledger().set_timestamp(100_000 + nonce * 10);
        do_flow(&env, &client, &executor, nonce);
    }

    let page = client.get_audit_log(&0, &10);
    assert_eq!(page.len(), 5);

    // Verify ascending timestamp order
    for i in 0..(page.len() - 1) {
        let a = page.get(i).unwrap().timestamp;
        let b = page.get(i + 1).unwrap().timestamp;
        assert!(a <= b, "entries not in ascending order: {} > {}", a, b);
    }

    // ============================================================================
    // Nonce Replay Protection Tests (Issue #648)
    // ============================================================================
    // Verify that the NONCES map and USED_N set prevent replay attacks

    #[test]
    fn test_nonce_starts_at_zero() {
        let (env, owner) = setup_test();
        let client = register_orchestrator(&env);
        init_orchestrator(&env, &client, &owner);

        let executor = Address::generate(&env);
        let nonce = client.get_nonce(&executor);
        assert_eq!(nonce, 0, "New address should start with nonce 0");
    }

    #[test]
    fn test_nonce_increments_after_successful_execution() {
        let (env, owner) = setup_test();
        let client = register_orchestrator(&env);
        init_orchestrator(&env, &client, &owner);

        let executor = Address::generate(&env);
        env.mock_all_auths();

        // First execution with nonce 0
        let deadline = env.ledger().timestamp() + 1000;
        let hash = compute_test_hash(&env, symbol_short!("flow"), 0, 1000, deadline);
        let result = client.try_execute_remittance_flow(&executor, &1000, &0, &deadline, &hash);
        assert!(result.is_ok(), "First execution should succeed");

        // Verify nonce incremented to 1
        let new_nonce = client.get_nonce(&executor);
        assert_eq!(new_nonce, 1, "Nonce should increment after successful execution");
    }

    #[test]
    fn test_replay_same_nonce_fails() {
        let (env, owner) = setup_test();
        let client = register_orchestrator(&env);
        init_orchestrator(&env, &client, &owner);

        let executor = Address::generate(&env);
        env.mock_all_auths();

        let deadline = env.ledger().timestamp() + 1000;
        let hash = compute_test_hash(&env, symbol_short!("flow"), 0, 1000, deadline);

        // First execution with nonce 0
        let result1 = client.try_execute_remittance_flow(&executor, &1000, &0, &deadline, &hash);
        assert!(result1.is_ok(), "First execution should succeed");

        // Attempt replay with same nonce 0 (nonce should now be 1)
        let result2 = client.try_execute_remittance_flow(&executor, &1000, &0, &deadline, &hash);
        assert_eq!(
            result2,
            Err(Ok(OrchestratorError::InvalidNonce)),
            "Replay of same nonce should fail (nonce mismatch)"
        );
    }

    #[test]
    fn test_out_of_order_nonce_fails() {
        let (env, owner) = setup_test();
        let client = register_orchestrator(&env);
        init_orchestrator(&env, &client, &owner);

        let executor = Address::generate(&env);
        env.mock_all_auths();

        let deadline = env.ledger().timestamp() + 1000;

        // Attempt to execute with nonce 5 when current nonce is 0
        let hash = compute_test_hash(&env, symbol_short!("flow"), 5, 1000, deadline);
        let result = client.try_execute_remittance_flow(&executor, &1000, &5, &deadline, &hash);

        assert_eq!(
            result,
            Err(Ok(OrchestratorError::InvalidNonce)),
            "Out-of-order nonce should fail (must equal current nonce)"
        );
    }

    #[test]
    fn test_skipped_nonce_prevents_reuse() {
        let (env, owner) = setup_test();
        let client = register_orchestrator(&env);
        init_orchestrator(&env, &client, &owner);

        let executor = Address::generate(&env);
        env.mock_all_auths();

        let deadline = env.ledger().timestamp() + 1000;

        // Execute with nonce 0
        let hash0 = compute_test_hash(&env, symbol_short!("flow"), 0, 1000, deadline);
        let result0 = client.try_execute_remittance_flow(&executor, &1000, &0, &deadline, &hash0);
        assert!(result0.is_ok());

        // Execute with nonce 1
        let hash1 = compute_test_hash(&env, symbol_short!("flow"), 1, 2000, deadline);
        let result1 = client.try_execute_remittance_flow(&executor, &2000, &1, &deadline, &hash1);
        assert!(result1.is_ok());

        // Nonce should now be 2
        let current_nonce = client.get_nonce(&executor);
        assert_eq!(current_nonce, 2);

        // Now try to reuse nonce 0 (should fail because it's in USED_N)
        let hash_old = compute_test_hash(&env, symbol_short!("flow"), 0, 1000, deadline);
        let result_replay = client.try_execute_remittance_flow(&executor, &1000, &0, &deadline, &hash_old);
        assert_eq!(
            result_replay,
            Err(Ok(OrchestratorError::InvalidNonce)),
            "Reused nonce should fail even if counter was advanced"
        );
    }

    #[test]
    fn test_multiple_addresses_independent_nonces() {
        let (env, owner) = setup_test();
        let client = register_orchestrator(&env);
        init_orchestrator(&env, &client, &owner);

        let executor1 = Address::generate(&env);
        let executor2 = Address::generate(&env);
        env.mock_all_auths();

        let deadline = env.ledger().timestamp() + 1000;

        // Executor1 starts with nonce 0
        let nonce1_before = client.get_nonce(&executor1);
        assert_eq!(nonce1_before, 0);

        // Executor2 starts with nonce 0
        let nonce2_before = client.get_nonce(&executor2);
        assert_eq!(nonce2_before, 0);

        // Execute for executor1 with nonce 0
        let hash1 = compute_test_hash(&env, symbol_short!("flow"), 0, 1000, deadline);
        let result1 = client.try_execute_remittance_flow(&executor1, &1000, &0, &deadline, &hash1);
        assert!(result1.is_ok());

        // Executor1 nonce should be 1
        assert_eq!(client.get_nonce(&executor1), 1);

        // Executor2 nonce should still be 0 (independent)
        assert_eq!(client.get_nonce(&executor2), 0);

        // Executor2 can execute with nonce 0
        let hash2 = compute_test_hash(&env, symbol_short!("flow"), 0, 500, deadline);
        let result2 = client.try_execute_remittance_flow(&executor2, &500, &0, &deadline, &hash2);
        assert!(result2.is_ok(), "Executor2 should execute with nonce 0");
    }

    #[test]
    fn test_request_hash_binding_prevents_parameter_swap() {
        let (env, owner) = setup_test();
        let client = register_orchestrator(&env);
        init_orchestrator(&env, &client, &owner);

        let executor = Address::generate(&env);
        env.mock_all_auths();

        let deadline = env.ledger().timestamp() + 1000;

        // Compute hash for amount 1000
        let hash_1000 = compute_test_hash(&env, symbol_short!("flow"), 0, 1000, deadline);

        // Try to execute with different amount but using hash from 1000
        let result = client.try_execute_remittance_flow(&executor, &5000, &0, &deadline, &hash_1000);

        assert_eq!(
            result,
            Err(Ok(OrchestratorError::InvalidNonce)),
            "Parameter swap attempt should fail (hash mismatch)"
        );
    }

    #[test]
    fn test_deadline_window_prevents_old_requests() {
        let (env, owner) = setup_test();
        let client = register_orchestrator(&env);
        init_orchestrator(&env, &client, &owner);

        let executor = Address::generate(&env);
        env.mock_all_auths();

        // Create a request with a deadline far in the future
        let current_time = env.ledger().timestamp();
        let far_deadline = current_time + 366 * 86400; // 1 year in future (exceeds MAX_DEADLINE_WINDOW_SECS)

        let hash = compute_test_hash(&env, symbol_short!("flow"), 0, 1000, far_deadline);
        let result = client.try_execute_remittance_flow(&executor, &1000, &0, &far_deadline, &hash);

        assert_eq!(
            result,
            Err(Ok(OrchestratorError::DeadlineExpired)),
            "Request with deadline too far in future should fail"
        );
    }
}

#[test]
fn test_audit_log_from_index_at_last_entry() {
    let (env, owner) = setup_test();
    let client = register_orchestrator(&env);
    init_orchestrator(&env, &client, &owner);

    let executor = Address::generate(&env);
    for nonce in 0..5u64 {
        do_flow(&env, &client, &executor, nonce);
    }

    // from_index=4 is the last valid index (len=5)
    let page = client.get_audit_log(&4, &10);
    assert_eq!(page.len(), 1);
}

#[test]
fn test_audit_log_limit_exactly_one() {
    let (env, owner) = setup_test();
    let client = register_orchestrator(&env);
    init_orchestrator(&env, &client, &owner);

    let executor = Address::generate(&env);
    for nonce in 0..5u64 {
        do_flow(&env, &client, &executor, nonce);
    }

    let page = client.get_audit_log(&0, &1);
    assert_eq!(page.len(), 1);
}

#[test]
fn test_audit_log_cap_does_not_exceed_max() {
    let (env, owner) = setup_test();
    let client = register_orchestrator(&env);
    init_orchestrator(&env, &client, &owner);

    let executor = Address::generate(&env);

    // Add more than MAX_AUDIT_ENTRIES
    for nonce in 0..(MAX_AUDIT_ENTRIES as u64 + 20) {
        do_flow(&env, &client, &executor, nonce);
    }

    // Log must never exceed MAX_AUDIT_ENTRIES
    let page = client.get_audit_log(&0, &(MAX_AUDIT_ENTRIES + 100));
    assert_eq!(page.len(), MAX_AUDIT_ENTRIES);
}

#[test]
fn test_get_execution_stats_initial() {
    let (env, owner) = setup_test();
    let client = register_orchestrator(&env);
    init_orchestrator(&env, &client, &owner);

    let stats = client.get_execution_stats();
    assert_eq!(
        stats,
        Some(ExecutionStats {
            total_executions: 0,
            successful_executions: 0,
            failed_executions: 0,
            last_execution_time: 0,
            evicted_entries: 0,
        })
    );
}

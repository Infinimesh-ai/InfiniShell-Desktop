//! Unit tests for the block-list persistence layer in [`super`].
//!
//! Covers session restoration and AI query history persistence behavior.

use std::sync::Arc;

use chrono::{DateTime, Duration, Local};
use diesel::sqlite::SqliteConnection;
use diesel::{Connection, ExpressionMethods, QueryDsl, RunQueryDsl};
use diesel_migrations::MigrationHarness;

use super::{
    get_all_restored_blocks, process_ai_queries_for_nld_history_match,
    process_ai_queries_for_uparrow_prompt, read_recent_ai_queries, save_block,
    upsert_ai_query_with_limit,
};
use crate::ai::agent::conversation::AIConversationId;
use crate::ai::agent::{AIAgentExchangeId, AIAgentInput, UserQueryMode};
use crate::ai::blocklist::{
    AIQueryHistoryOutputStatus, PersistedAIInput, PersistedAIInputType, SerializedBlockListItem,
};
use crate::ai::llms::LLMId;
use crate::persistence::{model, schema};
use crate::terminal::ShellHost;
use crate::terminal::model::block::SerializedBlock;
use crate::terminal::shell::ShellType;

/// Builds an in-memory SQLite database with all migrations applied.
fn test_connection() -> SqliteConnection {
    let mut conn =
        SqliteConnection::establish(":memory:").expect("in-memory sqlite connection should open");
    conn.run_pending_migrations(::persistence::MIGRATIONS)
        .expect("migrations should run");
    conn
}

#[test]
fn restoration_ignores_legacy_visible_bootstrap_blocks() {
    let mut conn = test_connection();
    let pane_uuid = vec![7; 16];
    diesel::insert_into(schema::windows::table)
        .values(model::NewWindow {
            active_tab_index: 0,
            window_width: None,
            window_height: None,
            origin_x: None,
            origin_y: None,
            quake_mode: false,
            universal_search_width: None,
            warp_ai_width: None,
            voltron_width: None,
            warp_drive_index_width: None,
            fullscreen_state: 0,
            agent_management_filters: None,
            left_panel_open: None,
            vertical_tabs_panel_open: None,
            theme_override: None,
            team_uid: None,
        })
        .execute(&mut conn)
        .expect("window should be inserted");
    diesel::insert_into(schema::tabs::table)
        .values(model::NewTab {
            window_id: 1,
            custom_title: None,
            color: None,
            tab_group_id: None,
            pinned: false,
        })
        .execute(&mut conn)
        .expect("tab should be inserted");
    diesel::insert_into(schema::pane_nodes::table)
        .values(model::NewPaneNode {
            tab_id: 1,
            parent_pane_node_id: None,
            flex: None,
            is_leaf: true,
        })
        .execute(&mut conn)
        .expect("pane node should be inserted");
    diesel::insert_into(schema::pane_leaves::table)
        .values((
            schema::pane_leaves::pane_node_id.eq(1),
            schema::pane_leaves::kind.eq(model::TERMINAL_PANE_KIND),
            schema::pane_leaves::is_focused.eq(true),
            schema::pane_leaves::custom_vertical_tabs_title.eq(None::<String>),
        ))
        .execute(&mut conn)
        .expect("pane leaf should be inserted");
    diesel::insert_into(schema::terminal_panes::table)
        .values(model::NewTerminalPane {
            id: 1,
            uuid: pane_uuid.clone(),
            cwd: Some("/tmp".to_owned()),
            is_active: true,
            shell_launch_data: None,
            input_config: None,
            llm_model_override: None,
            active_profile_id: None,
            conversation_ids: None,
            active_conversation_id: None,
        })
        .execute(&mut conn)
        .expect("terminal pane should be inserted");
    let bootstrap = SerializedBlock::new_for_test(b"Welcome to Ubuntu".to_vec(), Vec::new());
    let mut user_command = SerializedBlock::new_for_test(b"echo ready".to_vec(), Vec::new());
    user_command.shell_host = Some(ShellHost {
        shell_type: ShellType::Bash,
        user: "test-user".to_owned(),
        hostname: "test-host".to_owned(),
    });
    save_block(&mut conn, pane_uuid.clone(), &bootstrap, true)
        .expect("bootstrap block should be inserted");
    save_block(&mut conn, pane_uuid, &user_command, true).expect("user block should be inserted");

    let restored = get_all_restored_blocks(&mut conn).expect("blocks should be restored");
    let restored_blocks: Vec<_> = restored.into_values().flatten().collect();

    assert_eq!(restored_blocks.len(), 1);
    let SerializedBlockListItem::Command { block } = &restored_blocks[0];
    assert_eq!(block.stylized_command, b"echo ready");
}

/// Builds a query-bearing [`PersistedAIInput`] with a fresh, unique `exchange_id`.
fn make_query(text: &str) -> Arc<PersistedAIInput> {
    Arc::new(PersistedAIInput {
        exchange_id: AIAgentExchangeId::new(),
        conversation_id: AIConversationId::new(),
        start_ts: Local::now(),
        inputs: vec![PersistedAIInputType::Query {
            text: text.to_string(),
            context: Default::default(),
            referenced_attachments: Default::default(),
        }],
        output_status: AIQueryHistoryOutputStatus::Completed,
        working_directory: None,
        model_id: LLMId::from("test-model"),
        coding_model_id: LLMId::from("test-coding-model"),
    })
}

/// Clones `query` with an explicit `start_ts` so ordering-sensitive tests are deterministic
/// (the NLD reader orders by `start_ts`, which `make_query`'s `Local::now()` cannot guarantee
/// across rapid inserts).
fn with_start_ts(query: Arc<PersistedAIInput>, start_ts: DateTime<Local>) -> Arc<PersistedAIInput> {
    Arc::new(PersistedAIInput {
        start_ts,
        ..(*query).clone()
    })
}

fn ai_query_count(conn: &mut SqliteConnection) -> i64 {
    use crate::persistence::schema::ai_queries::dsl::ai_queries;
    ai_queries
        .count()
        .first(conn)
        .expect("count query should succeed")
}

/// Returns the persisted `exchange_id`s ordered by `id` ascending (i.e. insertion / FIFO order).
fn remaining_exchange_ids(conn: &mut SqliteConnection) -> Vec<String> {
    use crate::persistence::schema::ai_queries::dsl::{ai_queries, exchange_id, id};
    ai_queries
        .select(exchange_id)
        .order(id.asc())
        .load::<String>(conn)
        .expect("load query should succeed")
}

fn input_json_for_exchange(conn: &mut SqliteConnection, exchange: &str) -> String {
    use crate::persistence::schema::ai_queries::dsl::{ai_queries, exchange_id, input};
    ai_queries
        .filter(exchange_id.eq(exchange))
        .select(input)
        .first::<String>(conn)
        .expect("row for exchange should exist")
}

/// Returns the text of the first query input on a [`PersistedAIInput`].
fn first_query_text(query: &PersistedAIInput) -> &str {
    match query.inputs.first().expect("query should have an input") {
        PersistedAIInputType::Query { text, .. } => text,
    }
}

#[test]
fn upsert_ai_query_caps_table_and_evicts_oldest_first() {
    let mut conn = test_connection();
    let limit = 3;

    // Insert five distinct exchanges into a table capped at three.
    let queries: Vec<Arc<PersistedAIInput>> =
        (0..5).map(|i| make_query(&format!("q{i}"))).collect();
    let exchange_ids: Vec<String> = queries.iter().map(|q| q.exchange_id.to_string()).collect();

    for query in &queries {
        upsert_ai_query_with_limit(&mut conn, query.clone(), limit).expect("upsert should succeed");
    }

    // The table never exceeds the limit.
    assert_eq!(ai_query_count(&mut conn), limit);

    // The two oldest (q0, q1) are evicted; the three newest remain in insertion order.
    assert_eq!(
        remaining_exchange_ids(&mut conn),
        exchange_ids[2..].to_vec()
    );
}

#[test]
fn upsert_ai_query_stays_below_limit_without_evicting() {
    let mut conn = test_connection();
    let limit = 3;

    // Filling exactly up to the limit should not evict anything.
    let queries: Vec<Arc<PersistedAIInput>> =
        (0..3).map(|i| make_query(&format!("q{i}"))).collect();
    let exchange_ids: Vec<String> = queries.iter().map(|q| q.exchange_id.to_string()).collect();

    for query in &queries {
        upsert_ai_query_with_limit(&mut conn, query.clone(), limit).expect("upsert should succeed");
    }

    assert_eq!(ai_query_count(&mut conn), limit);
    assert_eq!(remaining_exchange_ids(&mut conn), exchange_ids);
}

#[test]
fn upsert_ai_query_updates_existing_exchange_without_evicting() {
    let mut conn = test_connection();
    let limit = 2;

    // Fill the table to its limit with two distinct exchanges.
    let first = make_query("first");
    let second = make_query("second");
    upsert_ai_query_with_limit(&mut conn, first.clone(), limit).expect("upsert should succeed");
    upsert_ai_query_with_limit(&mut conn, second.clone(), limit).expect("upsert should succeed");
    assert_eq!(ai_query_count(&mut conn), limit);

    // Re-upsert the oldest exchange (same `exchange_id`) repeatedly. Because this is an update of
    // an existing exchange rather than a new one, it must update in place and never evict.
    let updated_first = Arc::new(PersistedAIInput {
        inputs: vec![PersistedAIInputType::Query {
            text: "first-updated".to_string(),
            context: Default::default(),
            referenced_attachments: Default::default(),
        }],
        ..(*first).clone()
    });
    for _ in 0..5 {
        upsert_ai_query_with_limit(&mut conn, updated_first.clone(), limit)
            .expect("upsert should succeed");
    }

    // Still exactly two rows, and both original exchanges survive (the oldest was not evicted).
    assert_eq!(ai_query_count(&mut conn), limit);
    assert_eq!(
        remaining_exchange_ids(&mut conn),
        vec![
            first.exchange_id.to_string(),
            second.exchange_id.to_string()
        ]
    );

    // The in-place update took effect.
    let input_json = input_json_for_exchange(&mut conn, &first.exchange_id.to_string());
    assert!(
        input_json.contains("first-updated"),
        "existing row should have been updated in place, got: {input_json}"
    );
}

/// Builds a [`PersistedAIInput`] whose inputs serialize to `[]`, mirroring legacy rows
/// written before empty inputs were skipped at write time.
fn make_empty_input_query() -> Arc<PersistedAIInput> {
    Arc::new(PersistedAIInput {
        inputs: vec![],
        ..(*make_query("unused")).clone()
    })
}

#[test]
fn process_ai_queries_for_nld_history_match_filters_empty_and_whitespace_inputs_oldest_first() {
    let mut conn = test_connection();

    // Explicit, strictly increasing timestamps keep the `start_ts`-ordered read deterministic.
    let t0 = Local::now();
    for query in [
        with_start_ts(make_query("older prompt"), t0),
        with_start_ts(make_query("   "), t0 + Duration::seconds(1)),
        with_start_ts(make_empty_input_query(), t0 + Duration::seconds(2)),
        with_start_ts(make_query("newer prompt"), t0 + Duration::seconds(3)),
    ] {
        upsert_ai_query_with_limit(&mut conn, query, 10).expect("upsert should succeed");
    }

    let recent_ai_queries = read_recent_ai_queries(&mut conn).expect("read should succeed");
    let prompts = process_ai_queries_for_nld_history_match(&recent_ai_queries);
    let texts: Vec<&str> = prompts.iter().map(|(text, _)| text.as_str()).collect();
    // `[]` and whitespace-only rows are dropped; the rest come back oldest-first.
    assert_eq!(texts, vec!["older prompt", "newer prompt"]);
}

#[test]
fn process_ai_queries_for_uparrow_prompt_keeps_newest_capped_oldest_first() {
    // Build 150 oldest-first queries; only the newest 100 should survive, order preserved.
    let queries: Vec<PersistedAIInput> = (0..150)
        .map(|i| (*make_query(&format!("q{i}"))).clone())
        .collect();

    let kept = process_ai_queries_for_uparrow_prompt(queries);

    assert_eq!(kept.len(), 100);
    // The newest 100 (q50..=q149) survive, still oldest-first.
    assert_eq!(first_query_text(&kept[0]), "q50");
    assert_eq!(first_query_text(&kept[99]), "q149");
}

#[test]
fn process_ai_queries_for_uparrow_prompt_keeps_all_when_under_cap() {
    // Fewer than the cap: everything is kept, order preserved.
    let queries: Vec<PersistedAIInput> = (0..3)
        .map(|i| (*make_query(&format!("q{i}"))).clone())
        .collect();

    let kept = process_ai_queries_for_uparrow_prompt(queries);

    let texts: Vec<&str> = kept.iter().map(first_query_text).collect();
    assert_eq!(texts, vec!["q0", "q1", "q2"]);
}

#[test]
fn empty_input_skip_filters_out_non_query_inputs() {
    // Mirrors the filter in `handle_ai_history_event`: only query-bearing inputs are persisted.
    // An exchange whose inputs are all non-query types collapses to an empty `inputs` vec, which
    // is the exact condition that skips persistence.
    let user_query = AIAgentInput::UserQuery {
        query: "hello".to_string(),
        context: Default::default(),
        static_query_type: None,
        referenced_attachments: Default::default(),
        user_query_mode: UserQueryMode::default(),
        running_command: None,
        intended_agent: None,
    };
    let non_query = AIAgentInput::ResumeConversation {
        context: Default::default(),
    };

    // A query input is persistable; a non-query input is not.
    assert!(PersistedAIInputType::try_from(&user_query).is_ok());
    assert!(PersistedAIInputType::try_from(&non_query).is_err());

    // An exchange carrying only non-query inputs collapses to empty -> skipped.
    let only_non_query = [non_query];
    let persisted: Vec<_> = only_non_query
        .iter()
        .filter_map(|input| PersistedAIInputType::try_from(input).ok())
        .collect();
    assert!(persisted.is_empty());

    // An exchange carrying a query input is persisted.
    let with_query = [user_query];
    let persisted: Vec<_> = with_query
        .iter()
        .filter_map(|input| PersistedAIInputType::try_from(input).ok())
        .collect();
    assert_eq!(persisted.len(), 1);
}

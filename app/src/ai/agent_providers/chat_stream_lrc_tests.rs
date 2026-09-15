use super::*;
use crate::ai::agent::task::helper::{MessageExt, ToolCallExt};

async fn lrc_actions(target_task_id: &str, should_spawn: bool) -> Vec<api::client_action::Action> {
    let mut server = mockito::Server::new_async().await;
    let mock = server.mock("POST", "/v1/chat/completions")
        .with_header("content-type", "text/event-stream")
        .with_body(concat!(
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"running\"},\"finish_reason\":null}]}\n\n",
            "data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
            "data: [DONE]\n\n"
        )).create_async().await;
    let params = RequestParams::new_for_test(
        vec![AIAgentInput::UserQuery {
            query: "检查轮询状态".to_owned(),
            context: Default::default(),
            static_query_type: None,
            referenced_attachments: Default::default(),
            user_query_mode: Default::default(),
            running_command: None,
            intended_agent: None,
        }],
        vec![],
    );
    let (_cancel, cancellation_rx) = futures::channel::oneshot::channel();
    let stream = generate_byop_output(ByopOutputInput {
        params,
        base_url: format!("{}/v1", server.url()),
        api_key: "test-key".to_owned(),
        model_id: "test-model".to_owned(),
        api_type: AgentProviderApiType::OpenAi,
        reasoning_effort: crate::settings::ReasoningEffortSetting::Auto,
        extra_headers: vec![],
        responses: Default::default(),
        task_id: "root-task".to_owned(),
        target_task_id: target_task_id.to_owned(),
        needs_create_task: false,
        lrc_command_id: Some("poll-command".to_owned()),
        lrc_should_spawn_subagent: should_spawn,
        context_window: None,
        cancellation_rx,
        attachment_caps: Default::default(),
    })
    .await
    .unwrap();
    let events: Vec<_> = stream.map(|event| event.unwrap()).collect().await;
    mock.assert_async().await;
    events
        .into_iter()
        .filter_map(|event| {
            if let Some(api::response_event::Type::ClientActions(actions)) = event.r#type {
                Some(actions.actions)
            } else {
                None
            }
        })
        .flatten()
        .filter_map(|action| action.action)
        .collect()
}

#[tokio::test]
async fn lrc_initialization_reuses_optimistic_target_before_adding_its_messages() {
    let actions = lrc_actions("optimistic-cli", true).await;
    let created_task = actions
        .iter()
        .find_map(|action| {
            if let api::client_action::Action::CreateTask(create) = action {
                create.task.as_ref()
            } else {
                None
            }
        })
        .unwrap();
    assert_eq!(created_task.id, "optimistic-cli");
    assert_eq!(
        created_task.dependencies.as_ref().unwrap().parent_task_id,
        "root-task"
    );

    let parent_call = actions
        .iter()
        .filter_map(|action| {
            if let api::client_action::Action::AddMessagesToTask(add) = action {
                Some(add)
            } else {
                None
            }
        })
        .flat_map(|add| &add.messages)
        .find_map(|message| message.tool_call()?.subagent())
        .unwrap();
    assert_eq!(parent_call.task_id, "optimistic-cli");

    let create_position = actions
        .iter()
        .position(|action| matches!(action, api::client_action::Action::CreateTask(_)))
        .unwrap();
    let messages_position = actions.iter().position(|action| matches!(action,
        api::client_action::Action::AddMessagesToTask(add) if add.task_id == "optimistic-cli"
    )).unwrap();
    assert!(create_position < messages_position);
    assert!(actions.iter().any(|action| matches!(action,
        api::client_action::Action::AddMessagesToTask(add)
            if add.task_id == "optimistic-cli" && add.messages.iter().any(|message| matches!(message.message, Some(api::message::Message::AgentOutput(_))))
    )));
}

#[tokio::test]
async fn initialized_cli_follow_up_writes_existing_task_without_create_task() {
    let actions = lrc_actions("initialized-cli", false).await;
    assert!(
        !actions
            .iter()
            .any(|action| matches!(action, api::client_action::Action::CreateTask(_)))
    );
    let message_task_ids: Vec<_> = actions
        .iter()
        .filter_map(|action| {
            if let api::client_action::Action::AddMessagesToTask(add) = action {
                Some(add.task_id.as_str())
            } else {
                None
            }
        })
        .collect();
    assert!(!message_task_ids.is_empty());
    assert!(
        message_task_ids
            .iter()
            .all(|task_id| *task_id == "initialized-cli")
    );
}

#[tokio::test]
async fn tag_in_without_optimistic_target_creates_child_of_root() {
    let actions = lrc_actions("root-task", true).await;
    let created_task = actions
        .iter()
        .find_map(|action| {
            if let api::client_action::Action::CreateTask(create) = action {
                create.task.as_ref()
            } else {
                None
            }
        })
        .unwrap();
    assert_ne!(created_task.id, "root-task");
    assert_eq!(
        created_task.dependencies.as_ref().unwrap().parent_task_id,
        "root-task"
    );
    let parent_call = actions
        .iter()
        .filter_map(|action| {
            if let api::client_action::Action::AddMessagesToTask(add) = action {
                Some(add)
            } else {
                None
            }
        })
        .flat_map(|add| &add.messages)
        .find_map(|message| message.tool_call()?.subagent())
        .unwrap();
    assert_eq!(parent_call.task_id, created_task.id);
}

use std::collections::{BTreeMap, HashSet};

use super::{
    models_dev_enriched_models, models_dev_synced_models, refreshed_agent_provider_models,
};
use crate::ai::agent_providers::models_dev::{
    Catalog, Model, ModelLimit, ModelModalities, Provider,
};
use crate::settings::{AgentProvider, AgentProviderModel};

#[test]
fn api_refresh_replaces_stale_models_and_preserves_matching_metadata() {
    let mut retained = AgentProviderModel::from_id("model-b".to_string());
    retained.name = "自定义别名".to_string();
    retained.context_window = 128_000;
    retained.max_output_tokens = 16_000;
    retained.reasoning = true;
    retained.image = Some(false);

    let existing = vec![
        AgentProviderModel::from_id("stale-model".to_string()),
        retained.clone(),
    ];

    let refreshed =
        refreshed_agent_provider_models(&existing, ["model-a".to_string(), "model-b".to_string()]);

    assert_eq!(
        refreshed,
        vec![AgentProviderModel::from_id("model-a".to_string()), retained]
    );
}

#[test]
fn api_refresh_deduplicates_ids_and_can_clear_the_list() {
    let existing = vec![AgentProviderModel::from_id("old-model".to_string())];

    let refreshed = refreshed_agent_provider_models(
        &existing,
        ["new-model".to_string(), "new-model".to_string()],
    );
    assert_eq!(
        refreshed,
        vec![AgentProviderModel::from_id("new-model".to_string())]
    );

    assert!(refreshed_agent_provider_models(&existing, Vec::<String>::new()).is_empty());
}

#[test]
fn models_dev_sync_enriches_existing_models_without_adding_catalog_entries() {
    let mut local_model = AgentProviderModel::from_id("model-b".to_string());
    local_model.name = "用户别名".to_string();
    local_model.image = Some(false);

    let mut provider = AgentProvider::new_empty();
    provider.name = "自定义聚合网关".to_string();
    provider.base_url = "https://gateway.example.com/v1".to_string();
    provider.models = vec![
        local_model,
        AgentProviderModel::from_id("local-only".to_string()),
    ];

    let catalog_model = Model {
        id: "model-b".to_string(),
        name: "Catalog name".to_string(),
        reasoning: true,
        tool_call: false,
        attachment: true,
        modalities: ModelModalities {
            input: vec!["image".to_string()],
            output: vec!["text".to_string()],
        },
        limit: ModelLimit {
            context: 128_000,
            output: 32_000,
        },
        ..Default::default()
    };
    let catalog: Catalog = BTreeMap::from([(
        "catalog-provider".to_string(),
        Provider {
            models: BTreeMap::from([("model-b".to_string(), catalog_model)]),
            ..Default::default()
        },
    )]);

    let (models, summary) = models_dev_synced_models(&provider, &catalog);

    assert_eq!(summary.matched, 1);
    assert_eq!(summary.changed, 1);
    assert_eq!(models.len(), 2);
    assert_eq!(models[0].name, "用户别名");
    assert_eq!(models[0].context_window, 128_000);
    assert_eq!(models[0].max_output_tokens, 32_000);
    assert!(models[0].reasoning);
    assert!(!models[0].tool_call);
    assert_eq!(models[0].image, Some(false));
    assert_eq!(models[0].pdf, Some(true));
    assert_eq!(models[1].id, "local-only");
}

#[test]
fn models_dev_auto_enrichment_only_updates_requested_models() {
    let mut provider = AgentProvider::new_empty();
    provider.models = vec![
        AgentProviderModel::from_id("new-model".to_string()),
        AgentProviderModel::from_id("existing-model".to_string()),
    ];

    let catalog: Catalog = BTreeMap::from([(
        "catalog-provider".to_string(),
        Provider {
            models: BTreeMap::from([
                (
                    "new-model".to_string(),
                    Model {
                        id: "new-model".to_string(),
                        name: "New model".to_string(),
                        reasoning: true,
                        limit: ModelLimit {
                            context: 200_000,
                            output: 20_000,
                        },
                        ..Default::default()
                    },
                ),
                (
                    "existing-model".to_string(),
                    Model {
                        id: "existing-model".to_string(),
                        name: "Existing model".to_string(),
                        reasoning: true,
                        limit: ModelLimit {
                            context: 100_000,
                            output: 10_000,
                        },
                        ..Default::default()
                    },
                ),
            ]),
            ..Default::default()
        },
    )]);

    let targets = HashSet::from(["new-model".to_string()]);
    let (models, summary) = models_dev_enriched_models(&provider, &catalog, &targets);

    assert_eq!(summary.matched, 1);
    assert_eq!(summary.changed, 1);
    assert_eq!(models[0].name, "New model");
    assert_eq!(models[0].context_window, 200_000);
    assert!(models[0].reasoning);
    assert_eq!(
        models[1],
        AgentProviderModel::from_id("existing-model".to_string())
    );
}

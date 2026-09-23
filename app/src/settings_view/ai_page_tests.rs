use std::collections::{BTreeMap, HashSet};

use super::{
    AgentProviderModelDraft, apply_agent_provider_model_draft, models_dev_enriched_models,
    models_dev_synced_models, refreshed_agent_provider_models,
};
use crate::ai::agent_providers::models_dev::{
    Catalog, Model, ModelLimit, ModelModalities, Provider,
};
use crate::settings::{AgentProvider, AgentProviderModel};

#[test]
fn api_refresh_removes_only_stale_api_models_and_preserves_matching_metadata() {
    let mut retained = AgentProviderModel::from_id("model-b".to_string());
    retained.name = "自定义别名".to_string();
    retained.context_window = 128_000;
    retained.max_output_tokens = 16_000;
    retained.reasoning = Some(true);
    retained.image = Some(false);

    let mut stale_api_model = AgentProviderModel::from_id("stale-api-model".to_string());
    stale_api_model.api_discovered = true;
    let manual_model = AgentProviderModel::from_id("manual-model".to_string());
    let existing = vec![stale_api_model, manual_model.clone(), retained.clone()];

    let refreshed =
        refreshed_agent_provider_models(&existing, ["model-a".to_string(), "model-b".to_string()]);

    let mut model_a = AgentProviderModel::from_id("model-a".to_string());
    model_a.api_discovered = true;
    assert_eq!(refreshed, vec![model_a, retained, manual_model]);
}

#[test]
fn api_refresh_deduplicates_ids_and_preserves_every_model_on_empty_response() {
    let manual = AgentProviderModel::from_id("manual-model".to_string());
    let mut old_api = AgentProviderModel::from_id("old-api-model".to_string());
    old_api.api_discovered = true;
    let existing = vec![manual.clone(), old_api];

    let refreshed = refreshed_agent_provider_models(
        &existing,
        ["new-model".to_string(), "new-model".to_string()],
    );
    let mut new_api = AgentProviderModel::from_id("new-model".to_string());
    new_api.api_discovered = true;
    assert_eq!(refreshed, vec![new_api, manual.clone()]);

    assert_eq!(
        refreshed_agent_provider_models(&existing, Vec::<String>::new()),
        existing
    );
    assert_eq!(
        refreshed_agent_provider_models(&existing, [" ".to_string()]),
        existing
    );
}

#[test]
fn api_refresh_never_reclassifies_a_legacy_model_as_api_managed() {
    let legacy = AgentProviderModel::from_id("legacy-model".to_string());
    let unreported = AgentProviderModel::from_id("unreported-model".to_string());

    let first = refreshed_agent_provider_models(
        &[legacy.clone(), unreported.clone()],
        ["legacy-model".to_string()],
    );
    assert!(!first[0].api_discovered);
    assert_eq!(first[1], unreported);

    let second = refreshed_agent_provider_models(&first, Vec::<String>::new());
    assert_eq!(second, vec![legacy, unreported]);
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

    let (models, summary) = models_dev_synced_models(&provider, &catalog, 123);

    assert_eq!(summary.matched, 1);
    assert_eq!(summary.changed, 1);
    assert_eq!(models.len(), 2);
    assert_eq!(models[0].name, "用户别名");
    assert_eq!(models[0].context_window, 0);
    assert_eq!(models[0].max_output_tokens, 0);
    assert_eq!(models[0].effective_context_window(), 128_000);
    assert_eq!(models[0].effective_max_output_tokens(), 32_000);
    assert!(models[0].effective_reasoning());
    assert!(!models[0].effective_tool_call());
    assert_eq!(models[0].image, Some(false));
    assert_eq!(models[0].pdf, None);
    assert!(models[0].catalog_metadata.as_ref().unwrap().pdf);
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
    let (models, summary) = models_dev_enriched_models(&provider, &catalog, &targets, 456);

    assert_eq!(summary.matched, 1);
    assert_eq!(summary.changed, 1);
    assert_eq!(models[0].name, "");
    assert_eq!(models[0].effective_name(), "New model");
    assert_eq!(models[0].effective_context_window(), 200_000);
    assert!(models[0].effective_reasoning());
    assert_eq!(
        models[1],
        AgentProviderModel::from_id("existing-model".to_string())
    );
}

#[test]
fn models_dev_sync_preserves_every_manual_override() {
    let mut model = AgentProviderModel::from_id("model-a".to_string());
    model.name = "My alias".to_string();
    model.context_window = 64_000;
    model.max_output_tokens = 4_000;
    model.reasoning = Some(false);
    model.tool_call = Some(true);
    model.image = Some(false);
    model.pdf = Some(false);
    model.audio = Some(true);

    let mut provider = AgentProvider::new_empty();
    provider.models = vec![model];
    let catalog = BTreeMap::from([(
        "catalog-provider".to_string(),
        Provider {
            models: BTreeMap::from([(
                "model-a".to_string(),
                Model {
                    id: "model-a".to_string(),
                    name: "Catalog name".to_string(),
                    reasoning: true,
                    tool_call: false,
                    attachment: true,
                    modalities: ModelModalities {
                        input: vec!["image".to_string()],
                        output: vec!["text".to_string()],
                    },
                    limit: ModelLimit {
                        context: 200_000,
                        output: 32_000,
                    },
                    ..Default::default()
                },
            )]),
            ..Default::default()
        },
    )]);

    let (models, summary) = models_dev_synced_models(&provider, &catalog, 789);

    assert_eq!(summary.preserved_overrides, 8);
    assert_eq!(models[0].effective_name(), "My alias");
    assert_eq!(models[0].effective_context_window(), 64_000);
    assert_eq!(models[0].effective_max_output_tokens(), 4_000);
    assert!(!models[0].effective_reasoning());
    assert!(models[0].effective_tool_call());
    assert_eq!(models[0].image, Some(false));
    assert_eq!(models[0].pdf, Some(false));
    assert_eq!(models[0].audio, Some(true));
}

#[test]
fn duplicate_global_model_id_requires_an_explicit_mapping() {
    let mut provider = AgentProvider::new_empty();
    provider.models = vec![AgentProviderModel::from_id("shared-model".to_string())];
    let catalog: Catalog = BTreeMap::from([
        (
            "provider-a".to_string(),
            Provider {
                models: BTreeMap::from([(
                    "shared-model".to_string(),
                    Model {
                        id: "shared-model".to_string(),
                        name: "A".to_string(),
                        ..Default::default()
                    },
                )]),
                ..Default::default()
            },
        ),
        (
            "provider-b".to_string(),
            Provider {
                models: BTreeMap::from([(
                    "shared-model".to_string(),
                    Model {
                        id: "shared-model".to_string(),
                        name: "B".to_string(),
                        ..Default::default()
                    },
                )]),
                ..Default::default()
            },
        ),
    ]);

    let (models, summary) = models_dev_synced_models(&provider, &catalog, 100);
    assert_eq!(summary.matched, 0);
    assert!(models[0].catalog_metadata.is_none());

    provider.models[0].models_dev_provider_id = Some("provider-b".to_string());
    let (models, summary) = models_dev_synced_models(&provider, &catalog, 101);
    assert_eq!(summary.matched, 1);
    assert_eq!(models[0].effective_name(), "B");
    assert_eq!(
        models[0]
            .catalog_metadata
            .as_ref()
            .unwrap()
            .match_confidence,
        crate::settings::AgentProviderModelCatalogMatch::Explicit
    );
}

#[test]
fn manual_sync_marks_an_unmatched_snapshot_without_dropping_its_capabilities() {
    let mut model = AgentProviderModel::from_id("removed-model".to_string());
    model.catalog_metadata = Some(crate::settings::AgentProviderModelCatalogMetadata {
        name: "Removed".to_string(),
        context_window: 100_000,
        max_output_tokens: 8_000,
        reasoning: false,
        tool_call: true,
        image: false,
        pdf: false,
        audio: false,
        provider_id: "old-provider".to_string(),
        model_id: "removed-model".to_string(),
        match_confidence: crate::settings::AgentProviderModelCatalogMatch::UniqueModelId,
        updated_at_unix_seconds: 1,
        unmatched_in_latest_catalog: false,
    });
    let mut provider = AgentProvider::new_empty();
    provider.models = vec![model];

    let (models, summary) = models_dev_synced_models(&provider, &Catalog::new(), 200);

    assert_eq!(summary.retained_unmatched, 1);
    assert_eq!(models[0].effective_context_window(), 100_000);
    assert!(
        models[0]
            .catalog_metadata
            .as_ref()
            .unwrap()
            .unmatched_in_latest_catalog
    );

    provider.models = models;
    let (models, summary) = models_dev_synced_models(&provider, &Catalog::new(), 201);
    assert_eq!(summary.changed, 0);
    assert_eq!(summary.retained_unmatched, 1);
    assert_eq!(models[0].effective_context_window(), 100_000);
}

#[test]
fn manual_sync_clears_the_unmatched_marker_after_catalog_match_recovers() {
    let mut model = AgentProviderModel::from_id("model-a".to_string());
    model.catalog_metadata = Some(crate::settings::AgentProviderModelCatalogMetadata {
        name: "Old name".to_string(),
        context_window: 100_000,
        max_output_tokens: 8_000,
        reasoning: false,
        tool_call: true,
        image: false,
        pdf: false,
        audio: false,
        provider_id: "catalog-provider".to_string(),
        model_id: "model-a".to_string(),
        match_confidence: crate::settings::AgentProviderModelCatalogMatch::UniqueModelId,
        updated_at_unix_seconds: 1,
        unmatched_in_latest_catalog: true,
    });
    let mut provider = AgentProvider::new_empty();
    provider.models = vec![model];
    let catalog = BTreeMap::from([(
        "catalog-provider".to_string(),
        Provider {
            models: BTreeMap::from([(
                "model-a".to_string(),
                Model {
                    id: "model-a".to_string(),
                    name: "New name".to_string(),
                    limit: ModelLimit {
                        context: 200_000,
                        output: 16_000,
                    },
                    ..Default::default()
                },
            )]),
            ..Default::default()
        },
    )]);

    let (models, summary) = models_dev_synced_models(&provider, &catalog, 200);

    assert_eq!(summary.matched, 1);
    assert!(
        !models[0]
            .catalog_metadata
            .as_ref()
            .unwrap()
            .unmatched_in_latest_catalog
    );
    assert_eq!(models[0].effective_context_window(), 200_000);
}

#[test]
fn changing_model_id_does_not_restore_old_overrides_on_repeated_save() {
    let mut model = AgentProviderModel::from_id("old-model".to_string());
    model.name = "Old alias".to_string();
    model.context_window = 64_000;
    model.max_output_tokens = 4_096;
    model.models_dev_provider_id = Some("old-provider".to_string());
    let mut draft = AgentProviderModelDraft {
        index: 0,
        name: "Old alias".to_string(),
        id: "new-model".to_string(),
        context_window: 64_000,
        max_output_tokens: 4_096,
        models_dev_provider_id: "old-provider".to_string(),
        models_dev_model_id: String::new(),
        editors: None,
    };

    assert_eq!(
        apply_agent_provider_model_draft(&mut model, &draft),
        (true, false)
    );
    draft.name.clear();
    draft.context_window = 0;
    draft.max_output_tokens = 0;
    draft.models_dev_provider_id.clear();
    assert_eq!(
        apply_agent_provider_model_draft(&mut model, &draft),
        (false, false)
    );
    assert_eq!(model.id, "new-model");
    assert_eq!(model.name, "");
    assert_eq!(model.context_window, 0);
    assert_eq!(model.max_output_tokens, 0);
    assert_eq!(model.models_dev_provider_id, None);
}

#[test]
fn alias_reentered_after_id_change_is_saved_but_not_carried_to_another_id() {
    let mut model = AgentProviderModel::from_id("old-model".to_string());
    model.name = "Same alias".to_string();
    let mut draft = AgentProviderModelDraft {
        index: 0,
        name: "Same alias".to_string(),
        id: "new-model".to_string(),
        context_window: 0,
        max_output_tokens: 0,
        models_dev_provider_id: String::new(),
        models_dev_model_id: String::new(),
        editors: None,
    };

    assert_eq!(
        apply_agent_provider_model_draft(&mut model, &draft),
        (true, false)
    );
    assert_eq!(model.name, "");
    draft.name.clear();
    assert_eq!(
        apply_agent_provider_model_draft(&mut model, &draft),
        (false, false)
    );
    draft.name = "Same alias".to_string();
    assert_eq!(
        apply_agent_provider_model_draft(&mut model, &draft),
        (false, false)
    );
    assert_eq!(model.name, "Same alias");
    draft.id = "third-model".to_string();
    assert_eq!(
        apply_agent_provider_model_draft(&mut model, &draft),
        (true, false)
    );
    assert_eq!(model.name, "");
}

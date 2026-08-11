use std::sync::LazyLock;

use regex::Regex;
use uuid::Uuid;

use crate::terminal::view::PromptSuggestion;

pub struct StaticPromptSuggestion {
    pub name: &'static str,
    pub pattern: &'static str,
    pub label_template: Option<StaticPromptSuggestionTemplate>,
    pub query_template: StaticPromptSuggestionTemplate,
}

#[derive(Clone, Copy)]
pub enum StaticPromptSuggestionTemplate {
    FeatureOrBugLabel,
    HelpFeatureOrBugLabel,
    ImplementFeatureOrBugQuery,
    CreatePullRequestQuery,
    StartNewProjectLabel,
    StartNewProjectQuery,
    NodeProjectLabel,
    NodeProjectQuery,
    ReactAppLabel,
    ReactAppQuery,
    NextAppLabel,
    NextAppQuery,
    RustProjectLabel,
    RustProjectQuery,
    PoetryProjectLabel,
    PoetryProjectQuery,
    DjangoProjectLabel,
    DjangoProjectQuery,
    RailsAppLabel,
    RailsAppQuery,
    GradleMavenProjectLabel,
    GradleMavenProjectQuery,
    GoProjectLabel,
    GoProjectQuery,
    SwiftProjectLabel,
    SwiftProjectQuery,
    TerraformConfigLabel,
    TerraformConfigQuery,
    PrismaSetupLabel,
    PrismaSetupQuery,
    InstallDependenciesQuery,
    RubyProjectLabel,
    RubyProjectQuery,
    ModelfileQuery,
    KubernetesUtilizationQuery,
    KubernetesInspectQuery,
    DockerContainersQuery,
    DockerImagesQuery,
    DockerComposeLabel,
    DockerComposeQuery,
    DockerNetworkQuery,
    VagrantBoxQuery,
    VagrantUpQuery,
    GrepSearchQuery,
    FindSearchQuery,
    SshKeygenQuery,
}

impl StaticPromptSuggestionTemplate {
    fn localized(self) -> String {
        match self {
            StaticPromptSuggestionTemplate::FeatureOrBugLabel => {
                crate::t!("passive-suggestion-feature-or-bug-label")
            }
            StaticPromptSuggestionTemplate::HelpFeatureOrBugLabel => {
                crate::t!("passive-suggestion-help-feature-or-bug-label")
            }
            StaticPromptSuggestionTemplate::ImplementFeatureOrBugQuery => {
                crate::t!("passive-suggestion-implement-feature-or-bug-query")
            }
            StaticPromptSuggestionTemplate::CreatePullRequestQuery => {
                crate::t!("passive-suggestion-create-pull-request-query")
            }
            StaticPromptSuggestionTemplate::StartNewProjectLabel => {
                crate::t!("passive-suggestion-start-new-project-label")
            }
            StaticPromptSuggestionTemplate::StartNewProjectQuery => {
                crate::t!("passive-suggestion-start-new-project-query")
            }
            StaticPromptSuggestionTemplate::NodeProjectLabel => {
                crate::t!("passive-suggestion-node-project-label")
            }
            StaticPromptSuggestionTemplate::NodeProjectQuery => {
                crate::t!("passive-suggestion-node-project-query")
            }
            StaticPromptSuggestionTemplate::ReactAppLabel => {
                crate::t!("passive-suggestion-react-app-label")
            }
            StaticPromptSuggestionTemplate::ReactAppQuery => {
                crate::t!("passive-suggestion-react-app-query")
            }
            StaticPromptSuggestionTemplate::NextAppLabel => {
                crate::t!("passive-suggestion-next-app-label")
            }
            StaticPromptSuggestionTemplate::NextAppQuery => {
                crate::t!("passive-suggestion-next-app-query")
            }
            StaticPromptSuggestionTemplate::RustProjectLabel => {
                crate::t!("passive-suggestion-rust-project-label")
            }
            StaticPromptSuggestionTemplate::RustProjectQuery => {
                crate::t!("passive-suggestion-rust-project-query")
            }
            StaticPromptSuggestionTemplate::PoetryProjectLabel => {
                crate::t!("passive-suggestion-poetry-project-label")
            }
            StaticPromptSuggestionTemplate::PoetryProjectQuery => {
                crate::t!("passive-suggestion-poetry-project-query")
            }
            StaticPromptSuggestionTemplate::DjangoProjectLabel => {
                crate::t!("passive-suggestion-django-project-label")
            }
            StaticPromptSuggestionTemplate::DjangoProjectQuery => {
                crate::t!("passive-suggestion-django-project-query")
            }
            StaticPromptSuggestionTemplate::RailsAppLabel => {
                crate::t!("passive-suggestion-rails-app-label")
            }
            StaticPromptSuggestionTemplate::RailsAppQuery => {
                crate::t!("passive-suggestion-rails-app-query")
            }
            StaticPromptSuggestionTemplate::GradleMavenProjectLabel => {
                crate::t!("passive-suggestion-gradle-maven-project-label")
            }
            StaticPromptSuggestionTemplate::GradleMavenProjectQuery => {
                crate::t!("passive-suggestion-gradle-maven-project-query")
            }
            StaticPromptSuggestionTemplate::GoProjectLabel => {
                crate::t!("passive-suggestion-go-project-label")
            }
            StaticPromptSuggestionTemplate::GoProjectQuery => {
                crate::t!("passive-suggestion-go-project-query")
            }
            StaticPromptSuggestionTemplate::SwiftProjectLabel => {
                crate::t!("passive-suggestion-swift-project-label")
            }
            StaticPromptSuggestionTemplate::SwiftProjectQuery => {
                crate::t!("passive-suggestion-swift-project-query")
            }
            StaticPromptSuggestionTemplate::TerraformConfigLabel => {
                crate::t!("passive-suggestion-terraform-config-label")
            }
            StaticPromptSuggestionTemplate::TerraformConfigQuery => {
                crate::t!("passive-suggestion-terraform-config-query")
            }
            StaticPromptSuggestionTemplate::PrismaSetupLabel => {
                crate::t!("passive-suggestion-prisma-setup-label")
            }
            StaticPromptSuggestionTemplate::PrismaSetupQuery => {
                crate::t!("passive-suggestion-prisma-setup-query")
            }
            StaticPromptSuggestionTemplate::InstallDependenciesQuery => {
                crate::t!("passive-suggestion-install-dependencies-query")
            }
            StaticPromptSuggestionTemplate::RubyProjectLabel => {
                crate::t!("passive-suggestion-ruby-project-label")
            }
            StaticPromptSuggestionTemplate::RubyProjectQuery => {
                crate::t!("passive-suggestion-ruby-project-query")
            }
            StaticPromptSuggestionTemplate::ModelfileQuery => {
                crate::t!("passive-suggestion-modelfile-query")
            }
            StaticPromptSuggestionTemplate::KubernetesUtilizationQuery => {
                crate::t!("passive-suggestion-kubernetes-utilization-query")
            }
            StaticPromptSuggestionTemplate::KubernetesInspectQuery => {
                crate::t!("passive-suggestion-kubernetes-inspect-query")
            }
            StaticPromptSuggestionTemplate::DockerContainersQuery => {
                crate::t!("passive-suggestion-docker-containers-query")
            }
            StaticPromptSuggestionTemplate::DockerImagesQuery => {
                crate::t!("passive-suggestion-docker-images-query")
            }
            StaticPromptSuggestionTemplate::DockerComposeLabel => {
                crate::t!("passive-suggestion-docker-compose-label")
            }
            StaticPromptSuggestionTemplate::DockerComposeQuery => {
                crate::t!("passive-suggestion-docker-compose-query")
            }
            StaticPromptSuggestionTemplate::DockerNetworkQuery => {
                crate::t!("passive-suggestion-docker-network-query")
            }
            StaticPromptSuggestionTemplate::VagrantBoxQuery => {
                crate::t!("passive-suggestion-vagrant-box-query")
            }
            StaticPromptSuggestionTemplate::VagrantUpQuery => {
                crate::t!("passive-suggestion-vagrant-up-query")
            }
            StaticPromptSuggestionTemplate::GrepSearchQuery => {
                crate::t!("passive-suggestion-grep-search-query")
            }
            StaticPromptSuggestionTemplate::FindSearchQuery => {
                crate::t!("passive-suggestion-find-search-query")
            }
            StaticPromptSuggestionTemplate::SshKeygenQuery => {
                crate::t!("passive-suggestion-ssh-keygen-query")
            }
        }
    }
}

/// Attempts to match a terminal command against predefined static prompt suggestions.
///
/// If the command matches a static rule, this returns a [`SuggestedQuery`] with details from the
/// command substituted into the rule's query template.
pub fn static_suggested_query(command: &str) -> Option<PromptSuggestion> {
    // Try each rule in turn and apply the first match.
    for pattern in &*RULE_PATTERNS {
        if let Some(captures) = pattern.regex.captures(command) {
            // If there's a match, apply placeholders to the query.
            let label = pattern
                .rule
                .label_template
                .map(|template| apply_captures(&template.localized(), &captures));
            let query = apply_captures(&pattern.rule.query_template.localized(), &captures);

            return Some(PromptSuggestion {
                id: Uuid::new_v4().to_string(),
                label,
                prompt: query,
                coding_query_context: None,
                static_prompt_suggestion_name: Some(pattern.rule.name.to_string()),
                should_start_new_conversation: false,
            });
        }
    }

    None
}

/// A static prompt suggestion with its pattern precompiled to a [`Regex`].
struct StaticPromptRule {
    rule: &'static StaticPromptSuggestion,
    regex: Regex,
}

static RULE_PATTERNS: LazyLock<Vec<StaticPromptRule>> = LazyLock::new(|| {
    STATIC_RULES
        .iter()
        .map(|rule| match Regex::new(rule.pattern) {
            Ok(regex) => StaticPromptRule { rule, regex },
            Err(e) => {
                panic!(
                    "Invalid pattern for static prompt rule `{}`: {}",
                    rule.name, e
                );
            }
        })
        .collect()
});

static STATIC_RULES: &[StaticPromptSuggestion] = &[
    // git checkout -b <branch>: Checks out a new branch named <branch>.
    StaticPromptSuggestion {
        name: "GIT_CHECKOUT_NEW_BRANCH",
        pattern: r"^git\s+checkout\s+-b\s+(\S+)\s*$",
        label_template: Some(StaticPromptSuggestionTemplate::FeatureOrBugLabel),
        query_template: StaticPromptSuggestionTemplate::ImplementFeatureOrBugQuery,
    },
    // git clone <repo>: Clones a repository named <repo>.
    StaticPromptSuggestion {
        name: "GIT_CLONE",
        pattern: r"^git\s+clone\s+(\S+)\s*$",
        label_template: Some(StaticPromptSuggestionTemplate::HelpFeatureOrBugLabel),
        query_template: StaticPromptSuggestionTemplate::ImplementFeatureOrBugQuery,
    },
    // git switch -c <branch>: Creates and switches to a new branch named <branch>.
    StaticPromptSuggestion {
        name: "GIT_SWITCH_NEW_BRANCH",
        pattern: r"^git\s+switch\s+-c\s+(\S+)\s*$",
        label_template: Some(StaticPromptSuggestionTemplate::FeatureOrBugLabel),
        query_template: StaticPromptSuggestionTemplate::ImplementFeatureOrBugQuery,
    },
    // git push: Pushes changes to a remote repository.
    StaticPromptSuggestion {
        name: "GIT_PUSH",
        pattern: r"^git\s+push\s*$",
        label_template: None,
        query_template: StaticPromptSuggestionTemplate::CreatePullRequestQuery,
    },
    // git init: Initializes a new, empty Git repository.
    StaticPromptSuggestion {
        name: "GIT_INIT",
        pattern: r"^git\s+init\s*$",
        label_template: Some(StaticPromptSuggestionTemplate::StartNewProjectLabel),
        query_template: StaticPromptSuggestionTemplate::StartNewProjectQuery,
    },
    // npm init / yarn init / pnpm init: Initializes a Node.js project.
    StaticPromptSuggestion {
        name: "NODE_PACKAGE_INIT",
        pattern: r"^(npm|yarn|pnpm)\s+init\s*$",
        label_template: Some(StaticPromptSuggestionTemplate::NodeProjectLabel),
        query_template: StaticPromptSuggestionTemplate::NodeProjectQuery,
    },
    // npx create-react-app <project>: Creates a new React app called <project>.
    StaticPromptSuggestion {
        name: "NPX_CREATE_REACT_APP",
        pattern: r"^npx\s+create-react-app\s+(\S+)\s*$",
        label_template: Some(StaticPromptSuggestionTemplate::ReactAppLabel),
        query_template: StaticPromptSuggestionTemplate::ReactAppQuery,
    },
    // npx create-next-app <project>: Creates a new Next.js app called <project>.
    StaticPromptSuggestion {
        name: "NPX_CREATE_NEXT_APP",
        pattern: r"^npx\s+create-next-app\s+(\S+)\s*$",
        label_template: Some(StaticPromptSuggestionTemplate::NextAppLabel),
        query_template: StaticPromptSuggestionTemplate::NextAppQuery,
    },
    // cargo new <project>: Creates a new Rust package named <project>.
    StaticPromptSuggestion {
        name: "CARGO_NEW_PROJECT",
        pattern: r"^cargo\s+new\s+(\S+)\s*$",
        label_template: Some(StaticPromptSuggestionTemplate::RustProjectLabel),
        query_template: StaticPromptSuggestionTemplate::RustProjectQuery,
    },
    // poetry new <project>: Creates a new Poetry-based Python project named <project>.
    StaticPromptSuggestion {
        name: "POETRY_NEW_PROJECT",
        pattern: r"^poetry\s+new\s+(\S+)\s*$",
        label_template: Some(StaticPromptSuggestionTemplate::PoetryProjectLabel),
        query_template: StaticPromptSuggestionTemplate::PoetryProjectQuery,
    },
    // django-admin startproject <project>: Creates a new Django project named <project>.
    StaticPromptSuggestion {
        name: "DJANGO_START_PROJECT",
        pattern: r"^django-admin\s+startproject\s+(\S+)\s*$",
        label_template: Some(StaticPromptSuggestionTemplate::DjangoProjectLabel),
        query_template: StaticPromptSuggestionTemplate::DjangoProjectQuery,
    },
    // rails new <app>: Creates a new Rails app named <app>.
    StaticPromptSuggestion {
        name: "RAILS_NEW_APP",
        pattern: r"^rails\s+new\s+(\S+)\s*$",
        label_template: Some(StaticPromptSuggestionTemplate::RailsAppLabel),
        query_template: StaticPromptSuggestionTemplate::RailsAppQuery,
    },
    // gradle init / mvn archetype:generate: Initializes a Gradle or Maven project.
    StaticPromptSuggestion {
        name: "JAVA_PROJECT_INIT",
        pattern: r"^(gradle\s+init|mvn\s+archetype:generate)\s*$",
        label_template: Some(StaticPromptSuggestionTemplate::GradleMavenProjectLabel),
        query_template: StaticPromptSuggestionTemplate::GradleMavenProjectQuery,
    },
    // go mod init <module>: Initializes a new Go module named <module>.
    StaticPromptSuggestion {
        name: "GO_MOD_INIT",
        pattern: r"^go\s+mod\s+init\s+(\S+)\s*$",
        label_template: Some(StaticPromptSuggestionTemplate::GoProjectLabel),
        query_template: StaticPromptSuggestionTemplate::GoProjectQuery,
    },
    // swift package init: Initializes a new Swift package.
    StaticPromptSuggestion {
        name: "SWIFT_PACKAGE_INIT",
        pattern: r"^swift\s+package\s+init\s*$",
        label_template: Some(StaticPromptSuggestionTemplate::SwiftProjectLabel),
        query_template: StaticPromptSuggestionTemplate::SwiftProjectQuery,
    },
    // terraform init: Initializes Terraform in the current directory.
    StaticPromptSuggestion {
        name: "TERRAFORM_INIT",
        pattern: r"^terraform\s+init\s*$",
        label_template: Some(StaticPromptSuggestionTemplate::TerraformConfigLabel),
        query_template: StaticPromptSuggestionTemplate::TerraformConfigQuery,
    },
    // prisma init: Initializes Prisma in the current project.
    StaticPromptSuggestion {
        name: "PRISMA_INIT",
        pattern: r"^prisma\s+init\s*$",
        label_template: Some(StaticPromptSuggestionTemplate::PrismaSetupLabel),
        query_template: StaticPromptSuggestionTemplate::PrismaSetupQuery,
    },
    // python -m venv <env_name>: Creates a new Python virtual environment named <env_name>.
    StaticPromptSuggestion {
        name: "PYTHON_CREATE_VENV",
        pattern: r"^python\s+-m\s+venv\s+(\S+)\s*$",
        label_template: None,
        query_template: StaticPromptSuggestionTemplate::InstallDependenciesQuery,
    },
    // bundle init: Creates a new Gemfile (Ruby Bundler).
    StaticPromptSuggestion {
        name: "BUNDLE_INIT",
        pattern: r"^bundle\s+init\s*$",
        label_template: Some(StaticPromptSuggestionTemplate::RubyProjectLabel),
        query_template: StaticPromptSuggestionTemplate::RubyProjectQuery,
    },
    // ollama pull <model>: Pulls an Ollama model named <model>.
    StaticPromptSuggestion {
        name: "OLLAMA_PULL_MODEL",
        pattern: r"^ollama\s+pull\s+(\S+)\s*$",
        label_template: None,
        query_template: StaticPromptSuggestionTemplate::ModelfileQuery,
    },
    // kubectl top nodes: Shows node resource usage in Kubernetes.
    StaticPromptSuggestion {
        name: "KUBECTL_TOP_NODES",
        pattern: r"^kubectl\s+top\s+(nodes|node|no)\s*$",
        label_template: None,
        query_template: StaticPromptSuggestionTemplate::KubernetesUtilizationQuery,
    },
    // kubectl top pods: Shows pod resource usage in Kubernetes.
    StaticPromptSuggestion {
        name: "KUBECTL_TOP_PODS",
        pattern: r"^kubectl\s+top\s+(pods|po|pod)\s*$",
        label_template: None,
        query_template: StaticPromptSuggestionTemplate::KubernetesUtilizationQuery,
    },
    // kubectl get...: Gets Kubernetes resources (any).
    StaticPromptSuggestion {
        name: "KUBECTL_GET_RESOURCES",
        pattern: r"^kubectl\s+get.*$",
        label_template: None,
        query_template: StaticPromptSuggestionTemplate::KubernetesInspectQuery,
    },
    // docker ps: Lists Docker containers.
    StaticPromptSuggestion {
        name: "DOCKER_LIST_CONTAINERS",
        pattern: r"^docker\s+ps\s*$",
        label_template: None,
        query_template: StaticPromptSuggestionTemplate::DockerContainersQuery,
    },
    // docker image ls: Lists Docker images.
    StaticPromptSuggestion {
        name: "DOCKER_LIST_IMAGES",
        pattern: r"^docker\s+image\s+ls\s*$",
        label_template: None,
        query_template: StaticPromptSuggestionTemplate::DockerImagesQuery,
    },
    // docker-compose up -d <service>: Spins up a service <service> in Docker Compose.
    StaticPromptSuggestion {
        name: "DOCKER_COMPOSE_UP_SERVICE",
        pattern: r"^docker-compose\s+up\s+-d\s+(\S+)\s*$",
        label_template: Some(StaticPromptSuggestionTemplate::DockerComposeLabel),
        query_template: StaticPromptSuggestionTemplate::DockerComposeQuery,
    },
    // docker network create <network>: Creates a Docker network named <network>.
    StaticPromptSuggestion {
        name: "DOCKER_NETWORK_CREATE",
        pattern: r"^docker\s+network\s+create\s+(\S+)\s*$",
        label_template: None,
        query_template: StaticPromptSuggestionTemplate::DockerNetworkQuery,
    },
    // vagrant init <box>: Initializes a Vagrant box named <box>.
    StaticPromptSuggestion {
        name: "VAGRANT_INIT_BOX",
        pattern: r"^vagrant\s+init\s+(\S+)\s*$",
        label_template: None,
        query_template: StaticPromptSuggestionTemplate::VagrantBoxQuery,
    },
    // vagrant up: Brings up a Vagrant environment.
    StaticPromptSuggestion {
        name: "VAGRANT_UP",
        pattern: r"^vagrant\s+up\s*$",
        label_template: None,
        query_template: StaticPromptSuggestionTemplate::VagrantUpQuery,
    },
    // grep -r <pattern>: Searches recursively for <pattern> in files.
    StaticPromptSuggestion {
        // Capture everything after `grep -r ` into capture group 1.
        name: "GREP_RECURSIVE_SEARCH",
        pattern: r"^grep\s+-r\s+(.*)$",
        label_template: None,
        query_template: StaticPromptSuggestionTemplate::GrepSearchQuery,
    },
    // find <args>: Searches for files/directories using `find`.
    StaticPromptSuggestion {
        // Capture everything after `find ` into capture group 1.
        // E.g. `find . -name "*.rs"`.
        name: "FIND_FILES",
        pattern: r"^find\s+(.*)$",
        label_template: None,
        query_template: StaticPromptSuggestionTemplate::FindSearchQuery,
    },
    // ssh-keygen (no args): Generates an SSH key with default options.
    StaticPromptSuggestion {
        // This pattern matches "ssh-keygen" by itself or anything after it (e.g. "-t rsa -b 4096").
        name: "SSH_KEYGEN",
        pattern: r"^ssh-keygen(?:\s+(.*))?$",
        // We’ll keep the label/query generic so it applies whether or not the user passed extra flags.
        // Not using the capture group here, but it's there if we need it for the future.
        label_template: None,
        query_template: StaticPromptSuggestionTemplate::SshKeygenQuery,
    },
];

pub fn apply_captures(template: &str, captures: &regex::Captures) -> String {
    // We'll look for placeholders of the form `{1}`, `{2}`, etc. and replace them with the
    // corresponding capture group.
    let mut result = String::from(template);

    for i in 1..captures.len() {
        let placeholder = format!("{{{i}}}");
        if let Some(m) = captures.get(i) {
            result = result.replace(&placeholder, m.as_str());
        }
    }
    result
}

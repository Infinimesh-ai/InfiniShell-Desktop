use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::Result;
use futures::future::BoxFuture;
use warp_util::host_id::HostId;
use warp_util::local_or_remote_path::LocalOrRemotePath;
use warpui_core::{AppContext, Entity, ModelContext, SingletonEntity};

use super::GlobalRules;

/// 默认规则文件列表。顺序 = 优先级(靠前者优先);同目录多个文件
/// 同时存在时 `RuleAtPath::respected_rule()` 只取优先级最高的一个。
///
/// - WARP.md  项目原生约定。
/// - AGENTS.md 社区通用(opencode / Cursor / Cline 等都识别)。
/// - CLAUDE.md Claude Code 原生约定，让从 Claude Code 迁过来的项目一键可用。
///
/// 扩展新名称时,本数组的插入位置 = 优先级,同时要在 `RuleAtPath` 加一个
/// 对应槽位,并在 `RuleAtPath::slot_for_file_name()` / `respected_rule()`
/// 中登记该槽位(增量更新与优先级两条路径都靠这两处)。
///
/// 定义在 `cfg_if` 外部，以便不编译 `local_fs` 的路径(WASM / 测试)也能引用。
pub(crate) const RULES_FILE_PATTERN: &[&str] = &["WARP.md", "AGENTS.md", "CLAUDE.md"];

cfg_if::cfg_if! {
    if #[cfg(feature = "local_fs")] {
        use repo_metadata::{
            RepoMetadataEvent, RepoMetadataModel, RepositoryIdentifier, StandingQueryContent,
        };
        use warp_util::remote_path::RemotePath;
        use warp_util::standardized_path::StandardizedPath;
        // `instant::Instant` 是本仓库全局约定的跨平台(含 WASM)起点,代替
        // `std::time::Instant`。使用 `clippy.toml` 中的 disallowed_types 强制。
        use instant::Instant;
        use std::time::{Duration, SystemTime};

        // —— Fast-path(对齐 opencode `findUp` 模式)——
        //
        // 主用途:cd 进入新 git 仓库后,异步 `index_and_store_rules` 完成前的
        // 时间窗口内,`pending_context` 同步调用此 fast-path 直接 stat + 读 cwd
        // 及其祖先目录的规则文件,保证 AGENTS.md / WARP.md / CLAUDE.md
        // **不会因为异步竞争漏注入**。
        // 正常路径(`find_applicable_rules`)一旦可用,fast-path 让位并清缓存。
        //
        // UI 不卡顿保障:
        //   - 单次最坏 `MAX_WALK_DEPTH * RULES_FILE_PATTERN.len()` 次 metadata
        //     + 命中文件 `read_to_string`(规则文件一般几 KB,Windows NTFS < 1ms/文件)。
        //   - `FAST_PATH_BUDGET` 时间预算硬截断,超时立即返回已收集部分,绝不阻塞。
        //   - 稳态命中(目录无变化)只做 stat,不重读文件;mtime / size / parent-dir-mtime
        //     任一变化即重扫。
        const MAX_WALK_DEPTH: usize = 6;
        const FAST_PATH_BUDGET: Duration = Duration::from_millis(20);
    }
}

/// Fast-path 缓存条目。`stamps` 记录已命中文件的 (path, mtime, size),
/// `walked_dir_stamps` 记录遍历过的目录的 (path, mtime),用于检测
/// "目录里新增 / 删除 / 修改了规则文件"两类失效。`negative` 缓存表示
/// 上次扫描没找到任何规则,后续相同 stamps 直接复用,不再 IO。
#[cfg(feature = "local_fs")]
#[derive(Clone, Debug)]
struct FastPathEntry {
    rules: Vec<ProjectRule>,
    /// fast-path 用的 "root" — 取**首层命中**的目录;全 miss 时取 cwd 本身。
    /// 用于构造 `ProjectRulesResult.root_path`,语义对齐 `find_applicable_rules`。
    root_path: PathBuf,
    stamps: Vec<(PathBuf, SystemTime, u64)>,
    walked_dir_stamps: Vec<(PathBuf, SystemTime)>,
}

pub type ProjectRuleContents = Vec<(LocalOrRemotePath, String)>;
/// App-provided transport for reading the exact rule paths discovered by repository metadata.
///
/// This remains injected because remote file reads are implemented in the app crate.
pub type ProjectRuleContentReader = fn(
    Vec<LocalOrRemotePath>,
    &AppContext,
) -> BoxFuture<'static, anyhow::Result<ProjectRuleContents>>;

#[cfg(feature = "local_fs")]
fn standing_project_rule_paths<'a>(
    repo_id: &RepositoryIdentifier,
    contents: impl IntoIterator<Item = &'a StandingQueryContent>,
) -> Vec<LocalOrRemotePath> {
    contents
        .into_iter()
        .filter(|content| !content.is_directory)
        .filter_map(|content| match repo_id {
            RepositoryIdentifier::Local(_) => {
                content.path.to_local_path().map(LocalOrRemotePath::Local)
            }
            RepositoryIdentifier::Remote(remote_root) => Some(LocalOrRemotePath::Remote(
                RemotePath::new(remote_root.host_id.clone(), content.path.clone()),
            )),
        })
        .collect()
}

#[derive(Debug, Clone)]
pub struct ProjectRule {
    pub path: LocalOrRemotePath,
    pub content: String,
}

#[derive(Debug, Clone)]
struct RuleAtPath {
    parent_path: LocalOrRemotePath,
    warp_md: Option<ProjectRule>,
    agents_md: Option<ProjectRule>,
    claude_md: Option<ProjectRule>,
}

impl RuleAtPath {
    fn respected_rule(&self) -> Option<&ProjectRule> {
        self.warp_md
            .as_ref()
            .or(self.agents_md.as_ref())
            .or(self.claude_md.as_ref())
    }

    /// 按文件名(大小写不敏感)定位对应的规则槽位,文件名不在
    /// `RULES_FILE_PATTERN` 中时返回 None。`upsert_rule` / `remove_rule`
    /// 共用此处,避免文件名分支在多处各写一遍而漏掉某个名称。
    fn slot_for_file_name(&mut self, file_name: &str) -> Option<&mut Option<ProjectRule>> {
        let file_name = file_name.to_lowercase();
        if file_name == "warp.md" {
            Some(&mut self.warp_md)
        } else if file_name == "agents.md" {
            Some(&mut self.agents_md)
        } else if file_name == "claude.md" {
            Some(&mut self.claude_md)
        } else {
            None
        }
    }
}

#[derive(Debug, Clone)]
pub struct ProjectRulesResult {
    pub root_path: LocalOrRemotePath,
    pub active_rules: Vec<ProjectRule>,
    pub additional_rule_paths: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectRulePath {
    pub path: PathBuf,
    pub project_root: PathBuf,
}

struct FindRulesResult {
    /// Rules that are active and should be eagerly applied.
    active_rules: Vec<ProjectRule>,
    /// Rule paths that are currently not active but available to be applied if
    /// a file under its directory is edited.
    available_rule_paths: Vec<String>,
}

#[derive(Debug, Default, Clone)]
struct ProjectRules {
    rules: Vec<RuleAtPath>,
}

impl ProjectRules {
    #[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
    fn rule_paths(&self) -> impl Iterator<Item = &LocalOrRemotePath> {
        self.rules.iter().flat_map(|rule| {
            // 覆盖全部槽位(优先级在读取时才应用),漏掉 claude_md 会让
            // CLAUDE.md 在持久化/保留集合计算中丢失。
            rule.warp_md
                .iter()
                .chain(rule.agents_md.iter())
                .chain(rule.claude_md.iter())
                .map(|rule| &rule.path)
        })
    }
    #[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
    fn local_rule_paths(&self) -> impl Iterator<Item = PathBuf> + '_ {
        self.rule_paths()
            .filter_map(|path| path.to_local_path().map(Path::to_path_buf))
    }
    #[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
    fn retain_rule_paths(&mut self, retained_paths: &HashSet<LocalOrRemotePath>) {
        self.rules.retain_mut(|rule| {
            if rule
                .warp_md
                .as_ref()
                .is_some_and(|rule| !retained_paths.contains(&rule.path))
            {
                rule.warp_md = None;
            }
            if rule
                .agents_md
                .as_ref()
                .is_some_and(|rule| !retained_paths.contains(&rule.path))
            {
                rule.agents_md = None;
            }
            if rule
                .claude_md
                .as_ref()
                .is_some_and(|rule| !retained_paths.contains(&rule.path))
            {
                rule.claude_md = None;
            }
            rule.warp_md.is_some() || rule.agents_md.is_some() || rule.claude_md.is_some()
        });
    }
    /// Finds the set of rules that are active in the given path and the set that are available to be applied.
    fn find_active_or_applicable_rules(&self, path: &LocalOrRemotePath) -> FindRulesResult {
        let mut active_rules = Vec::new();
        let mut available_rule_paths = Vec::new();

        // Collect all applicable rules (rules in directories that are ancestors of the target path)
        for rule in &self.rules {
            if let Some(respected_rule) = rule.respected_rule() {
                // Check if the rule's directory is an ancestor of or equal to the target path
                if path.starts_with(&rule.parent_path) {
                    active_rules.push(respected_rule.clone());
                } else {
                    available_rule_paths.push(respected_rule.path.display_path());
                }
            }
        }

        FindRulesResult {
            active_rules,
            available_rule_paths,
        }
    }

    /// Remove a rule from the set of project rules. This returns the removed rule.
    #[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
    fn remove_rule(&mut self, path: &LocalOrRemotePath) -> Option<ProjectRule> {
        let parent = path.parent()?;
        let file_name = path.file_name()?;

        let rule = self
            .rules
            .iter_mut()
            .find(|rule| rule.parent_path == parent)?;

        rule.slot_for_file_name(file_name)?.take()
    }

    /// Upsert a rule to the set of project rules. This will create a new RuleAtPath entry if none exists and update the existing one
    /// otherwise.
    #[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
    fn upsert_rule(&mut self, path: &LocalOrRemotePath, content: String) {
        let Some(parent) = path.parent() else {
            return;
        };
        let Some(file_name) = path.file_name() else {
            return;
        };

        let existing_rule = self
            .rules
            .iter_mut()
            .find(|rule| rule.parent_path == parent);

        let rule_file = Some(ProjectRule {
            path: path.clone(),
            content,
        });

        match existing_rule {
            Some(rule) => {
                if let Some(slot) = rule.slot_for_file_name(file_name) {
                    *slot = rule_file;
                }
            }
            None => {
                let mut rule = RuleAtPath {
                    parent_path: parent,
                    warp_md: None,
                    agents_md: None,
                    claude_md: None,
                };
                if let Some(slot) = rule.slot_for_file_name(file_name) {
                    *slot = rule_file;
                }
                self.rules.push(rule);
            }
        };
    }
}

/// Singleton model that keeps track of mapping between paths and rule files
/// Currently supports WARP.md files, but designed to be extensible
#[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
#[derive(Default)]
pub struct ProjectContextModel {
    /// Mapping from directory path to list of rule files found in that directory
    path_to_rules: HashMap<LocalOrRemotePath, ProjectRules>,
    /// Fast-path 同步规则缓存(对齐 opencode `findUp` 模式)。
    ///
    /// 仅在 `find_applicable_rules` 返回 None(异步索引未就绪 / 不在已索引根下)时
    /// 兜底使用,避免 cd 后立即发 AI 请求时漏注入 AGENTS.md / WARP.md。
    /// 单线程访问(WarpUI Singleton 在 main thread),用 `RefCell` 而非锁,
    /// 满足 `pending_context(&self, app: &AppContext)` 这种 `&self` 调用形态。
    #[cfg(feature = "local_fs")]
    fast_path_cache: RefCell<HashMap<PathBuf, FastPathEntry>>,
    /// Latest metadata-backed async refresh per exact repository identity.
    /// This uses the same identifier carried by metadata events rather than an arbitrary file path.
    #[cfg(feature = "local_fs")]
    rule_refresh_generations: HashMap<RepositoryIdentifier, u64>,
    #[cfg(feature = "local_fs")]
    next_rule_refresh_generation: u64,
    /// File-based global rules and their local watcher state. Kept separate
    /// from `path_to_rules`, which is project-scoped.
    pub(super) global_rules: GlobalRules,
    /// File-based global rules published by connected remote hosts. Kept
    /// separate from local globals so existing local Rules UI accessors remain
    /// local-only.
    remote_global_rules: HashMap<HostId, Vec<ProjectRule>>,
}

#[derive(Default, Debug)]
pub struct RulesDelta {
    pub discovered_rules: Vec<ProjectRulePath>,
    pub deleted_rules: Vec<PathBuf>,
}

impl RulesDelta {
    /// Merge another delta into this one, preserving the ordering of operations.
    ///
    /// When the same path appears across sequential deltas the *last* operation
    /// wins. For example:
    ///   - (add A, delete A) → net effect is **delete**
    ///   - (delete A, add A) → net effect is **add**
    ///
    /// This is important because consumers (e.g. persistence) apply the delta
    /// incrementally; a symmetric "cancel both sides" approach would silently
    /// drop real state changes.
    #[cfg(test)]
    fn merge(&mut self, other: RulesDelta) {
        // Each newly-discovered path supersedes any prior deletion or earlier
        // discovery of the same path.
        for discovered in &other.discovered_rules {
            self.deleted_rules.retain(|p| *p != discovered.path);
            self.discovered_rules.retain(|r| r.path != discovered.path);
        }
        // Each newly-deleted path supersedes any prior discovery or earlier
        // deletion of the same path.
        for deleted in &other.deleted_rules {
            self.discovered_rules.retain(|r| r.path != *deleted);
            self.deleted_rules.retain(|p| *p != *deleted);
        }
        self.discovered_rules.extend(other.discovered_rules);
        self.deleted_rules.extend(other.deleted_rules);
    }
}

#[derive(Default, Debug)]
pub struct GlobalRulesDelta {
    pub discovered_rules: Vec<PathBuf>,
    pub deleted_rules: Vec<PathBuf>,
}

/// Events emitted by the ProjectContextModel
pub enum ProjectContextModelEvent {
    /// Emitted when a path has been indexed
    PathIndexed,
    /// Emitted when the known set of rule files changed
    KnownRulesChanged(RulesDelta),
    /// Emitted when the set of indexed global rule files changed
    GlobalRulesChanged(GlobalRulesDelta),
}

impl ProjectContextModel {
    #[cfg_attr(not(feature = "local_fs"), allow(unused_variables))]
    pub fn new_from_persisted(
        persisted_rules: Vec<ProjectRulePath>,
        project_rule_content_reader: ProjectRuleContentReader,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        #[cfg_attr(not(feature = "local_fs"), allow(unused_mut))]
        let mut model = Self::default();
        #[cfg(feature = "local_fs")]
        {
            ctx.subscribe_to_model(&RepoMetadataModel::handle(ctx), move |me, _, event, ctx| {
                match event {
                    RepoMetadataEvent::RepositoryUpdated { id } => {
                        me.refresh_project_rules_for_repo(
                            id.clone(),
                            project_rule_content_reader,
                            ctx,
                        );
                    }
                    RepoMetadataEvent::StandingQueryResultsUpdated { id, delta } => {
                        if delta.project_rules_changed() {
                            me.refresh_project_rules_for_repo(
                                id.clone(),
                                project_rule_content_reader,
                                ctx,
                            );
                        }
                    }
                    RepoMetadataEvent::RepositoryRemoved { id } => {
                        me.remove_project_rules_for_repo(id, ctx);
                    }
                    RepoMetadataEvent::FileTreeUpdated { .. }
                    | RepoMetadataEvent::FileTreeEntryUpdated { .. }
                    | RepoMetadataEvent::UpdatingRepositoryFailed { .. }
                    | RepoMetadataEvent::IncrementalUpdateReady { .. } => {}
                }
            });

            ctx.spawn(
                async move { Self::read_persisted_rules(persisted_rules).await },
                |me, mut res, ctx| {
                    // Metadata refreshes may have completed before persistence loads; retain
                    // the fresher metadata-backed state for overlapping roots.
                    res.extend(me.path_to_rules.drain());
                    me.path_to_rules = res;
                    ctx.emit(ProjectContextModelEvent::PathIndexed);
                },
            );

            // Remote snapshots may have arrived before this model subscribed to metadata events,
            // so hydrate any remote repositories that are already tracked.
            let remote_repo_ids = RepoMetadataModel::as_ref(ctx)
                .remote_repository_ids(ctx)
                .cloned()
                .map(RepositoryIdentifier::Remote)
                .collect::<Vec<_>>();
            for repo_id in remote_repo_ids {
                model.refresh_project_rules_for_repo(repo_id, project_rule_content_reader, ctx);
            }
        }

        model
    }

    /// Reconciles project rule contents from the repository metadata standing result set.
    #[cfg_attr(not(feature = "local_fs"), allow(unused_variables))]
    pub fn index_and_store_rules(
        &mut self,
        root_path: PathBuf,
        project_rule_content_reader: ProjectRuleContentReader,
        ctx: &mut ModelContext<Self>,
    ) -> Result<()> {
        #[cfg(feature = "local_fs")]
        {
            let repo_path = StandardizedPath::from_local_canonicalized(&root_path)?;
            let repo_id = RepositoryIdentifier::local(repo_path.clone());
            if RepoMetadataModel::as_ref(ctx)
                .standing_query_results(&repo_id, ctx)
                .is_none()
            {
                RepoMetadataModel::handle(ctx).update(ctx, |metadata, ctx| {
                    metadata.index_lazy_loaded_path(&repo_path, ctx)
                })?;
            }
            self.refresh_project_rules_for_repo(repo_id, project_rule_content_reader, ctx);
        }
        Ok(())
    }

    #[cfg(feature = "local_fs")]
    fn refresh_project_rules_for_repo(
        &mut self,
        repo_id: RepositoryIdentifier,
        project_rule_content_reader: ProjectRuleContentReader,
        ctx: &mut ModelContext<Self>,
    ) {
        if repo_id.to_local_or_remote_path().is_none() {
            return;
        };
        let rule_paths = standing_project_rule_paths(
            &repo_id,
            RepoMetadataModel::as_ref(ctx)
                .standing_query_results(&repo_id, ctx)
                .into_iter()
                .flat_map(|results| results.project_rules()),
        );
        let read_rule_contents = project_rule_content_reader(rule_paths.clone(), ctx);

        self.next_rule_refresh_generation += 1;
        let refresh_generation = self.next_rule_refresh_generation;
        self.rule_refresh_generations
            .insert(repo_id.clone(), refresh_generation);
        let repo_id_for_result = repo_id.clone();
        ctx.spawn(read_rule_contents, move |me, result, ctx| {
            if me.rule_refresh_generations.get(&repo_id_for_result) != Some(&refresh_generation) {
                return;
            }
            match result {
                Ok(contents) => {
                    let Some(project_root) = repo_id_for_result.to_local_or_remote_path() else {
                        return;
                    };
                    let existing_rules = me
                        .path_to_rules
                        .get(&project_root)
                        .cloned()
                        .unwrap_or_default();
                    let rules = Self::reconcile_project_rules(rule_paths, contents, existing_rules);
                    me.apply_project_rules(repo_id_for_result, rules, ctx);
                }
                Err(error) => log::warn!("Failed to read project rules: {error}"),
            }
        });
    }

    #[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
    fn reconcile_project_rules(
        rule_paths: Vec<LocalOrRemotePath>,
        rule_contents: ProjectRuleContents,
        mut existing_rules: ProjectRules,
    ) -> ProjectRules {
        let retained_paths = rule_paths.iter().cloned().collect::<HashSet<_>>();
        existing_rules.retain_rule_paths(&retained_paths);
        for (path, content) in rule_contents {
            existing_rules.upsert_rule(&path, content);
        }
        existing_rules
    }

    #[cfg(feature = "local_fs")]
    fn remove_project_rules_for_repo(
        &mut self,
        repo_id: &RepositoryIdentifier,
        ctx: &mut ModelContext<Self>,
    ) {
        self.rule_refresh_generations.remove(repo_id);
        let Some(project_root) = repo_id.to_local_or_remote_path() else {
            return;
        };
        if let Some(rules) = self.path_to_rules.remove(&project_root) {
            // KnownRulesChanged is consumed by local persistence and carries local PathBufs.
            // Remote removals still update in-memory state and emit PathIndexed below.
            if matches!(repo_id, RepositoryIdentifier::Local(_)) {
                let deleted_rules = rules.local_rule_paths().collect();
                ctx.emit(ProjectContextModelEvent::KnownRulesChanged(RulesDelta {
                    discovered_rules: Vec::new(),
                    deleted_rules,
                }));
            }
            ctx.emit(ProjectContextModelEvent::PathIndexed);
        }
    }

    #[cfg(feature = "local_fs")]
    fn apply_project_rules(
        &mut self,
        repo_id: RepositoryIdentifier,
        rules: ProjectRules,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(project_root) = repo_id.to_local_or_remote_path() else {
            return;
        };
        if let RepositoryIdentifier::Local(local_root) = &repo_id {
            let Some(local_root) = local_root.to_local_path() else {
                return;
            };
            let new_paths = rules.local_rule_paths().collect::<Vec<_>>();
            let previous = self
                .path_to_rules
                .insert(project_root, rules)
                .unwrap_or_default();
            let deleted_rules = previous
                .local_rule_paths()
                .filter(|path| !new_paths.contains(path))
                .collect();
            let discovered_rules = new_paths
                .into_iter()
                .map(|path| ProjectRulePath {
                    path,
                    project_root: local_root.clone(),
                })
                .collect();
            ctx.emit(ProjectContextModelEvent::KnownRulesChanged(RulesDelta {
                discovered_rules,
                deleted_rules,
            }));
        } else {
            self.path_to_rules.insert(project_root, rules);
        }
        ctx.emit(ProjectContextModelEvent::PathIndexed);
    }

    /// Index all configured global rule sources.
    ///
    /// `ProjectContextModel` remains the public rule-context facade; the
    /// global source registry, cache, and watcher plumbing live in
    /// `global_rules`.
    pub fn index_global_rules(&mut self, ctx: &mut ModelContext<Self>) {
        self.global_rules.index(ctx);
    }

    /// Project-only rule lookup. Returns `Some` only when an indexed project
    /// root above `path` actually contributes a rule — globals are
    /// deliberately ignored.
    ///
    /// Use this for callers that need a project-initialization signal rather
    /// than the full rule context sent to agents.
    pub fn find_applicable_project_rules(
        &self,
        path: &LocalOrRemotePath,
    ) -> Option<ProjectRulesResult> {
        let mut current_path = path.clone();

        // Walk upwards from `path` toward the filesystem root, stopping at the
        // first directory we have indexed project rules for. `path_to_rules`
        // is keyed by indexed project root, so popping the path produces
        // every ancestor directory until we hit a known root or `pop()`
        // returns false (we've reached the top of the path).
        loop {
            if let Some(rules) = self.path_to_rules.get(&current_path) {
                let result = rules.find_active_or_applicable_rules(path);
                if result.active_rules.is_empty() && result.available_rule_paths.is_empty() {
                    return None;
                }
                return Some(ProjectRulesResult {
                    root_path: current_path,
                    active_rules: result.active_rules,
                    additional_rule_paths: result.available_rule_paths,
                });
            }

            current_path = current_path.parent()?;
        }
    }

    /// Returns the rules applicable to `path`, layering global rules on top of
    /// any project rules discovered up the directory tree.
    ///
    /// Precedence is `global > project WARP.md > project AGENTS.md`. Globals
    /// are always included (when present) regardless of project state; the
    /// existing in-directory `WARP.md > AGENTS.md` shadow inside
    /// [`RuleAtPath::respected_rule`] still applies to project rules.
    ///
    /// This is the entry point used by `BlocklistAIContextModel` when packing
    /// `AIAgentContext::ProjectRules` for an agent query. Callers that need
    /// a project-only signal should use
    /// [`Self::find_applicable_project_rules`] instead.
    pub fn find_applicable_rules(&self, path: &LocalOrRemotePath) -> Option<ProjectRulesResult> {
        let project_result = self.find_applicable_project_rules(path);

        // Layered precedence: global rules are always included alongside
        // project rules. `global_rules` is a `BTreeMap`, so iteration is
        // sorted by path — deterministic without needing a separate
        // ordering pass.
        let mut active_rules: Vec<ProjectRule> = self.global_rules.active_rules().collect();
        if let Some(remote) = path.as_remote() {
            active_rules.extend(
                self.remote_global_rules
                    .get(&remote.host_id)
                    .into_iter()
                    .flatten()
                    .cloned(),
            );
        }
        let (project_root, additional_rule_paths) = match project_result {
            Some(project) => {
                active_rules.extend(project.active_rules);
                (Some(project.root_path), project.additional_rule_paths)
            }
            None => (None, Vec::new()),
        };

        if active_rules.is_empty() && additional_rule_paths.is_empty() {
            return None;
        }

        // Use the indexed project root when available; otherwise fall back to
        // the parent of the first local or remote global rule.
        let root_path =
            project_root.or_else(|| active_rules.first().and_then(|rule| rule.path.parent()))?;

        Some(ProjectRulesResult {
            root_path,
            active_rules,
            additional_rule_paths,
        })
    }

    /// 规则查询的统一入口:正常路径优先,异步索引未就绪时同步 fast-path 兜底。
    ///
    /// 对齐 opencode `Instruction.systemPaths()` 的 `findUp` 行为(
    /// `opencode/packages/opencode/src/session/instruction.ts`):从 cwd 起逐级
    /// 向上 stat 规则文件,首层命中即停。fast-path 与正常路径**绝不并存**:
    /// 正常路径一返回 Some,立即清掉 fast-path cache 中对应条目,确保索引完成后
    /// 后续请求百分百走正常路径(能拿到子目录规则 + watcher 实时更新)。
    #[cfg_attr(not(feature = "local_fs"), allow(unused_variables))]
    pub fn find_rules_with_fast_path(&self, cwd: &Path) -> Option<ProjectRulesResult> {
        if let Some(found) = self.find_applicable_rules(&LocalOrRemotePath::Local(cwd.to_path_buf()))
        {
            #[cfg(feature = "local_fs")]
            {
                // 正常路径已可用,丢弃 fast-path cache(避免后续拿到 stale 数据)。
                self.fast_path_cache.borrow_mut().remove(cwd);
            }
            return Some(found);
        }

        #[cfg(feature = "local_fs")]
        {
            return self.fast_path_lookup(cwd);
        }

        #[allow(unreachable_code)]
        None
    }

    /// Fast-path 同步查找 + 读取 cwd 及祖先目录的规则文件。只在正常路径 None 时调。
    ///
    /// 返回语义与 `find_applicable_rules` 一致:
    ///   - Some(ProjectRulesResult) 带至少 1 个 active rule
    ///   - None 表示未找到任何规则(已写 negative cache,后续相同 stamps 不再 IO)
    #[cfg(feature = "local_fs")]
    fn fast_path_lookup(&self, cwd: &Path) -> Option<ProjectRulesResult> {
        // 1) 缓存命中路径:stat 一遍 stamps,全部对齐则复用缓存(不重读文件)。
        if let Some(entry) = self.fast_path_cache.borrow().get(cwd).cloned() {
            if Self::fast_path_entry_still_valid(&entry) {
                return Self::result_from_fast_path_entry(&entry);
            }
        }

        // 2) 缓存 miss / 失效:同步扫描。预算 `FAST_PATH_BUDGET` 硬截断,UI 绝不卡。
        let entry = Self::scan_fast_path(cwd);
        let result = Self::result_from_fast_path_entry(&entry);
        self.fast_path_cache
            .borrow_mut()
            .insert(cwd.to_path_buf(), entry);
        result
    }

    /// 从 `start` 起逐级向上同步 stat + 读取规则文件。对齐 opencode `findUp`,
    /// 但添加 `MAX_WALK_DEPTH` + `FAST_PATH_BUDGET` 双保障让 UI 绝不阻塞。
    ///
    /// 每层依 `RULES_FILE_PATTERN`(WARP.md > AGENTS.md > CLAUDE.md)取首个命中的,对齐
    /// `RuleAtPath::respected_rule()` 语义。
    #[cfg(feature = "local_fs")]
    fn scan_fast_path(start: &Path) -> FastPathEntry {
        let deadline = Instant::now() + FAST_PATH_BUDGET;
        let mut rules = Vec::new();
        let mut stamps = Vec::new();
        let mut walked_dir_stamps = Vec::new();
        let mut first_hit_dir: Option<PathBuf> = None;
        let mut current: PathBuf = start.to_path_buf();

        for _ in 0..MAX_WALK_DEPTH {
            if Instant::now() >= deadline {
                break;
            }

            // 记录目录 mtime,后续可以识别"目录里新增/删除了规则文件"两类变动。
            if let Ok(meta) = std::fs::metadata(&current) {
                if let Ok(mtime) = meta.modified() {
                    walked_dir_stamps.push((current.clone(), mtime));
                }
            }

            // 本层按优先级查找首个规则文件。对齐 RuleAtPath::respected_rule() 语义。
            for filename in RULES_FILE_PATTERN {
                if Instant::now() >= deadline {
                    break;
                }
                let candidate = current.join(filename);
                let Ok(meta) = std::fs::metadata(&candidate) else {
                    continue;
                };
                if !meta.is_file() {
                    continue;
                }
                let Ok(mtime) = meta.modified() else { continue };
                let size = meta.len();
                let Ok(content) = std::fs::read_to_string(&candidate) else {
                    continue;
                };
                if first_hit_dir.is_none() {
                    first_hit_dir = Some(current.clone());
                }
                rules.push(ProjectRule {
                    path: LocalOrRemotePath::Local(candidate.clone()),
                    content,
                });
                stamps.push((candidate, mtime, size));
                break; // 本层只取 1 个
            }

            if !current.pop() {
                break;
            }
        }

        FastPathEntry {
            root_path: first_hit_dir.unwrap_or_else(|| start.to_path_buf()),
            rules,
            stamps,
            walked_dir_stamps,
        }
    }

    /// 缓存失效检查。只 stat,不读文件内容。
    /// - 命中文件 mtime/size 不变 → 内容可复用
    /// - 遍历过的目录 mtime 不变 → 不会有新增/删除的规则文件
    ///
    /// 带 `FAST_PATH_BUDGET` 预算,stat 期间超时即视为失效重扫。
    #[cfg(feature = "local_fs")]
    fn fast_path_entry_still_valid(entry: &FastPathEntry) -> bool {
        let deadline = Instant::now() + FAST_PATH_BUDGET;
        for (path, mtime, size) in &entry.stamps {
            if Instant::now() >= deadline {
                return false;
            }
            let Ok(meta) = std::fs::metadata(path) else {
                return false;
            };
            if meta.len() != *size {
                return false;
            }
            if meta.modified().ok().as_ref() != Some(mtime) {
                return false;
            }
        }
        for (dir, mtime) in &entry.walked_dir_stamps {
            if Instant::now() >= deadline {
                return false;
            }
            let Ok(meta) = std::fs::metadata(dir) else {
                return false;
            };
            if meta.modified().ok().as_ref() != Some(mtime) {
                return false;
            }
        }
        true
    }

    /// 把 FastPathEntry 转换为对外统一的 `ProjectRulesResult`。
    /// 空 rules 返 None,语义对齐 `find_applicable_rules`。
    #[cfg(feature = "local_fs")]
    fn result_from_fast_path_entry(entry: &FastPathEntry) -> Option<ProjectRulesResult> {
        if entry.rules.is_empty() {
            return None;
        }
        Some(ProjectRulesResult {
            root_path: LocalOrRemotePath::Local(entry.root_path.clone()),
            active_rules: entry.rules.clone(),
            additional_rule_paths: Vec::new(),
        })
    }

    #[cfg(feature = "local_fs")]
    async fn read_persisted_rules(
        rule_paths: Vec<ProjectRulePath>,
    ) -> HashMap<LocalOrRemotePath, ProjectRules> {
        let mut rules: HashMap<LocalOrRemotePath, ProjectRules> = HashMap::new();

        for rule in rule_paths {
            match async_fs::read_to_string(&rule.path).await {
                Ok(content) => {
                    let existing_rules = rules
                        .entry(LocalOrRemotePath::Local(rule.project_root))
                        .or_default();
                    existing_rules.upsert_rule(&LocalOrRemotePath::Local(rule.path), content);
                }
                Err(e) => {
                    log::debug!(
                        "Failed to read rule file from persistence {}: {}",
                        rule.path.display(),
                        e
                    );
                    // Continue processing other files even if one fails
                }
            }
        }

        rules
    }

    pub fn indexed_rules(&self) -> impl Iterator<Item = LocalOrRemotePath> + '_ {
        self.path_to_rules.values().flat_map(|rules| {
            rules.rules.iter().filter_map(|rules| {
                rules
                    .respected_rule()
                    .map(|project_rule| project_rule.path.clone())
            })
        })
    }

    /// Absolute locations of every indexed global rule file (e.g. `~/.agents/AGENTS.md`).
    /// Iteration order is sorted by path because global rules are backed by a `BTreeMap`.
    pub fn global_rule_paths(&self) -> impl Iterator<Item = LocalOrRemotePath> + '_ {
        self.global_rules.paths()
    }

    /// Returns every indexed global rule with its cached content, sorted by path.
    pub fn global_rules(&self) -> impl Iterator<Item = ProjectRule> + '_ {
        self.global_rules.active_rules()
    }
    /// Replaces the file-based global rule catalog for one remote host.
    pub fn set_remote_global_rules(&mut self, host_id: HostId, mut rules: Vec<ProjectRule>) {
        rules.sort_by_key(|rule| rule.path.display_path());
        self.remote_global_rules.insert(host_id, rules);
    }

    /// Removes the file-based global rule catalog for a disconnected remote host.
    pub fn remove_remote_global_rules(&mut self, host_id: &HostId) {
        self.remote_global_rules.remove(host_id);
    }

    /// Returns the rule file paths associated with a specific workspace root path.
    pub fn rules_for_workspace(&self, workspace_path: &Path) -> Vec<PathBuf> {
        self.path_to_rules
            .get(&LocalOrRemotePath::Local(workspace_path.to_path_buf()))
            .into_iter()
            .flat_map(|rules| {
                rules.rules.iter().filter_map(|rule| {
                    rule.respected_rule().and_then(|project_rule| {
                        project_rule.path.to_local_path().map(Path::to_path_buf)
                    })
                })
            })
            .collect()
    }
}

impl Entity for ProjectContextModel {
    type Event = ProjectContextModelEvent;
}

impl SingletonEntity for ProjectContextModel {}

#[cfg(test)]
#[path = "model_tests.rs"]
mod tests;

# InfiniShell Desktop Roadmap

InfiniShell Desktop is the open-source desktop app in the
[InfiniShell product lineup](https://infinishell.dev/#lineup). It gives individual
developers and terminal users a local-first AI terminal on Linux, macOS and
Windows. Users bring their own models, CLI agents and SSH hosts, while credentials,
history and agent state stay on their machines by default.

This roadmap covers **InfiniShell Desktop only**. It no longer uses phases to
describe the entire product lineup as the evolution of one shared runtime. Shipped
code, release notes and the relevant specs are the source of truth for individual
features.

## Product-line boundaries

The three products share an AI-native operations philosophy of understanding goals,
controlling execution and retaining useful experience. They are nevertheless
distinct products with clear boundaries and do not need to share one client,
account system or agent runtime.

| Product | Form and audience | Relationship to this roadmap |
|---|---|---|
| **InfiniShell Desktop** | Open-source desktop software for individual developers and terminal users | Scope of this repository and roadmap |
| **InfiniShell Box** | A local AI operations appliance for individual operators and small teams | Separate product, not scheduled in this repository |
| **InfiniShell Enterprise** | A privately deployed platform for operations, security and DevOps/SRE teams | A separate project owns the web, multi-user, permissions, audit, knowledge and collaboration capabilities |

The lineup may reuse proven concepts, protocols or components, but reuse must not
compromise Desktop's local-first, account-free, independently useful and open-access
boundaries.

## Ongoing directions

### 1. An open, local agent experience

- Keep improving BYOP protocol compatibility across streaming, tool calls,
  reasoning, multimodal input, context compaction and long-task recovery.
- Give the built-in agent, third-party CLI agents, MCP and Skills a consistent
  experience across Blocks, approvals, notifications and conversation lifecycle.
- Keep system prompts, tool permissions, command allowlists and conversation
  persistence visible, configurable and auditable.
- Do not bind Desktop to one model, provider or InfiniShell-hosted gateway.

### 2. Personal and project-scoped SSH operations

- Stabilize SSH, shell integration and remote-server paths from macOS, Linux and
  Windows clients to POSIX and Windows PowerShell remotes, while preserving safe
  fallback for complex OpenSSH configurations.
- Improve SSH host management, project inventories, repositories and rules so the
  agent can understand target machines reliably.
- Strengthen approvals, canary execution, timeouts, result aggregation and failure
  boundaries for single-host and cross-host commands.
- Progressively improve fully local project and machine memory so experience can be
  reused while remaining under the user's control.

### 3. Complete the local-first and privacy boundary

- Continue removing inherited remnants of upstream accounts, teams, billing,
  sharing, Drive sync, cloud conversations, telemetry and error reporting.
- Clarify the storage boundaries of local SQLite, OS keychains and configuration
  files so data is understandable, portable and recoverable.
- Audit network paths. Apart from model providers, MCP servers, updates and remote
  connections explicitly configured by the user, Desktop should not send data
  implicitly.

### 4. Cross-platform terminal quality

- Maintain build, test, packaging and release gates for all three platforms,
  prioritizing platform differences in the terminal, PTY, shell integration, input
  methods and remote sessions.
- Continue improving the English and Simplified Chinese UI, CJK font and text
  layout, Markdown/code rendering, keyboard operation, performance and
  accessibility.
- Selectively adopt terminal-core improvements from Warp and use regression checks
  to keep removed cloud dependencies from returning to Desktop.

### 5. Coordinate the lineup without conflating it

- Keep understandable shared semantics for hosts, projects, Skills, MCP, approvals
  and execution results, reducing the conceptual cost of moving between Desktop,
  Box and Enterprise.
- Design interoperability protocols only when requirements and security boundaries
  are concrete. Possible future interoperability is not a reason to add a server
  dependency to Desktop.
- Enterprise governance experience can inform Desktop's local safety design, while
  multi-tenancy, centralized control and organization permissions remain in
  Enterprise.

## Current non-goals for this repository

The following capabilities may belong to other InfiniShell products, but they are
not on the current InfiniShell-Desktop roadmap:

- extracting Desktop's agent into a universal harness that must serve multiple
  clients, or making such a rewrite a prerequisite for the existing Rust agent
  stack;
- building a web terminal, IDE client, enterprise console or shared cloud account
  in this repository;
- providing hosted agent clusters, container/VM sandbox scheduling or a Kubernetes
  control plane;
- implementing enterprise multi-user collaboration, RBAC, centralized audit,
  organization-scoped share links or centralized task scheduling;
- building enterprise inbound channels for Slack, Discord, Telegram or issue
  trackers;
- reintroducing mandatory login, cloud sync or a central service that Desktop must
  depend on for product-line interoperability.

Any cross-product capability should define its boundary, protocol and security model
in a product-specific spec rather than appear as an implied future phase of the
Desktop roadmap.

---

[简体中文](./roadmap.zh-CN.md)

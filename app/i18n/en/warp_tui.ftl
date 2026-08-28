# =============================================================================
# SECTION: InfiniShell TUI — startup and zero state
# Files: crates/warp_tui/src/ui.rs, crates/warp_tui/src/zero_state.rs
# =============================================================================

tui-conversation-restoring = Loading session...
tui-conversation-restoring-cancel-hint = Esc or Ctrl-C to cancel and start a new session
tui-conversation-restore-failed = Could not restore conversation: { $message }
tui-press-ctrl-c-to-exit = Press Ctrl-C to exit.
tui-welcome-title = Welcome to InfiniShell TUI
tui-welcome-capabilities-title = What’s different about InfiniShell TUI
tui-welcome-capability-coding-agents = State of the art coding agents
tui-welcome-capability-models = Frontier and open-weight models
tui-welcome-capability-model-routers = Fully customizable model routers
tui-welcome-capability-orchestration = Orchestration for fleets of agents
tui-welcome-capability-shell = Better shell command support
tui-terminal-starting = Starting terminal…
tui-whats-new = What's new
tui-mcp-no-servers = No servers available · run /mcp
tui-mcp-connected = { $count } connected
tui-mcp-starting = { $count } starting
tui-mcp-needs-auth = { $count } { $count ->
        [one] needs authentication
       *[other] need authentication
    }
tui-mcp-stopping = { $count } stopping
tui-mcp-failed = { $count } failed
tui-mcp-offline = { $count } offline
tui-mcp-available = { $count } available
tui-mcp-config-errors = { $count } config { $count ->
        [one] error
       *[other] errors
    }
tui-autoupdate-checking = checking for updates…
tui-autoupdate-updating = updating…
tui-autoupdate-up-to-date = up to date
tui-autoupdate-failed = automatic update failed
tui-autoupdate-restart-required = update installed, restart to apply
tui-dev-build = dev build
tui-project-context-discovering = Discovering project context…
tui-project-rule-loaded = { $file } loaded
tui-project-skills-discovered = { $count } { $count ->
        [one] skill
       *[other] skills
    } discovered

# Menus, selectors, and shared status labels
tui-slash-commands-loading = Loading slash commands…
tui-slash-commands-empty = No slash commands found
tui-slash-theme-auto-current = (currently auto: { $theme })
tui-slash-current-state = (currently { $state })
tui-state-on = on
tui-state-off = off
tui-state-disabled = disabled
tui-state-available = available
tui-state-offline = offline
tui-state-starting = starting…
tui-state-stopping = stopping…
tui-state-authentication-required = authentication required
tui-state-failed-with-message = failed · { $message }
tui-theme-auto = auto
tui-theme-light = light
tui-theme-dark = dark
tui-models-title = Models
tui-models-empty = No models found
tui-model-default-key-connected = (default) (key connected)
tui-model-default = (default)
tui-model-key-connected = (key connected)
tui-model-discount = { $percentage }% off
tui-mcp-search-placeholder = Search MCP servers…
tui-mcp-no-matches = No matching MCP servers
tui-mcp-no-servers-available = No MCP servers available
tui-mcp-servers-title = MCP servers
tui-mcp-provider-config-error = { $provider } config error
tui-mcp-running-tools = running · { $count } { $count ->
        [one] tool
       *[other] tools
    }
tui-skills-title = Skills
tui-skills-empty = No skills found
tui-conversations-loading = Loading conversations…
tui-conversations-empty = No conversations found
tui-conversations-title = Conversations
tui-history-empty = No history
tui-history-no-matches = No matching history
tui-history-title = History
tui-long-running-command-use-agent = { $key }  to use agent
tui-keybinding-select-previous-option = Select the previous option
tui-keybinding-select-next-option = Select the next option
tui-position-of-total = of { $total }{ " " }
tui-badge-default = default
tui-badge-recent = recent
tui-badge-connected = connected
tui-badge-recommended = recommended
tui-no-matches = No matches
tui-retry-with-symbol = ↻ Retry
tui-enter-value-to-continue = Enter a value to continue.
tui-loading = Loading…
tui-search = Search

# Permissions, agent questions, and statusline configuration
tui-keybinding-confirm-permission-response = Confirm the selected permission response
tui-keybinding-edit-requested-action = Edit the requested action
tui-yes = yes
tui-no = no
tui-other = Other
tui-other-ellipsis = Other…
tui-hint-exit-editor = { " " }to exit editor{ "  " }
tui-permission-hint-cancel = { " " }to cancel{ "  " }
tui-hint-run = { " " }to run
tui-keybinding-confirm-highlighted-answer = Select or confirm the highlighted answer
tui-keybinding-advance-multiple-answers = Advance after selecting multiple answers
tui-keybinding-previous-question = Show the previous question
tui-keybinding-next-question = Show the next question
tui-agent-questions = Agent questions
tui-select-all-that-apply = { " " }(select all that apply)
tui-hint-advance = to advance{ " " }
tui-hint-select = to select{ " " }
tui-hint-navigate = to navigate{ " " }
tui-hint-cancel-question = to cancel question
tui-questions-unavailable = Questions unavailable
tui-questions-skipped-auto-approve = Questions skipped due to auto-approve
tui-questions-skipped = Questions skipped
tui-answered-question = Answered question
tui-answered-all-questions = Answered all { $total } questions
tui-answered-some-questions = Answered { $answered } of { $total } questions
tui-skipped = Skipped
tui-question-with-content = Q: { $question }
tui-answer-with-content = A: { $answer }
tui-keybinding-toggle-statusline-item = Toggle the highlighted statusline item
tui-keybinding-save-statusline = Save and close the statusline configuration
tui-keybinding-move-statusline-left = Move the highlighted statusline item left
tui-keybinding-move-statusline-right = Move the highlighted statusline item right
tui-hint-toggle = to toggle{ "  " }
tui-hint-save-and-close = to save and close{ "  " }
tui-hint-reorder = to reorder
tui-configure-statusline = Configure statusline
tui-statusline-auto-approve = Auto-approve indicator
tui-statusline-vim-mode = Vim mode indicator
tui-statusline-model = Model
tui-statusline-working-directory = Working directory
tui-statusline-git-branch = Git branch
tui-statusline-git-branch-status = Git branch status
tui-statusline-git-diff-status = Git diff status
tui-statusline-github-pull-request = GitHub pull request
tui-statusline-credit-usage = Credit usage
tui-statusline-context-window-usage = Context window usage
tui-statusline-date = Date
tui-statusline-time-12-hour = Time (12 hour format)
tui-statusline-time-24-hour = Time (24 hour format)
tui-statusline-agent-todo-list = Agent to-do list
tui-statusline-voice-input = Voice input

# Terminal session, statusline, and transient feedback
tui-settings-invalid-syntax = Settings failed to load: invalid syntax.
tui-settings-invalid-values = Settings failed to load: invalid values.
tui-custom-ascii-initial-load-failed = Could not load custom ASCII art. Using the built-in InfiniShell logo.
tui-custom-ascii-reload-failed = Could not reload custom ASCII art. Keeping the current object.
tui-log-bundle-saved = Log bundle saved to { $path }
tui-log-bundle-failed = Failed to create log bundle (check logs)
tui-cost-no-active-conversation = Cannot show conversation cost: no active conversation
tui-cost-empty-conversation = Cannot show conversation cost: conversation is empty
tui-cost-conversation-in-progress = Cannot show conversation cost: conversation is in progress
tui-hint-cancel-compact = { " " }to cancel
tui-hint-install-enable = to install and enable
tui-hint-start = to start
tui-hint-stop = to stop
tui-hint-retry = to retry
tui-hint-authenticate = to authenticate
tui-hint-logout-remove-credentials = { " " }to log out & remove credentials{ "  " }
tui-hint-close = { " " }to close
tui-conversation-exported-overwritten = Conversation exported to { $path } (overwrote existing file)
tui-conversation-exported = Conversation exported to { $path }
tui-keybinding-use-agent-with-command = Use the agent with the running command
tui-keybinding-return-control-to-command = Return control to the running command
tui-keybinding-accept-terminal-action = Accept the blocked terminal-use action
tui-keybinding-toggle-auto-approve = Toggle auto approve
tui-keybinding-toggle-latest-plan = Toggle the latest plan
tui-keybinding-toggle-visible-plan = Toggle the latest visible plan
tui-keybinding-focus-image-attachments = Focus image attachments
tui-keybinding-paste-clipboard = Paste from the clipboard
tui-keybinding-start-voice-input = Start voice input
tui-keybinding-focus-session-input = Return focus to the session input
tui-keybinding-focus-main-agent-input = Return to the main agent and focus its input
tui-shell-mode = Shell mode
tui-context-remaining = context remaining
tui-ctrl-c-kill-child = ctrl-c again to kill child agent
tui-ctrl-c-exit = ctrl-c again to exit
tui-conversation-loading = Loading conversation…
tui-running-command-return = ctrl-c to return to command
tui-voice-listening-release-to-stop = listening to voice input... · release key to stop
tui-voice-listening-key-to-stop = listening to voice input... · esc or enter to stop
tui-voice-transcribing-cancel = Transcribing... · esc to cancel
tui-voice-label = ◉ Voice
tui-voice-transcribing = … Transcribing
tui-status-version = Version
tui-status-session = Session
tui-status-conversation-id = Conversation ID
tui-status-working-directory = Working directory
tui-status-title = Status
tui-tasks-progress = Tasks { $completed }/{ $total }
tui-untitled = Untitled
tui-none = None
tui-auto-approve-on = Auto approve on
tui-auto-approve-off = Auto approve off
tui-copied-to-clipboard = copied to clipboard
tui-copy-failed = failed to copy to clipboard
tui-command-already-running = cannot run — command already running
tui-new-conversation-command-running = cannot start new conversation while terminal command is running
tui-fork-no-active-conversation = /fork requires an active conversation
tui-fork-empty-conversation = Nothing to fork — start a conversation first.
tui-fork-no-resume-id = This conversation cannot be forked until it has a resume ID.
tui-fork-failed = Conversation forking failed.
tui-switch-command-running = Cannot switch conversations while a command is in progress.
tui-switch-conversation-running = Cannot switch conversations while the current conversation is in progress.
tui-switch-loading = Another conversation is already loading.
tui-switch-unavailable = That conversation is no longer available.
tui-voice-usage = Usage: /voice (no arguments)
tui-forked-conversation-resume-original = Forked conversation. To resume the original in another session, run: { $command }
tui-create-project-prompt-required = Please describe the project you want to create after /create-new-project
tui-conversation-copied-to-clipboard = Conversation copied to clipboard
tui-export-no-active-conversation = No active conversation to export
tui-debugging-info-copied = Debugging information copied to clipboard
tui-debugging-id-unavailable = No debugging ID available for this conversation yet.
tui-statusline-saved = Statusline configuration saved.
tui-statusline-reset = Statusline reset to defaults.
tui-statusline-save-failed = Could not save the statusline configuration.
tui-nld-enabled = Natural language detection enabled.
tui-nld-disabled = Natural language detection disabled.
tui-nld-save-failed = Could not save the natural language detection setting.
tui-vim-mode-enabled = Vim mode enabled.
tui-vim-mode-disabled = Vim mode disabled.
tui-vim-mode-save-failed = Could not save the vim mode setting.
tui-theme-invalid-argument = Theme must be auto, light, or dark.
tui-theme-set-auto = Theme set to auto mode (currently { $theme }).
tui-theme-set = Theme set to { $theme } mode.
tui-theme-save-failed = Could not save the theme setting.
tui-shell-starting = Starting shell...
tui-mcp-installed-starting = { $name } installed and starting
tui-cloud-conversations-load-failed = Could not load cloud conversations. Showing local conversations only.
tui-restore-unsupported-conversation = InfiniShell TUI only supports Oz/Warp conversations.
tui-conversation-load-failed = The conversation could not be loaded.
tui-restored-conversation-mismatch = The restored conversation did not match the requested conversation.
tui-warping-auto-approve-off = ▶▶ Auto approve off
tui-warping-auto-approve-on = ▶▶ Auto approve on
tui-warping-full-access-on = ▶▶ Full access on
tui-voice-input-cancelled = Voice input cancelled
tui-mcp-install-flow-failed = Unable to open the MCP installation flow
tui-summarizing-conversation = Summarizing conversation
tui-warping = Warping

# Tool-call transcript labels
tui-tool-awaiting-approval = { $label } (awaiting approval)
tui-tool-command-generating = Generating command…
tui-tool-command-run = Run `{ $command }`
tui-tool-command-running = Running `{ $command }`
tui-tool-command-ran = Ran `{ $command }`
tui-tool-command-still-running = `{ $command }` is still running
tui-tool-command-exited = `{ $command }` exited with code { $code }
tui-tool-command-denied = `{ $command }` denied (denylisted)
tui-tool-command-failed = `{ $command }` failed
tui-tool-command-cancelled = Cancelled `{ $command }`
tui-tool-command-input-writing = Writing command input…
tui-tool-command-input-write = Write input to running command
tui-tool-command-input-writing-running = Writing input to running command…
tui-tool-command-input-wrote = Wrote input to running command
tui-tool-command-input-failed = Failed to write to running command
tui-tool-command-input-cancelled = Write to running command cancelled
tui-tool-files-reading = Reading files…
tui-tool-files-read = Read { $files }
tui-tool-files-reading-named = Reading { $files }
tui-tool-files-failed = Failed to read { $files }
tui-tool-files-cancelled = Cancelled reading { $files }
tui-tool-grep-starting = Grepping…
tui-tool-grep = Grep for { $queries } in { $path }
tui-tool-grep-running = Grepping for { $queries } in { $path }
tui-tool-grep-succeeded-with-count = Grepped for { $queries } in { $path }, { $count ->
    [one] { $count } matching file
   *[other] { $count } matching files
    }
tui-tool-grep-succeeded = Grepped for { $queries } in { $path }
tui-tool-grep-failed = Grep for { $queries } failed
tui-tool-grep-cancelled = Grep for { $queries } cancelled
tui-tool-mcp-resource-reading = Reading MCP resource…
tui-tool-mcp-resource-reading-name = Reading "{ $name }" MCP resource…
tui-tool-mcp-resource-read = Read MCP resource { $resource }
tui-tool-mcp-resource-reading-uri = Reading MCP resource { $resource }
tui-tool-mcp-resource-failed = MCP resource { $resource } failed
tui-tool-mcp-resource-cancelled = MCP resource { $resource } cancelled
tui-tool-mcp-server-suffix = { " " }on { $server }
tui-tool-mcp-calling = Calling MCP tool{ $suffix }…
tui-tool-mcp-calling-name = Calling "{ $name }" MCP tool{ $suffix }…
tui-tool-mcp-call = Call MCP tool { $name }{ $suffix }
tui-tool-mcp-calling-name-plain = Calling MCP tool { $name }{ $suffix }
tui-tool-mcp-called = Called MCP tool { $name }{ $suffix }
tui-tool-mcp-failed = MCP tool { $name }{ $suffix } failed
tui-tool-mcp-cancelled = MCP tool { $name }{ $suffix } cancelled
tui-tool-new-conversation-suggesting = Suggesting a new conversation…
tui-tool-new-conversation-suggested = Suggested starting a new conversation
tui-tool-current-conversation-continuing = Continuing current conversation
tui-tool-new-conversation-started = New conversation started
tui-tool-new-conversation-cancelled = New conversation suggestion cancelled
tui-tool-documents-reading = Reading documents…
tui-tool-documents-read = Read { $documents }
tui-tool-documents-reading-count = Reading { $documents }
tui-tool-documents-failed = Failed to read documents
tui-tool-documents-cancelled = Cancelled reading documents
tui-tool-plan-update = Update plan
tui-tool-plan-updating = Updating plan…
tui-tool-plan-updated = Updated plan ({ $count ->
    [one] { $count } edit
   *[other] { $count } edits
    })
tui-tool-plan-update-failed = Failed to update plan
tui-tool-plan-update-cancelled = Update plan cancelled
tui-tool-plan-create = Create plan
tui-tool-plan-generating = Generating plan…
tui-tool-documents-created = Created { $count } documents
tui-tool-plan-created = Created plan
tui-tool-plan-create-failed = Failed to create plan
tui-tool-plan-create-cancelled = Create plan cancelled
tui-tool-command-output-read = Read command output
tui-tool-command-output-reading = Reading command output…
tui-tool-command-output-failed = Failed to read command output
tui-tool-command-output-cancelled = Read command output cancelled
tui-tool-review-comments-preparing = Preparing review comments…
tui-tool-review-comments-insert = Insert { $comments }
tui-tool-review-comments-inserting = Inserting { $comments }…
tui-tool-review-comments-inserted = Inserted { $comments }
tui-tool-review-comments-failed = Failed to insert review comments
tui-tool-review-comments-cancelled = Insert review comments cancelled
tui-tool-skill-reading = Reading skill…
tui-tool-skill-read = Read skill { $skill }
tui-tool-skill-reading-name = Reading skill { $skill }
tui-tool-skill-failed = Failed to read skill { $skill }
tui-tool-skill-cancelled = Cancelled reading skill { $skill }
tui-tool-control-transferring = Handing control to you…
tui-tool-control-transferring-reason = Handing control to you: { $reason }
tui-tool-control-transferred = You are in control
tui-tool-control-transfer-failed = Control transfer failed
tui-tool-control-transfer-cancelled = Control transfer cancelled
tui-tool-question-preparing = Preparing question…
tui-tool-questions-asking = { $count ->
    [one] Asking { $count } question
   *[other] Asking { $count } questions
    }
tui-tool-questions-failed = Questions failed
tui-tool-questions-cancelled = Questions cancelled
tui-answered-questions = Answered questions
tui-tool-agents-configuring = Configuring agents…
tui-tool-agents-spawning = { $count ->
    [one] Spawning { $count } agent…
   *[other] Spawning { $count } agents…
    }
tui-tool-agents-spawned = { $count ->
    [one] Spawned { $count } agent
   *[other] Spawned { $count } agents
    }
tui-tool-agents-spawn-failed = { $count ->
    [one] Failed to spawn { $count } agent
   *[other] Failed to spawn { $count } agents
    }
tui-tool-agents-spawned-some = Spawned { $launched } of { $total } agents
tui-tool-orchestration-disabled = Orchestration disabled — agents not launched
tui-tool-orchestration-failed-error = Failed to start orchestration: { $error }
tui-tool-orchestration-failed = Failed to start orchestration
tui-tool-agents-cancelled = Spawn agents cancelled
tui-tool-events-waiting = Waiting for agent events…
tui-tool-events-done = Done waiting for agent events
tui-tool-events-failed = Waiting for agent events failed
tui-tool-events-cancelled = Wait for events cancelled
tui-tool-files-finding = Finding files…
tui-tool-files-find = Find files matching { $patterns } in { $path }
tui-tool-files-finding-pattern = Finding files matching { $patterns } in { $path }
tui-tool-files-found-count = { $count ->
    [one] Found { $count } file matching { $patterns }
   *[other] Found { $count } files matching { $patterns }
    }
tui-tool-files-found = Found files matching { $patterns }
tui-tool-files-search-failed = File search for { $patterns } failed
tui-tool-files-search-cancelled = File search for { $patterns } cancelled
tui-tool-generic-running = { $name }…
tui-tool-generic-done = { $name } — done
tui-tool-generic-failed = { $name } — failed
tui-tool-generic-cancelled = { $name } — cancelled
tui-current-directory = the current directory
tui-files = files
tui-count-files = { $count ->
    [one] { $count } file
   *[other] { $count } files
    }
tui-count-documents = { $count ->
    [one] { $count } document
   *[other] { $count } documents
    }
tui-count-review-comments = { $count ->
    [one] { $count } review comment
   *[other] { $count } review comments
    }

# Credentials, MCP installation, and attachments
tui-api-key-anthropic = Anthropic API key
tui-api-key-google = Google API key
tui-api-key-openai = OpenAI API key
tui-api-key-grok = X premium or SuperGrok subscription
tui-api-key-warp-credit-fallback = Warp credit fallback
tui-api-key-grok-build-unavailable = Grok subscriptions aren't available in this build.
tui-api-key-grok-byok-required = Grok subscriptions require BYOK access for this workspace.
tui-api-key-member-credentials-disallowed = Your organization doesn't allow member-provided credentials.
tui-api-key-clear-failed = Could not clear the selected API key. Try again.
tui-api-keys = API keys
tui-api-key-provider-title = { $provider } API key
tui-api-key-connecting = (Connecting...)
tui-api-key-connected = (Connected)
tui-api-key-not-connected = (Not connected)
tui-api-key-warp-credit-fallback-description = In the event of an error, requests may be routed to use Warp credits. Warp will prioritize using your API keys over Warp credits.
tui-state-on-parenthesized = (on)
tui-state-off-parenthesized = (off)
tui-api-key-grok-already-connected = Grok is already connected. Press Ctrl-X to disconnect.
tui-api-key-save-failed = Could not save this API key. Try again.
tui-api-key-fallback-save-failed = Could not save the Warp credit fallback setting.
tui-hint-set-api-key = to set API key
tui-hint-clear-api-key = to clear API key
tui-hint-close-menu = to close menu
tui-hint-toggle-warp-credit-fallback = to toggle Warp credit fallback
tui-api-key-connect-provider = Connect { $provider } API key
tui-hint-save-key = to save key
tui-hint-cancel = to cancel
tui-api-key-connect-grok = Connect X premium/SuperGrok
tui-hint-confirm = to confirm
tui-mcp-install-inactive = The MCP installation flow is no longer active
tui-mcp-install-not-collecting-variable = The MCP installation flow is not collecting a variable
tui-mcp-install-variable-unavailable = The MCP variable is no longer available
tui-mcp-install-required-value = Enter a value for the required MCP variable
tui-mcp-install-select-listed-value = Select one of the listed values
tui-hint-install-and-enable = to install and enable
tui-hint-continue = to continue
tui-enter-value = Enter value…
tui-mcp-install-title = Install and enable { $name }
tui-mcp-install-enter-variable = Enter a value for { $key } ({ $current }/{ $total })
tui-keybinding-next-attachment = Select the next attachment
tui-keybinding-previous-attachment = Select the previous attachment
tui-keybinding-remove-attachment = Remove the selected attachment
tui-keybinding-return-input-focus = Return focus to the input
tui-attachment-image = [image]
tui-attachment-file = [file]
tui-loading-ellipsis = loading…
tui-image-read-failed = Could not read image { $path }.
tui-image-path-not-file = Image path is not a file: { $path }.
tui-image-too-large-path = Image is too large: { $path }.
tui-image-unsupported-type = Unsupported image type for { $path }. Use PNG, JPG, GIF, or WebP.
tui-image-process-failed = Could not process image { $path }.
tui-image-invalid-filename = Image has no valid filename: { $path }.
tui-clipboard-unavailable = The system clipboard is unavailable.
tui-clipboard-image-data-unavailable = Clipboard image data is unavailable.
tui-clipboard-no-supported-image = The clipboard does not contain a supported image.
tui-clipboard-image-too-large = The clipboard image is too large.
tui-clipboard-image-process-failed = The clipboard image could not be processed.

# Permission cards and terminal-use subagents
tui-permission-read-files = Is it OK if I read these files?
tui-permission-search-files = Is it OK if I search these files?
tui-permission-find-files = Is it OK if I find files matching these patterns?
tui-permission-call-mcp-server = Is it OK if I call an MCP tool on { $server }?
tui-permission-call-mcp = Is it OK if I call this MCP tool?
tui-permission-call-mcp-name-server = Is it OK if I call MCP tool { $name } on { $server }?
tui-permission-call-mcp-name = Is it OK if I call MCP tool { $name }?
tui-permission-read-mcp-resource = Is it OK if I read this MCP resource?
tui-permission-write-command-input = Is it OK if I write this input to the running command?
tui-permission-new-conversation = Should I start a new conversation?
tui-permission-transfer-control = Is it OK if I hand control of the running command to you?
tui-permission-generic-action = Is it OK if I { $action }?
tui-tool-details-in-path = { $content }
      in { $path }
tui-tool-details-mcp-server = { $name } on { $server }
tui-new-conversation-detail = Continue the agent's next step in a fresh conversation.
tui-command-finished = Command finished
tui-agent-needs-input = Agent needs your input
tui-agent-monitoring-command = Agent is monitoring command
tui-agent-waiting-instructions = Agent waiting for instructions
tui-hint-take-control = to take control
tui-user-in-control = User is in control
tui-agent-paused-user-control = Agent paused · user is in control
tui-agent-handed-control = Agent handed control to you
tui-hint-hand-back = to hand back
tui-input-kind-raw = Input
tui-input-kind-line = Line input
tui-input-kind-pasted = Pasted input
tui-agent-wants-write-command = Agent wants to write to the running command
tui-agent-wants-transfer-control = Agent wants to hand command control to you
tui-reason-detail = Reason: { $reason }
tui-agent-wants-read-files = Agent wants to read files
tui-agent-wants-search-files = Agent wants to search file contents
tui-agent-wants-find-files = Agent wants to find files
tui-patterns-path-detail = Patterns: { $patterns }
    Path: { $path }
tui-check-in-seconds = { " " }· Check in { $seconds }s
tui-check-in-minutes = { " " }· Check in { $minutes }m
tui-last-instruction = Last instruction: { $instruction }
tui-allow-ctrl-enter = Allow · Ctrl+Enter
tui-reject-ctrl-c = Reject · Ctrl+C

# Plans, file edits, and shell-command permissions
tui-creating = Creating
tui-created = Created
tui-plan-updating = Updating plan
tui-plan-updated = Updated plan
tui-hint-collapse-plan = { $binding } to collapse plan
tui-keybinding-toggle-all-diffs = Expand or collapse all diffs
tui-files-edited-with-stats = { $count ->
    [one] Edited { $count } file ({ $stats })
   *[other] Edited { $count } files ({ $stats })
    }
tui-files-edited = { $count ->
    [one] Edited { $count } file
   *[other] Edited { $count } files
    }
tui-file-edits-cancelled = File edits cancelled
tui-file-edits-failed = File edits failed
tui-file-edits-preparing = Preparing edits…
tui-editing = Editing
tui-file-edit-header = { $verb } { $subject }
tui-deleted = Deleted
tui-updated = Updated
tui-edited = Edited
tui-hint-expand-collapse = to expand/collapse
tui-permission-file-edits = Is it OK if I make these file edits?
tui-keybinding-save-shell-command = Save the edited shell command
tui-enter-command-to-continue = Enter a command to continue.
tui-hint-edit-command = to edit command
tui-permission-run-command = Is it OK if I run this command and read the output?

# Cloud runs, handoff, and orchestration
tui-keybinding-open-cloud-run-link = Open the cloud run link
tui-keybinding-focus-orchestration-tabs = Focus the orchestration tab bar
tui-cloud-run-starting = Starting cloud run…
tui-github-auth-required = GitHub Authentication Required
tui-github-auth-rerun-orchestration = Authenticate with GitHub, then run the orchestration request again.
tui-hint-authenticate-or-click-link = to authenticate or click the link below
tui-cloud-environment-start-failed = Failed to start environment
tui-cloud-run-in-progress = Cloud run in progress
tui-cloud-run-blocked = Cloud run blocked
tui-cloud-run-succeeded = Cloud run succeeded
tui-cloud-run-failed = Cloud run failed
tui-cloud-run-cancelled = Cloud run cancelled
tui-hint-view-or-click-link = to view or click the link below
tui-press = Press
tui-hint-focus-subagents = Shift + ↑ sub-agents
tui-handoff-environment-question = Which environment should run this conversation?
tui-handoff-model-question = Which model should run this conversation?
tui-handoff-unavailable = Cloud handoff is unavailable.
tui-handoff-command-running = Can't hand off while a command is running. Cancel it or wait for it to finish.
tui-handoff-child-active = Can't hand off while child work is active or waiting for input.
tui-handoff-nothing = Nothing to hand off — start a conversation or add a prompt.
tui-handoff-not-synced = This conversation hasn't synced yet. Send another message, then try again.
tui-handoff-invalid-model = The selected model can't run in Oz cloud.
tui-handoff-start-failed = Couldn't start the handoff. Check the current conversation and try again.
tui-handoff-select-environment-first = Select an environment before starting the handoff.
tui-handoff-environment-unavailable = The selected environment is no longer available.
tui-handoff-choose-compatible-model = The selected model cannot run in Oz cloud. Choose a compatible model.
tui-handoff-no-longer-available = Cloud handoff is no longer available.
tui-handoff-return-local = The handoff can no longer start. Return to local input and try again.
tui-no-cloud-environments = No cloud environments available
tui-select-environment = Select an environment
tui-incompatible-label = { $label } (incompatible)
tui-handoff-network-failed = Couldn't start the handoff. Check your network connection and try again.
tui-orchestration-location-question = { $count ->
    [one] Where should the agent run?
   *[other] Where should the agents run?
    }
tui-orchestration-harness-question = { $count ->
    [one] Which harness should the agent use?
   *[other] Which harness should the agents use?
    }
tui-orchestration-api-key-question = { $count ->
    [one] Which API key should the agent use?
   *[other] Which API key should the agents use?
    }
tui-orchestration-host-question = { $count ->
    [one] Which host should run the agent?
   *[other] Which host should run the agents?
    }
tui-orchestration-environment-question = { $count ->
    [one] Which environment should the agent use?
   *[other] Which environment should the agents use?
    }
tui-orchestration-model-question = { $count ->
    [one] Which model should the agent use?
   *[other] Which model should the agents use?
    }
tui-location = Location
tui-cloud = Cloud
tui-local = Local
tui-harness = Harness
tui-skip-advanced = Skip (advanced)
tui-select-api-key = Select an API key
tui-api-key = API key
tui-host = Host
tui-environment = Environment
tui-empty-environment = Empty environment
tui-model = Model
tui-default-model = Default model
tui-agents-count-heading = Agents ({ $count }):
tui-orchestration-permission-title = Can I start additional agents for this task?
tui-hint-accept = to accept
tui-hint-edit = to edit
tui-hint-reject = to reject
tui-hint-go-back = to go back
tui-edit-agent-configuration = Edit agent configuration
tui-keybinding-previous-orchestration-tab = Select the previous orchestration tab
tui-keybinding-next-orchestration-tab = Select the next orchestration tab
tui-keybinding-first-child-agent = Select the first child agent
tui-keybinding-last-child-agent = Select the last child agent
tui-agents = Agents
tui-orchestrator = orchestrator
tui-hint-go-start-end = to go to start/end
tui-hint-send-message = to send a message
tui-hint-kill-subagent = to kill sub-agent
tui-local-harness-child-unsupported = Local { $harness } child agents aren't supported in InfiniShell TUI yet.
tui-remote-child-unavailable = Remote child agents are unavailable in this local build.
tui-local-child-create-failed = Failed to create local child task: { $error }

# Input hints, shortcuts, editor bindings, voice, and review labels
tui-shell-input-hint = Run a shell command • ? for shortcuts • esc for agent mode
tui-terminal = Terminal
tui-shortcut-interrupt-command = interrupt command
tui-shortcut-hand-back-control = hand back control
tui-shortcuts-lowercase = shortcuts
tui-commands-lowercase = commands
tui-shell-mode-lowercase = shell mode
tui-conversations-lowercase = conversations
tui-agent-mode-lowercase = agent mode
tui-input-history-lowercase = input history
tui-toggle-auto-approve-lowercase = toggle auto-approve
tui-expand-collapse-plans-lowercase = expand/collapse plans
tui-shortcuts = Shortcuts
tui-terminal-use = Terminal use
tui-take-control-lowercase = take control
tui-return-control-command-lowercase = return control to command
tui-orchestration = Orchestration
tui-navigate-agents-lowercase = navigate to agents
tui-hint-shortcuts = ? for shortcuts
tui-hint-other-agents = Shift + ↑ for other agents
tui-hint-commands = / for commands
tui-hint-conversations = ← for conversations
tui-hint-ask-agent = Ask the agent anything
tui-hint-shell-mode = ! for shell mode
tui-editor-insert-newline = Insert a newline
tui-editor-delete-previous-character = Delete the previous character
tui-editor-delete-next-character = Delete the next character
tui-editor-delete-previous-word = Delete the previous word
tui-editor-delete-next-word = Delete the next word
tui-editor-move-left = Move cursor left
tui-editor-move-right = Move cursor right
tui-editor-move-up = Move cursor up
tui-editor-move-down = Move cursor down
tui-editor-move-word-left = Move cursor one word left
tui-editor-move-word-right = Move cursor one word right
tui-editor-move-line-start = Move cursor to start of line
tui-editor-move-line-end = Move cursor to end of line
tui-editor-select-left = Extend selection left
tui-editor-select-right = Extend selection right
tui-editor-select-up = Extend selection up
tui-editor-select-down = Extend selection down
tui-editor-select-word-left = Extend selection one word left
tui-editor-select-word-right = Extend selection one word right
tui-editor-select-line-start = Extend selection to start of line
tui-editor-select-line-end = Extend selection to end of line
tui-editor-select-all = Select all text
tui-editor-copy = Copy selected text
tui-editor-cut = Cut selected text
tui-editor-paste = Paste text from the clipboard
tui-editor-delete-line-end = Delete to end of line
tui-editor-delete-line-start = Delete to start of line
tui-editor-yank = Paste the last deleted text
tui-editor-undo = Undo
tui-editor-redo = Redo
tui-keybinding-submit-input = Submit the input
tui-keybinding-contextual-escape = Handle contextual input escape
tui-keybinding-mcp-logout = Log out of the selected MCP server and remove its credentials
tui-keybinding-complete-shell-command = Complete the shell command
tui-voice-input-unavailable = Voice input is unavailable
tui-microphone-access-denied = Microphone access denied
tui-voice-input-start-failed = Unable to start voice input
tui-voice-input-stop-failed = Failed to stop voice input
tui-voice-input-stopped = Voice input stopped
tui-voice-transcription-unavailable = Voice transcription is unavailable
tui-voice-input-limit-reached = Voice input limit reached
tui-voice-transcription-failed = Failed to transcribe voice input
tui-image-attachments-unavailable = Image attachments are unavailable.
tui-image-attachment-wait = Wait for the current image attachment to finish.
tui-image-attachment-limit = Image attachment limit is { $count } per query.
tui-model-no-image-attachments = The selected model does not support image attachments.
tui-grok-enter-browser-code = Enter the code shown in your browser to finish connecting.
tui-grok-authorization-failed = Couldn't complete Grok authorization. Press Esc, then select Grok to try again.
tui-grok-code-failed = Couldn't connect Grok with that code. Check the code and try again.
tui-pull-request = Pull request
tui-diff-side-new = new
tui-diff-side-old = old
tui-diff-side-diff = diff

# Command-line interface
tui-cli-provider-api-key-prompt = Provider API key:
tui-cli-invalid-resume-token = invalid server conversation token: { $token }
tui-cli-about = InfiniShell's terminal user interface
tui-cli-help-resume = Resume an Oz/Warp conversation by server token
tui-cli-help-auto-approve = Enable auto-approve by default for new conversations
tui-cli-help-full-access = Use Full Access for new conversations, subject to organization and sandbox policy
tui-cli-help-api-key = API key for non-interactive authentication
tui-cli-help-set-provider-api-key = Securely store a model-provider API key for InfiniShell TUI
tui-cli-help-clear-provider-api-key = Remove a securely stored model-provider API key from InfiniShell TUI
tui-cli-grok-connect-in-tui = Grok credentials must be connected with /api-keys in an active TUI
tui-cli-no-provider-api-key = No provider API key was supplied
tui-cli-grok-clear-in-tui = Grok credentials must be cleared with /api-keys in an active TUI
tui-cli-provider-api-key-saved = { $provider } API key saved
tui-cli-provider-api-key-cleared = { $provider } API key cleared
tui-cli-continue-conversation = To continue this conversation, run:

# Transcript sections and quota notices
tui-thought-for-duration = Thought for { $duration }
tui-thinking-ellipsis = Thinking...
tui-conversation-summary = Conversation summary
tui-image-without-description = [Image without description]
tui-image-label = Image
tui-cost-unavailable = Cost unavailable
tui-orchestrator-title = Orchestrator
tui-unknown-agent = Unknown agent
tui-key-enter-or-number = Enter or number
tui-key-tab-or-arrows = Tab or ← →
tui-code-block-truncated = … code block truncated …
tui-first-credit-title = You need AI credits in order to use InfiniShell TUI.
tui-first-credit-action = Start using AI
tui-out-of-credits-title = I’m sorry, I couldn’t complete that request.
tui-out-of-credits-detail = In order to use InfiniShell’s AI features, subscribe to a Warp plan or buy packs of credits.
tui-out-of-credits-action = Get started with AI
tui-failed-output-usage-notice = This response won't count towards your usage.
tui-tasks-count = Tasks { $count }
tui-completed-task-item = Completed { $title }{ $position }
tui-additional-task-item = , { $title }{ $position }
tui-notification-session-started = InfiniShell TUI session started.
tui-notification-working = InfiniShell TUI is working on your request.
tui-notification-completed = InfiniShell TUI completed your request.
tui-notification-error = InfiniShell TUI encountered an error.
tui-notification-cancelled = InfiniShell TUI was cancelled.
tui-notification-waiting-input = InfiniShell TUI is waiting for your input.
tui-notification-reconnecting = InfiniShell TUI is reconnecting.
tui-notification-waiting-events = InfiniShell TUI is waiting for events.
tui-notification-task-cancelled = The task was cancelled before completion.
tui-image-description = Image: { $description }
tui-hidden-lines = … { $count ->
    [one] { $count } line
   *[other] { $count } lines
    }
tui-diff-range-out-of-bounds = Diff range { $range } is out of bounds for file with { $count } lines
tui-invalid-remote-path = Invalid remote path: { $path }
tui-agent = Agent
tui-unsupported-embedded-content = [Unsupported embedded content]
tui-empty-table = [Empty table]
tui-table-has-no-rows = [Table has no rows]
tui-code-block-unavailable = [Code block unavailable]
tui-elapsed-seconds-compact = ({ $count }s)
tui-session-state-terminal-unavailable = Terminal model is unavailable
tui-session-state-cli-controller-unavailable = CLI subagent controller is unavailable
tui-session-state-transcript-unavailable = Transcript view is unavailable
tui-session-state-input-mode-unavailable = Input-mode model is unavailable
tui-session-state-suggestions-mode-unavailable = Suggestions-mode model is unavailable
tui-session-state-orchestration-tabs-unavailable = Orchestration tab bar is unavailable

# Configuring a BYOP Responses API provider

[简体中文](./ai-provider-responses.zh-CN.md)

This guide is for people connecting InfiniShell to the OpenAI Responses API or
a compatible gateway. It explains what each provider setting changes, which
settings can be combined, and what to check first when a provider returns a
common `400 Bad Request`.

> Third-party gateways may implement only part of the Responses API. Unless a
> provider explicitly documents an advanced capability, start with the
> compatibility defaults below. Enable one advanced option at a time after
> basic chat and tool calling work.

## Open the provider settings

Open **Settings → AI → Providers**, then add or expand a provider. A leading
`●` marks the selected option.

For an OpenAI-compatible endpoint that supports `/v1/responses`, start with:

| Setting | Recommended value |
|---|---|
| API Type | `OpenAI-Response` |
| Responses state | `Local / ZDR` |
| Transport | `HTTP + SSE` |
| Automatic compaction | `Off` |
| Pro mode | Off |
| All-turn reasoning context | Off, use the provider default |
| Background + resume | Off |
| Programmatic tool calling | Off |
| Extra Headers | Empty unless required by the provider |

This profile prioritizes privacy and compatibility. Advanced switches are not
a “more is better” checklist; each one requires matching server support.

## API Type

API Type selects the request format and response parser. It describes the
protocol the endpoint actually accepts, not merely the company that supplied
the model.

| Option | Primary endpoint | Use it for |
|---|---|---|
| `OpenAI` | `/v1/chat/completions` | General OpenAI Chat Completions-compatible services |
| `OpenAI-Response` | `/v1/responses` | Responses, multi-turn reasoning, and advanced agent capabilities |
| `Gemini` | Native Gemini protocol | Direct Google Gemini connections |
| `Anthropic` | `/v1/messages` | Direct Anthropic Messages API connections |
| `Ollama` | Native Ollama protocol | Local or self-hosted Ollama |
| `DeepSeek` | Native DeepSeek protocol | DeepSeek reasoning models that require correct `reasoning_content` replay |

If a provider says only “OpenAI compatible” and does not mention Responses,
start with `OpenAI`. Choose `OpenAI-Response` only when the provider documents
`/v1/responses` support or a real request succeeds.

The Base URL normally ends at the API version directory, for example:

```text
https://provider.example/v1/
```

Do not append `/responses` or `/chat/completions` yourself; InfiniShell adds the
endpoint for the selected protocol. API keys are stored in the operating
system keychain and sent to the provider endpoint you configure. Never expose
a complete key in an issue, log, or screenshot.

## Responses state

Responses state decides whether InfiniShell or the provider owns multi-turn
history.

| Mode | What InfiniShell sends | Advantage | Caveat |
|---|---|---|---|
| `Local / ZDR` | `store:false`; the client replays required message, reasoning, and tool items | Clear privacy boundary and the lowest gateway requirements | Requests grow as the conversation grows |
| `Provider chain` | `store:true`; later turns use `previous_response_id` | Sends only new input on each turn | The provider stores state and must implement response chaining correctly |
| `Cloud conversation` | Creates a Conversations object and continues with its conversation ID | Explicit, durable server-side conversation | Requires `/v1/conversations` and has the broadest retention boundary |

Use `Local / ZDR` for privacy-sensitive use, third-party gateways, or uncertain
server capabilities. Use `Provider chain` with OpenAI or a provider whose full
response-chain support you have verified. Use `Cloud conversation` only when
you need durable server-side conversations and have verified the Conversations
API.

`Local / ZDR` means InfiniShell sets `store:false` and manages conversation
state locally. It does not replace a provider's privacy policy and cannot
guarantee that a third-party gateway keeps no access logs. See OpenAI's
[Conversation state](https://developers.openai.com/api/docs/guides/conversation-state)
guide for the underlying state patterns.

## Transport

### HTTP + SSE

Each turn creates a Response over HTTP and receives streaming events over SSE.
It is the default, works through the widest range of reverse proxies, CDNs, and
enterprise gateways, and is required for Background mode in InfiniShell.

### WebSocket

A persistent WebSocket connection carries multiple `response.create` turns.
It is useful for long agent workflows with many model/tool round trips. It
reduces continuation and transport overhead; it does not automatically make
the model reason faster.

Verify that both the provider and every intermediate gateway support Responses
WebSocket before enabling it. InfiniShell does not send `background` in
WebSocket mode; switch back to `HTTP + SSE` for background tasks. See OpenAI's
[WebSocket Mode](https://developers.openai.com/api/docs/guides/websocket-mode)
guide for the protocol.

## Automatic compaction

This setting adds a Responses `context_management.compact_threshold` of
`32k`, `64k`, or `128k`. When rendered input crosses the threshold, the server
compresses earlier state into a compaction item that later turns can continue
from.

| Option | Behavior |
|---|---|
| `Off` | Do not request automatic compaction |
| `32k` | Compact early and control request size aggressively |
| `64k` | Balance context fidelity and request size |
| `128k` | Compact later and retain more original context |

This threshold is not the model's Context or Output limit. The model row's
Context/Output values describe model capacity; this setting decides when to
compress history.

Leave compaction `Off` when a third-party gateway does not document it. If a
long conversation reaches context limits, test `128k` first, then consider
`64k` based on the model and workload. See OpenAI's
[Compaction](https://developers.openai.com/api/docs/guides/compaction) guide.

## GPT-5.6 reasoning

### Pro mode

Enabling Pro mode sends `reasoning.mode: "pro"`. GPT-5.6 performs more model
work for difficult tasks, generally increasing latency and token use. It is a
fit for complex optimization, high-value code review, or deep analysis; leave
it off for routine or latency-sensitive work.

Pro mode and reasoning effort are independent settings. Enabling Pro mode does
not automatically select the highest effort.

### All-turn reasoning context

Enabling this setting explicitly sends `reasoning.context: "all_turns"`, so a
multi-turn task can reuse available reasoning items while its goals,
assumptions, and priorities remain stable.

Turning the button off means InfiniShell does not override
`reasoning.context`; it does not disable reasoning. OpenAI's current GPT-5.6
default is `all_turns`, while third-party endpoints may choose another default.
In `Local / ZDR` mode, InfiniShell replays encrypted reasoning items returned
by the server.

See OpenAI's
[Model guidance](https://developers.openai.com/api/docs/guides/latest-model)
for GPT-5.6 reasoning modes and defaults.

## Agent capabilities

### Background + resume

Background mode is for responses that may run for several minutes. The server
executes the response asynchronously, and the client can query status and
resume its event stream.

InfiniShell enables it only for this combination:

```text
Provider chain or Cloud conversation
+
HTTP + SSE
```

The button is unavailable with `Local / ZDR` or WebSocket. Background
execution requires temporary server-side state and therefore changes the data
retention boundary. See OpenAI's
[Background mode](https://developers.openai.com/api/docs/guides/background)
guide.

### Programmatic tool calling

Programmatic Tool Calling (PTC) lets the model generate controlled JavaScript
that calls eligible tools in a hosted runtime and reduces multiple
intermediate results. It fits predictable filtering, joining, ranking,
deduplication, aggregation, or validation stages. It is not required for
ordinary function calling.

The server must support `programmatic_tool_calling`, `allowed_callers`,
`program`, and `program_output`. Leave it off unless support is explicit. See
OpenAI's
[Programmatic Tool Calling](https://developers.openai.com/api/docs/guides/tools-programmatic-tool-calling)
guide.

## Common profiles

### Privacy and compatibility first

```text
OpenAI-Response
Local / ZDR
HTTP + SSE
All advanced options off
```

Use this for third-party compatible gateways and as the starting point for any
new provider.

### Tool-heavy, long agent chains

```text
OpenAI-Response
Provider chain
WebSocket
Enable all-turn reasoning or PTC only when needed
```

Use this only after verifying WebSocket and response-chain support. Background
mode is not available in this profile.

### Very long background tasks

```text
OpenAI-Response
Provider chain or Cloud conversation
HTTP + SSE
Background + resume
Optionally Pro mode and automatic compaction
```

Use this when a difficult task must keep running after the client disconnects.

## Common errors

| Error | Likely cause | What to do |
|---|---|---|
| `404` or missing `/responses` | The provider implements only Chat Completions | Change API Type to `OpenAI` |
| `Unsupported tool type: programmatic_tool_calling` | The provider does not support PTC | Disable Programmatic tool calling; upgrade InfiniShell for compatibility fallback |
| `Invalid schema for function ... required ... Missing ...` | The provider performs strict tool-schema validation | Upgrade to an InfiniShell build containing the Responses strict-schema fix; report the provider's schema requirement if it persists |
| `/conversations` returns `404` or `400` | The provider does not implement the Conversations API | Use `Local / ZDR` or `Provider chain` |
| WebSocket handshake failure or frequent disconnects | The provider, proxy, or network does not support the persistent connection | Switch to `HTTP + SSE` |
| Background is unavailable | The current state is local or the transport is WebSocket | Select cloud state and `HTTP + SSE` |
| `context_management` or `compact_threshold` is unsupported | The provider does not implement server-side compaction | Set Automatic compaction to `Off` |

When troubleshooting, restore the recommended profile at the top of this
guide. Verify basic chat and ordinary tool calling, then enable one advanced
option at a time. Remove API keys, Authorization headers, user input, and other
sensitive content before sharing logs.

# Kubernetes cluster — `k8s_get` / `k8s_logs` / `k8s_delete`

|  |  |
| --- | --- |
| **Module** | [`src/skills/kubernetes.rs`](../../src/skills/kubernetes.rs) |
| **Tools** | `k8s_contexts`, `k8s_get`, `k8s_describe`, `k8s_logs`, `k8s_apply`, `k8s_scale`, `k8s_delete` |
| **Network** | Kubernetes API server (kubeconfig / in-cluster credentials) |
| **Default** | on — gated by `[kubernetes]` |
| **Config** | `[kubernetes]` in [`config/09-kubernetes.toml`](../../config/09-kubernetes.toml) |

## What it does
A **cluster-control** capability: lodestone talks to the Kubernetes API server
directly via [kube-rs](https://kube.rs), reading your kubeconfig (default
location, `$KUBECONFIG`, or a configured path/context) or in-cluster
service-account credentials — it never invokes the `kubectl` binary. The `kind`
argument accepts kubectl-style names (`pods`, `deploy`, `svc`, `nodes`,
`configmap`, …), resolved to the API resource and scope via API discovery. Each
action is its own tool for per-action permission granularity.

## Tools
| Tool | Arguments | Access | Purpose |
| --- | --- | --- | --- |
| `k8s_contexts` | — | read | List kubeconfig contexts + the current one (no cluster contact). |
| `k8s_get` | `kind`, `name?`, `namespace?` | read | Get one named object (full JSON), or list a kind (one line each). |
| `k8s_describe` | `kind`, `name`, `namespace?` | read | Full JSON of one named object. |
| `k8s_logs` | `pod`, `namespace?`, `container?`, `tail?` | read | A pod's logs (last `tail` lines, default 200, capped 2000; optional container). |
| `k8s_apply` | `manifest` | write | Server-side apply a kubefile — YAML, multi-document (`---`-separated) allowed; creates or updates objects. |
| `k8s_scale` | `kind`, `name`, `replicas`, `namespace?` | write | Scale a deployment/statefulset/replicaset to `replicas`. |
| `k8s_delete` | `kind`, `name`, `namespace?`, `confirm?`, `trust?` | destructive | Delete a resource by kind + name (confirm first). |

Namespace resolution for namespaced kinds: a per-call `namespace` override, else
the configured `[kubernetes].namespace`, else `"default"`.

## Configuration & gating
The `[kubernetes]` section in
[`config/09-kubernetes.toml`](../../config/09-kubernetes.toml):

- `enabled` (default `true`, env `LODESTONE_KUBERNETES_ENABLED`) — exposes the
  whole family. When off, all `k8s_*` tools disappear (gating lives in
  `main.rs::effective_disabled`).
- `allow_destructive` (default `false`, env
  `LODESTONE_KUBERNETES_ALLOW_DESTRUCTIVE`) — **pre-authorizes** `k8s_delete`,
  skipping the confirmation prompt below.
- `kubeconfig` (env `LODESTONE_KUBECONFIG`) — path to a kubeconfig; empty = the
  default location / `$KUBECONFIG` / in-cluster.
- `context` (env `LODESTONE_KUBE_CONTEXT`) — kubeconfig context; empty = the
  file's current-context.
- `namespace` (env `LODESTONE_KUBE_NAMESPACE`) — default namespace for calls;
  empty = `"default"`.

**Confirmation guard.** `k8s_delete` is always exposed but routes through the
confirmation [`guard`](../../src/skills/guard.rs) (golden rule 8). The **first**
call performs nothing — it returns a one-time `confirm` token describing exactly
what will be deleted. Call again with `confirm=<token>` to actually delete, or
`confirm=<token>` plus `trust=true` to also stop being asked for `k8s_delete` for
the rest of the session. Tokens are single-use and expire after 5 minutes. This
works on any MCP client (no elicitation support required). Setting
`allow_destructive` pre-authorizes the action and skips the prompt entirely.

> Helm release *mutation* (install/upgrade/uninstall) is out of scope for the
> direct-API approach; use `docs_helm` and `artifacthub_search` for Helm
> discovery instead.

## Example uses
- **See which clusters are wired up** — `k8s_contexts` before doing anything.
- **Triage a crash-looping pod** — `k8s_get pods` (list a namespace) → `k8s_logs`
  (the suspect pod) → `k8s_describe pod` for events and status.
- **Scale a deployment after deploying** — `k8s_apply` (a kubefile) →
  `k8s_get deploy` to confirm → `k8s_scale` to the desired replica count.
- **Clean up** — `k8s_delete` returns a token; call again with `confirm` (or set
  `allow_destructive` to skip the prompt).

## See also
[containers.md](../containers.md), [golden-rules.md](../golden-rules.md), [tools.md](../tools.md)

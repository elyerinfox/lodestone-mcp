# Package managers — `package_*`

|  |  |
| --- | --- |
| **Module** | [`src/skills/packages.rs`](../../src/skills/packages.rs) |
| **Tools** | `package_managers`, `package_search`, `package_info`, `package_list`, `package_updates`, `package_install`, `package_upgrade`, `package_remove` |
| **Default** | **off** — gated by `[packages].enabled` |
| **Config** | `[packages]` in [`config/21-packages.toml`](../../config/21-packages.toml) |
| **Destructive** | `package_install`, `package_upgrade`, `package_remove` — all guard-gated |

## Supported managers

| `kind` | Binary | Scope |
| --- | --- | --- |
| `winget` | `winget` | Windows Package Manager |
| `chocolatey` (aliases: `choco`) | `choco` | Chocolatey (Windows) |
| `brew` (aliases: `homebrew`) | `brew` | Homebrew (macOS / Linux) |
| `apt` (aliases: `apt-get`) | `apt`, `apt-cache`, `apt-get` | Debian / Ubuntu APT |
| `dnf` | `dnf` | Fedora / RHEL DNF |
| `yum` | `yum` | RHEL / CentOS YUM |
| `apk` | `apk` | Alpine APK |
| `pacman` | `pacman` | Arch Pacman (official repos) |
| `yay` (aliases: `aur`) | `yay` | AUR via yay (community PKGBUILDs) |
| `zypper` | `zypper` | openSUSE Zypper |
| `pkg` | `pkg` | FreeBSD pkg |

Each call carries a `kind` argument matching one of the names above. Adding a
manager is one enum variant + its command lookups in
[`src/skills/packages.rs`](../../src/skills/packages.rs) — no new tools.

## Tools

| Tool | Arguments | Access | Purpose |
| --- | --- | --- | --- |
| `package_managers` | — | read | List every supported PM with a ✓ / · marker for whether its binary is on `$PATH`. |
| `package_search` | `kind`, `query` | read | PM-native search (substring on most). Raw output, truncated to `[retrieval].max_chars`. |
| `package_info` | `kind`, `name` | read | PM-native package metadata (version, deps, description). |
| `package_list` | `kind` | read | Installed packages via the named PM. |
| `package_updates` | `kind` | read | Available updates without applying them. |
| `package_install` | `kind`, `name`, `confirm?`, `trust?` | **destructive** | Install a named package after the guard challenge. |
| `package_upgrade` | `kind`, `name?`, `confirm?`, `trust?` | **destructive** | Upgrade one package (or all when `name` is omitted). |
| `package_remove` | `kind`, `name`, `confirm?`, `trust?` | **destructive** | Remove a named package. |

## Capability gating

The family is `Ready` whenever **any** supported PM binary is on `$PATH`.
Per-call, the wrapper additionally re-checks the specific `kind` the model
named — `package_install { kind: "winget", … }` on a Linux host returns
"Windows Package Manager (winget) isn't installed or not on $PATH on this host"
rather than spawning and failing opaquely. The dashboard's host-capabilities
panel shows the family's badge; individual missing managers surface via the
errors above.

## Destructive workflow (guard, golden rule 8)

`package_install` / `package_upgrade` / `package_remove` follow the project's
[guard](../golden-rules.md) protocol exactly like docker / k8s / ffmpeg / shell:

1. **First call** returns a confirmation prompt (no action taken). Example:
   ```
   Confirm: install 'htop' via apt
   Reply with confirm="<token>" to proceed, or confirm + trust=true to skip future
   prompts for package_install in this session.
   ```
2. **Second call with `confirm`** runs the operation.
3. `trust=true` whitelists *this exact action tag* for the session
   (`package_install`, `package_upgrade`, `package_remove` are separate
   tags — confirming an install doesn't auto-authorize a remove).
4. `[packages].allow_destructive = true` pre-authorizes all three for the
   session — flip "challenge" to "proceed" without removing the guard from
   the call path.

## No `sudo`

The skill **never** invokes `sudo` or any other privilege wrapper. Privilege
is the operator's choice:

- Container: pick a UID with the privileges you want lodestone to have.
- Host install: run lodestone as a user with passwordless sudo, or
  `setcap`-style capabilities, or a `doas` wrapper.
- Per-PM split: `brew` and `apk` typically don't need root; `apt` / `dnf`
  / `yum` / `pacman` / `pkg` / `zypper` typically do. The error you'll see
  from the PM itself ("Permission denied" / "are you root?") tells you
  exactly what's missing.

## Non-interactive flags

Each destructive command pre-sets the manager's "don't ask y/N" flag so a
backgrounded call can't hang on a prompt:

- winget: `--silent --accept-source-agreements --accept-package-agreements`
- chocolatey: `-y --no-progress`
- apt / dnf / yum / pkg: `-y`
- pacman / yay: `--noconfirm`
- zypper: `--non-interactive`
- apk / brew: non-interactive by default

The unit tests in `src/skills/packages.rs` enforce this — adding a PM that
lacks a non-interactive install flag fails CI.

## Example
```text
# 1. See what's available on this host.
package_managers
# -> 3 detected on this host: ✓ brew, ✓ apt, ✓ pkg, · winget, · choco, …

# 2. Find a package.
package_search { kind: "apt", query: "htop" }

# 3. Read up on it.
package_info { kind: "apt", name: "htop" }

# 4. Install (first call — confirmation prompt).
package_install { kind: "apt", name: "htop" }
# -> Confirm: install 'htop' via apt. Reply with confirm=<token> ...

# 5. Install (second call — actually runs).
package_install { kind: "apt", name: "htop", confirm: "<token>" }
```

## See also
[golden-rules.md](../golden-rules.md), [tools.md](../tools.md),
[guard.md](guard.md)

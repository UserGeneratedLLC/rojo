# /release - Version Bump and Tag Release

Bump the project version, update the changelog, commit, push, and tag a new release.

The user must provide the new version string (e.g. `8.0.1`). If not provided, ask for it before proceeding.

---

## Step 1: Pre-Flight — Submodule Check

Run these two commands and inspect the output:

```powershell
git submodule status
```

```powershell
git submodule foreach "git status --short"
```

**Verify ALL submodules are clean:**
- No submodule prefixed with `+` (dirty) or `-` (uninitialized) in `git submodule status`
- No uncommitted changes in any submodule from `git submodule foreach`

**If any submodule is dirty or uninitialized, STOP immediately.** Report which submodules have issues and do not proceed until the user resolves them.

**Ignore** `rbx-dom/aftman.toml` — it references the Atlas version but is not part of the release process.

---

## Step 2: Version Bump

Replace the old version with the new version in each of these files:

| File | What to change |
|---|---|
| `Cargo.toml` | `version = "X.Y.Z"` (line 3) |
| `plugin/Version.txt` | Entire file content — must match `Cargo.toml` exactly (enforced by `build.rs`) |

**Do NOT manually edit `Cargo.lock`.** It is auto-updated in the next step.

**Note:** `rokit.toml` does NOT contain atlas — the tool builds from source and should never self-reference a version that may not be released yet.

After editing the three files above, run:

```powershell
cargo check
```

This does two things:
1. Auto-updates `Cargo.lock` with the new version
2. Validates that `plugin/Version.txt` matches `Cargo.toml` (build.rs assertion — build will fail if mismatched)

**If `cargo check` fails, fix the issue before continuing.**

---

## Step 3: Update CHANGELOG.md

### 3a. Gather commit history

Run this to get all commits since the previous version tag (replace `vOLD` with the previous tag):

```powershell
git log --oneline vOLD..HEAD
```

### 3b. Write the release section

Insert a new version section between `## Unreleased` and the previous version's header. Format:

```markdown
## Unreleased

## [X.Y.Z] (Month Dth, Year)

* Summary of change 1
* Summary of change 2
* ...

[X.Y.Z]: https://github.com/UserGeneratedLLC/rojo/releases/tag/vX.Y.Z
```

**Guidelines for writing entries:**
- Group related commits into single bullet points (don't list every commit separately)
- Write user-facing descriptions, not raw commit messages
- Use present tense ("Add", "Fix", "Improve", not "Added", "Fixed", "Improved")
- Omit internal-only changes (CI tweaks, test refactors, snapshot updates) unless they affect behavior
- Keep link definitions in ascending numeric order (per repo convention)

### 3c. Include a git summary

After the bullet points, add a collapsible section with the raw commit log so readers can see full detail:

```markdown
<details>
<summary>Full commit log</summary>

- `abc1234` Commit message one
- `def5678` Commit message two
- ...

</details>
```

---

## Step 4: Commit, Push, Tag, Push Tag

Run these sequentially:

```powershell
git add Cargo.toml Cargo.lock plugin/Version.txt CHANGELOG.md
```

```powershell
git commit -m "$(cat <<'EOF'
X.Y.Z

- Key change 1
- Key change 2
- ...
EOF
)"
```

**Commit message guidelines:**
- First line: just the version number
- Body: concise bullet list of the most important changes since the last tag (derived from the changelog entries written in step 3)
- Distill to the highlights — not a copy of the full changelog

```powershell
git push
```

```powershell
git tag vX.Y.Z
```

```powershell
git push origin vX.Y.Z
```

The tag push triggers `.github/workflows/release.yml` which creates a draft GitHub release and builds artifacts for all platforms.

---

## All Versioned Files (Reference)

| File | Role | Updated by |
|---|---|---|
| `Cargo.toml` | Crate version (source of truth) | Manual edit |
| `Cargo.lock` | Lockfile version | `cargo check` (automatic) |
| `plugin/Version.txt` | Plugin version, validated at build time | Manual edit |
| `CHANGELOG.md` | Release notes | Manual edit |
